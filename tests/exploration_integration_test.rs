//! End-to-end integration test: graph-enhanced pipeline.
//!
//! Verifies the new query engine methods work together: resolve_seed,
//! build_map, find_weighted_path, get_cluster_detail.

use canopy::query::QueryEngine;
use canopy::store::Store;
use std::collections::HashMap;
use tempfile::TempDir;

fn build_fixture() -> (Store, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("test.redb");
    let store = Store::open(&path).expect("open store");

    // 15-entity fixture: auth subsystem with realistic topology
    let fns = [
        ("HANDLE_LOGIN::auth.rs", "handle_login", "auth.rs"),
        ("HANDLE_LOGOUT::auth.rs", "handle_logout", "auth.rs"),
        ("VERIFY_TOKEN::auth.rs", "verify_token", "auth.rs"),
        ("HASH_PASSWORD::auth.rs", "hash_password", "auth.rs"),
        ("GEN_TOKEN::auth.rs", "generate_token", "auth.rs"),
        ("AUTH_SERVICE::auth.rs", "AuthService", "auth.rs"),
        ("ROUTER_DISPATCH::router.rs", "router_dispatch", "router.rs"),
        ("MIDDLEWARE::middleware.rs", "auth_middleware", "middleware.rs"),
        ("DB_LOOKUP::db.rs", "user_lookup", "db.rs"),
        ("DB_WRITE::db.rs", "user_write", "db.rs"),
        ("LOGGER::log.rs", "log_event", "log.rs"),
        ("ERR_HANDLE::err.rs", "handle_error", "err.rs"),
        ("CONFIG_LOAD::config.rs", "load_config", "config.rs"),
        ("USER_MODEL::model.rs", "User", "model.rs"),
        ("TOKEN_MODEL::model.rs", "Token", "model.rs"),
    ];
    for (key, name, file) in &fns {
        store
            .insert_entity(
                key,
                "FUNCTION",
                &format!("pub fn {name}()"),
                "s",
                file,
                serde_json::json!({
                    "name": name,
                    "file_path": file,
                    "line": 10,
                    "kind": "function",
                }),
                None,
                Some("pub".into()),
            )
            .unwrap();
    }

    // Call topology
    let calls = [
        ("ROUTER_DISPATCH::router.rs", "HANDLE_LOGIN::auth.rs"),
        ("ROUTER_DISPATCH::router.rs", "HANDLE_LOGOUT::auth.rs"),
        ("MIDDLEWARE::middleware.rs", "VERIFY_TOKEN::auth.rs"),
        ("HANDLE_LOGIN::auth.rs", "HASH_PASSWORD::auth.rs"),
        ("HANDLE_LOGIN::auth.rs", "GEN_TOKEN::auth.rs"),
        ("HANDLE_LOGIN::auth.rs", "DB_LOOKUP::db.rs"),
        ("HANDLE_LOGIN::auth.rs", "LOGGER::log.rs"),
        ("VERIFY_TOKEN::auth.rs", "DB_LOOKUP::db.rs"),
        ("GEN_TOKEN::auth.rs", "LOGGER::log.rs"),
    ];
    for (src, tgt) in &calls {
        store
            .insert_relationship(src, tgt, "CALLS", "", 1.0, "", "s", false)
            .unwrap();
    }

    // Assign auth-related symbols to cluster 1
    let mut cluster_map: HashMap<String, u32> = HashMap::new();
    for key in &[
        "HANDLE_LOGIN::auth.rs",
        "HANDLE_LOGOUT::auth.rs",
        "VERIFY_TOKEN::auth.rs",
        "HASH_PASSWORD::auth.rs",
        "GEN_TOKEN::auth.rs",
        "AUTH_SERVICE::auth.rs",
    ] {
        cluster_map.insert((*key).to_string(), 1);
    }
    store.store_clusters(&cluster_map).unwrap();

    let mut labels = HashMap::new();
    labels.insert(1u32, "authentication".to_string());
    store.store_cluster_meta(&labels).unwrap();

    (store, dir)
}

#[test]
fn test_build_map_on_handle_login() {
    let (store, _dir) = build_fixture();
    let engine = QueryEngine::new(&store);

    let detail = engine.build_map("HANDLE_LOGIN::auth.rs", 10);
    assert!(detail.is_some(), "build_map should find handle_login");

    let d = detail.unwrap();
    assert_eq!(d.name, "handle_login");
    assert_eq!(d.file_path, "auth.rs");

    // Should have callees: hash_password, gen_token, db_lookup, logger
    assert!(
        d.calls.len() >= 3,
        "expected at least 3 callees, got {}",
        d.calls.len()
    );

    // Should have callers: router_dispatch
    assert!(
        d.called_by.iter().any(|c| c.name == "router_dispatch"),
        "expected router_dispatch as caller"
    );

    // Should have cluster label
    assert_eq!(d.cluster_label.as_deref(), Some("authentication"));
}

#[test]
fn test_find_weighted_path_router_to_db() {
    let (store, _dir) = build_fixture();
    let engine = QueryEngine::new(&store);

    // Path: router_dispatch -> handle_login -> db_lookup
    let hops = engine.find_weighted_path("ROUTER_DISPATCH::router.rs", "DB_LOOKUP::db.rs", 5);

    assert_eq!(hops.len(), 2, "expected 2-hop path");
    assert_eq!(hops[0].from_key, "ROUTER_DISPATCH::router.rs");
    assert_eq!(hops[0].to_key, "HANDLE_LOGIN::auth.rs");
    assert_eq!(hops[1].from_key, "HANDLE_LOGIN::auth.rs");
    assert_eq!(hops[1].to_key, "DB_LOOKUP::db.rs");
    // All CALLS edges, cost 1.0 each
    assert!(hops.iter().all(|h| h.edge_type == "CALLS"));
    assert!(hops.iter().all(|h| (h.edge_cost - 1.0).abs() < 0.01));
}

#[test]
fn test_get_cluster_detail_auth_cluster() {
    let (store, _dir) = build_fixture();
    let engine = QueryEngine::new(&store);

    let result = engine.get_cluster_detail(1, "authentication", 10, 10).unwrap();

    assert_eq!(result.label, "authentication");
    assert_eq!(result.members.len(), 6, "auth cluster has 6 members");
    assert!(!result.relationships.is_empty(), "auth cluster should have intra-cluster relationships");
}

#[test]
fn test_resolve_seed_then_build_map() {
    let (store, _dir) = build_fixture();
    let engine = QueryEngine::new(&store);

    // Resolve "handle_login" to entity key
    let matches = engine.resolve_seed("HANDLE_LOGIN");
    assert!(!matches.is_empty());

    let key = &matches[0].entity_name;
    let detail = engine.build_map(key, 10);
    assert!(detail.is_some(), "build_map should work with resolved key");
}

#[test]
fn test_did_you_mean_on_typo() {
    let (store, _dir) = build_fixture();
    let engine = QueryEngine::new(&store);

    let suggestions = engine.suggest_similar_entities("HandleLgn", 5);
    assert!(!suggestions.is_empty(), "expected suggestions for typo");
}
