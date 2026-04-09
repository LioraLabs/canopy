use canopy::resolve::{FuzzyResolver, Resolution};
use canopy::store::Store;

fn setup_store_with_entities(dir: &tempfile::TempDir) -> Store {
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": "src/test.rs", "line": line, "kind": kind})
    };
    store.insert_entity(
        "EXPAND_GRAPH::SRC/TEST.RS", "FUNCTION", "pub fn expand_graph()",
        "doc1", "src/test.rs", meta("expand_graph", "function", 10),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "EXPAND_GRAPH_FROM_ENTITIES::SRC/TEST.RS", "FUNCTION",
        "pub fn expand_graph_from_entities()", "doc1", "src/test.rs",
        meta("expand_graph_from_entities", "function", 50),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "EXPAND_GRAPH_FROM_ENTITIES_CAPPED::SRC/TEST.RS", "FUNCTION",
        "pub fn expand_graph_from_entities_capped()", "doc1", "src/test.rs",
        meta("expand_graph_from_entities_capped", "function", 100),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "GRAPHCONTEXT::SRC/TEST.RS", "TYPE", "pub struct GraphContext",
        "doc1", "src/test.rs", meta("GraphContext", "type", 5),
        None, Some("pub".into()),
    ).unwrap();
    store
}

#[test]
fn test_exact_match_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store_with_entities(&dir);
    let resolver = FuzzyResolver::new(&store, 10);
    match resolver.resolve("expand_graph") {
        Resolution::Exact(key) => { assert!(key.starts_with("EXPAND_GRAPH::")); }
        other => panic!("expected Exact, got {:?}", other),
    }
}

#[test]
fn test_prefix_match_single_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store_with_entities(&dir);
    let resolver = FuzzyResolver::new(&store, 10);
    match resolver.resolve("GraphCon") {
        Resolution::Exact(key) => { assert!(key.starts_with("GRAPHCONTEXT::")); }
        other => panic!("expected Exact, got {:?}", other),
    }
}

#[test]
fn test_prefix_match_multiple_returns_suggestions() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store_with_entities(&dir);
    let resolver = FuzzyResolver::new(&store, 10);
    match resolver.resolve("expand_graph_from") {
        Resolution::Suggestions { candidates, total, .. } => {
            assert_eq!(total, 2);
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected Suggestions, got {:?}", other),
    }
}

#[test]
fn test_no_match_returns_fuzzy_suggestions() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store_with_entities(&dir);
    let resolver = FuzzyResolver::new(&store, 10);
    match resolver.resolve("exapnd_grph") {
        Resolution::Suggestions { candidates, .. } => { assert!(!candidates.is_empty()); }
        Resolution::NoMatch(_) => {}
        other => panic!("expected Suggestions or NoMatch, got {:?}", other),
    }
}

#[test]
fn test_fuzzy_excludes_distant_matches() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": "src/test.rs", "line": line, "kind": kind})
    };

    // Close match — should be found
    store.insert_entity(
        "TIMELINEPLAYBACK::SRC/TEST.RS", "TYPE", "struct TimelinePlayback",
        "doc1", "src/test.rs", meta("TimelinePlayback", "type", 10),
        None, Some("pub".into()),
    ).unwrap();

    // Distant match — should NOT be found with tight threshold
    store.insert_entity(
        "FINALIZE_LAYOUT::SRC/TEST.RS", "FUNCTION", "fn finalize_layout()",
        "doc1", "src/test.rs", meta("finalize_layout", "function", 50),
        None, Some("pub".into()),
    ).unwrap();

    let resolver = FuzzyResolver::new(&store, 10);
    match resolver.resolve("TimelinePlayhead") {
        Resolution::Suggestions { candidates, .. } => {
            let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
            assert!(names.contains(&"TimelinePlayback"), "close match should be found");
            assert!(!names.contains(&"finalize_layout"), "distant match should be excluded");
        }
        Resolution::NoMatch(_) => {
            panic!("should find at least TimelinePlayback");
        }
        other => panic!("expected Suggestions, got {:?}", other),
    }
}

#[test]
fn test_suggestions_capped() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup_store_with_entities(&dir);
    let resolver = FuzzyResolver::new(&store, 2);
    match resolver.resolve("expand") {
        Resolution::Suggestions { candidates, total, .. } => {
            assert!(candidates.len() <= 2);
            assert!(total >= 3);
        }
        other => panic!("expected Suggestions, got {:?}", other),
    }
}

#[test]
fn test_case_sensitive_prefers_exact_case() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": "src/lib.rs", "line": line, "kind": kind})
    };
    store.insert_entity(
        "ANALYSIS::SRC/LIB.RS", "TYPE", "pub struct Analysis",
        "doc1", "src/lib.rs", meta("Analysis", "struct", 10),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "ANALYSIS_MOD::SRC/LIB.RS", "MODULE", "mod analysis",
        "doc1", "src/lib.rs", meta("analysis", "module", 50),
        None, Some("pub".into()),
    ).unwrap();

    let resolver = FuzzyResolver::new(&store, 10);

    // "Analysis" (capital A) should resolve to the struct
    match resolver.resolve("Analysis") {
        Resolution::Exact(key) => {
            assert!(key.contains("ANALYSIS::"), "expected struct key, got {}", key);
        }
        other => panic!("expected Exact for 'Analysis', got {:?}", other),
    }

    // "analysis" (lowercase) should resolve to the module
    match resolver.resolve("analysis") {
        Resolution::Exact(key) => {
            assert!(key.contains("ANALYSIS_MOD::"), "expected module key, got {}", key);
        }
        other => panic!("expected Exact for 'analysis', got {:?}", other),
    }
}

#[test]
fn test_file_path_qualifier_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, file: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": file, "line": line, "kind": kind})
    };
    store.insert_entity(
        "HANDLER::SRC/AUTH.RS", "FUNCTION", "pub fn handler()",
        "doc1", "src/auth.rs", meta("handler", "function", "src/auth.rs", 10),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "HANDLER::SRC/API.RS", "FUNCTION", "pub fn handler()",
        "doc1", "src/api.rs", meta("handler", "function", "src/api.rs", 20),
        None, Some("pub".into()),
    ).unwrap();

    let resolver = FuzzyResolver::new(&store, 10);

    // "handler" alone should be ambiguous
    match resolver.resolve("handler") {
        Resolution::Suggestions { total, .. } => {
            assert_eq!(total, 2, "handler alone should match 2");
        }
        other => panic!("expected Suggestions for 'handler', got {:?}", other),
    }

    // "src/auth.rs:handler" should resolve to the auth handler
    match resolver.resolve("src/auth.rs:handler") {
        Resolution::Exact(key) => {
            assert!(key.contains("SRC/AUTH.RS"), "expected auth key, got {}", key);
        }
        other => panic!("expected Exact for 'src/auth.rs:handler', got {:?}", other),
    }
}

#[test]
fn test_kind_qualifier_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": "src/lib.rs", "line": line, "kind": kind})
    };
    store.insert_entity(
        "RESOLVER_STRUCT::SRC/LIB.RS", "TYPE", "pub struct Resolver",
        "doc1", "src/lib.rs", meta("Resolver", "struct", 10),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "RESOLVER_FN::SRC/LIB.RS", "FUNCTION", "pub fn resolver()",
        "doc1", "src/lib.rs", meta("resolver", "function", 50),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "RESOLVER_MOD::SRC/LIB.RS", "MODULE", "mod Resolver",
        "doc1", "src/lib.rs", meta("Resolver", "module", 100),
        None, Some("pub".into()),
    ).unwrap();

    let resolver = FuzzyResolver::new(&store, 10);

    // "struct:Resolver" should resolve to the struct
    match resolver.resolve("struct:Resolver") {
        Resolution::Exact(key) => {
            assert!(key.contains("RESOLVER_STRUCT::"), "expected struct key, got {}", key);
        }
        other => panic!("expected Exact for 'struct:Resolver', got {:?}", other),
    }

    // "function:resolver" should resolve to the function
    match resolver.resolve("function:resolver") {
        Resolution::Exact(key) => {
            assert!(key.contains("RESOLVER_FN::"), "expected fn key, got {}", key);
        }
        other => panic!("expected Exact for 'function:resolver', got {:?}", other),
    }
}

#[test]
fn test_bracket_format_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();
    let meta = |name: &str, kind: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": "src/lib.rs", "line": line, "kind": kind})
    };
    store.insert_entity(
        "ANALYSIS_STRUCT::SRC/LIB.RS", "TYPE", "pub struct Analysis",
        "doc1", "src/lib.rs", meta("Analysis", "struct", 10),
        None, Some("pub".into()),
    ).unwrap();
    store.insert_entity(
        "ANALYSIS_MOD::SRC/LIB.RS", "MODULE", "mod Analysis",
        "doc1", "src/lib.rs", meta("Analysis", "module", 50),
        None, Some("pub".into()),
    ).unwrap();

    let resolver = FuzzyResolver::new(&store, 10);

    // "Analysis [struct]" should resolve to the struct
    match resolver.resolve("Analysis [struct]") {
        Resolution::Exact(key) => {
            assert!(key.contains("ANALYSIS_STRUCT::"), "expected struct key, got {}", key);
        }
        other => panic!("expected Exact for 'Analysis [struct]', got {:?}", other),
    }
}
