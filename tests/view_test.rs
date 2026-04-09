use canopy::store::Store;
use canopy::view::build_graph_json;
use tempfile::TempDir;

fn tmp_store() -> (Store, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("test.redb");
    let store = Store::open(&path).expect("open store");
    (store, dir)
}

#[test]
fn test_build_graph_json_empty_store() {
    let (store, _dir) = tmp_store();
    let json = build_graph_json(&store).expect("build graph json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");
    assert_eq!(parsed["nodes"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["links"].as_array().unwrap().len(), 0);
}

#[test]
fn test_build_graph_json_with_entities_and_relationships() {
    let (store, _dir) = tmp_store();

    // Insert two entities
    store
        .insert_entity(
            "STORE::SRC/STORE.RS",
            "TYPE",
            "pub struct Store",
            "canopy",
            "src/store.rs",
            serde_json::json!({"file_path": "src/store.rs", "line": 107, "kind": "struct"}),
            None,
            Some("pub".to_string()),
        )
        .unwrap();
    store
        .insert_entity(
            "INSERT_CHUNK::SRC/STORE.RS",
            "FUNCTION",
            "pub fn insert_chunk",
            "canopy",
            "src/store.rs",
            serde_json::json!({"file_path": "src/store.rs", "line": 158, "kind": "function"}),
            Some("Store".to_string()),
            Some("pub".to_string()),
        )
        .unwrap();

    // Insert a relationship
    store
        .insert_relationship(
            "STORE::SRC/STORE.RS",
            "INSERT_CHUNK::SRC/STORE.RS",
            "CONTAINS",
            "contains, parent",
            1.0,
            "Store contains insert_chunk",
            "canopy",
            false,
        )
        .unwrap();

    let json = build_graph_json(&store).expect("build graph json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");

    let nodes = parsed["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);

    // Check node fields
    let store_node = nodes.iter().find(|n| n["id"] == "STORE::SRC/STORE.RS").unwrap();
    assert_eq!(store_node["name"], "STORE");
    assert_eq!(store_node["type"], "TYPE");
    assert_eq!(store_node["file"], "src/store.rs");
    assert_eq!(store_node["line"], 107);

    let links = parsed["links"].as_array().unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0]["source"], "STORE::SRC/STORE.RS");
    assert_eq!(links[0]["target"], "INSERT_CHUNK::SRC/STORE.RS");
    assert_eq!(links[0]["type"], "CONTAINS");
}

#[test]
fn test_build_graph_json_includes_cluster_data() {
    let (store, _dir) = tmp_store();

    // Insert two entities
    store
        .insert_entity(
            "STORE::SRC/STORE.RS",
            "TYPE",
            "pub struct Store",
            "canopy",
            "src/store.rs",
            serde_json::json!({"file_path": "src/store.rs", "line": 107, "kind": "struct"}),
            None,
            Some("pub".to_string()),
        )
        .unwrap();
    store
        .insert_entity(
            "CONFIG::SRC/CONFIG.RS",
            "TYPE",
            "pub struct Config",
            "canopy",
            "src/config.rs",
            serde_json::json!({"file_path": "src/config.rs", "line": 10, "kind": "struct"}),
            None,
            Some("pub".to_string()),
        )
        .unwrap();

    // Store cluster assignments
    let mut clusters = std::collections::HashMap::new();
    clusters.insert("STORE::SRC/STORE.RS".to_string(), 0u32);
    clusters.insert("CONFIG::SRC/CONFIG.RS".to_string(), 1u32);
    store.store_clusters(&clusters).unwrap();

    // Store cluster labels
    let mut labels = std::collections::HashMap::new();
    labels.insert(0u32, "STORE::SRC/STORE.RS".to_string());
    labels.insert(1u32, "CONFIG::SRC/CONFIG.RS".to_string());
    store.store_cluster_meta(&labels).unwrap();

    let json = build_graph_json(&store).expect("build graph json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");

    // clusters field should be an object mapping entity_id -> cluster_id
    let clusters_val = &parsed["clusters"];
    assert!(clusters_val.is_object(), "clusters should be an object");
    assert_eq!(clusters_val["STORE::SRC/STORE.RS"], 0);
    assert_eq!(clusters_val["CONFIG::SRC/CONFIG.RS"], 1);

    // cluster_labels should be an object mapping cluster_id (as string key) -> label
    let labels_val = &parsed["cluster_labels"];
    assert!(labels_val.is_object(), "cluster_labels should be an object");
    assert_eq!(labels_val["0"], "STORE::SRC/STORE.RS");
    assert_eq!(labels_val["1"], "CONFIG::SRC/CONFIG.RS");
}

#[test]
fn test_build_graph_json_empty_clusters() {
    let (store, _dir) = tmp_store();
    let json = build_graph_json(&store).expect("build graph json");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse json");

    // Should still have clusters and cluster_labels fields, just empty
    assert!(parsed["clusters"].is_object());
    assert!(parsed["cluster_labels"].is_object());
}
