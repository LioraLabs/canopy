use canopy::cli::cluster::{ClusterResult, ClusterEdge, format_cluster_result};
use canopy::cluster::louvain;
use canopy::resolve::Candidate;
use std::collections::HashMap;

#[test]
fn test_format_cluster_result() {
    let result = ClusterResult {
        label: "graph-traversal".to_string(),
        members: vec![
            Candidate { name: "expand_graph".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 388, ambiguous: false },
            Candidate { name: "walk_edges".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 250, ambiguous: false },
            Candidate { name: "GraphContext".to_string(), kind: "type".to_string(), file_path: "src/query.rs".to_string(), line: 22, ambiguous: false },
        ],
        relationships: vec![
            ClusterEdge {
                source: "expand_graph".to_string(),
                target: "walk_edges".to_string(),
                edge_type: "CALLS".to_string(),
            },
        ],
        truncated_members: None,
        truncated_relationships: None,
    };

    let output = format_cluster_result(&result);
    assert!(output.contains("cluster: graph-traversal (3 members)"));
    assert!(output.contains("expand_graph [function] src/query.rs:388"));
    assert!(output.contains("key relationships:"));
    assert!(output.contains("expand_graph --CALLS--> walk_edges"));
}

#[test]
fn test_format_cluster_result_truncated() {
    let result = ClusterResult {
        label: "big-cluster".to_string(),
        members: vec![
            Candidate { name: "a".to_string(), kind: "function".to_string(), file_path: "src/a.rs".to_string(), line: 1, ambiguous: false },
        ],
        relationships: vec![],
        truncated_members: Some(29),
        truncated_relationships: Some(50),
    };

    let output = format_cluster_result(&result);
    assert!(output.contains("... and 29 more members"));
    assert!(output.contains("... and 50 more relationships"));
}

#[test]
fn test_louvain_two_cliques() {
    let entities: Vec<String> = (0..6).map(|i| format!("node_{}", i)).collect();
    let mut affinities: HashMap<(String, String), f64> = HashMap::new();

    // Clique 1: nodes 0,1,2
    affinities.insert(("node_0".into(), "node_1".into()), 1.0);
    affinities.insert(("node_0".into(), "node_2".into()), 1.0);
    affinities.insert(("node_1".into(), "node_2".into()), 1.0);

    // Clique 2: nodes 3,4,5
    affinities.insert(("node_3".into(), "node_4".into()), 1.0);
    affinities.insert(("node_3".into(), "node_5".into()), 1.0);
    affinities.insert(("node_4".into(), "node_5".into()), 1.0);

    // Weak bridge
    affinities.insert(("node_2".into(), "node_3".into()), 0.1);

    let labels = louvain(&entities, &affinities, 1.0);

    assert_eq!(labels[&"node_0".to_string()], labels[&"node_1".to_string()]);
    assert_eq!(labels[&"node_0".to_string()], labels[&"node_2".to_string()]);
    assert_eq!(labels[&"node_3".to_string()], labels[&"node_4".to_string()]);
    assert_eq!(labels[&"node_3".to_string()], labels[&"node_5".to_string()]);
    assert_ne!(labels[&"node_0".to_string()], labels[&"node_3".to_string()]);
}

#[test]
fn test_louvain_single_node() {
    let entities = vec!["alone".to_string()];
    let affinities: HashMap<(String, String), f64> = HashMap::new();
    let labels = louvain(&entities, &affinities, 1.0);
    assert_eq!(labels.len(), 1);
    assert!(labels.contains_key("alone"));
}

#[test]
fn test_louvain_all_disconnected() {
    let entities: Vec<String> = (0..5).map(|i| format!("node_{}", i)).collect();
    let affinities: HashMap<(String, String), f64> = HashMap::new();
    let labels = louvain(&entities, &affinities, 1.0);
    let unique_labels: std::collections::HashSet<u32> = labels.values().copied().collect();
    assert_eq!(unique_labels.len(), 5);
}

#[test]
fn test_cluster_label_prefers_non_test_entity() {
    let dir = tempfile::tempdir().unwrap();
    let store = canopy::store::Store::open(dir.path().join("test.db").as_path()).unwrap();

    let meta = |name: &str, kind: &str, file: &str, line: usize| {
        serde_json::json!({"name": name, "file_path": file, "line": line, "kind": kind})
    };

    store.insert_entity(
        "TEST_AUTH::TESTS/AUTH_TEST.RS", "FUNCTION", "fn test_auth()",
        "doc1", "tests/auth_test.rs", meta("test_auth", "function", "tests/auth_test.rs", 1),
        None, None,
    ).unwrap();

    store.insert_entity(
        "AUTHENTICATE::SRC/AUTH.RS", "FUNCTION", "fn authenticate()",
        "doc1", "src/auth.rs", meta("authenticate", "function", "src/auth.rs", 10),
        None, None,
    ).unwrap();

    // Give test entity MORE relationships (higher degree)
    store.insert_relationship("TEST_AUTH::TESTS/AUTH_TEST.RS", "AUTHENTICATE::SRC/AUTH.RS", "CALLS", "", 1.0, "", "canopy", false).unwrap();
    store.insert_relationship("TEST_AUTH::TESTS/AUTH_TEST.RS", "TEST_AUTH::TESTS/AUTH_TEST.RS", "CALLS", "", 1.0, "", "canopy", false).unwrap();

    let mut clusters = std::collections::HashMap::new();
    clusters.insert("TEST_AUTH::TESTS/AUTH_TEST.RS".to_string(), 1u32);
    clusters.insert("AUTHENTICATE::SRC/AUTH.RS".to_string(), 1u32);

    let labels = canopy::cluster::compute_cluster_labels(&clusters, &store).unwrap();
    assert_eq!(labels[&1u32], "AUTHENTICATE::SRC/AUTH.RS");
}

#[test]
fn test_louvain_bridge_node_doesnt_merge_cliques() {
    let entities: Vec<String> = (0..7).map(|i| format!("n{}", i)).collect();
    let mut affinities: HashMap<(String, String), f64> = HashMap::new();

    // Clique A: n0, n1, n2
    affinities.insert(("n0".into(), "n1".into()), 1.0);
    affinities.insert(("n0".into(), "n2".into()), 1.0);
    affinities.insert(("n1".into(), "n2".into()), 1.0);

    // Clique B: n4, n5, n6
    affinities.insert(("n4".into(), "n5".into()), 1.0);
    affinities.insert(("n4".into(), "n6".into()), 1.0);
    affinities.insert(("n5".into(), "n6".into()), 1.0);

    // Bridge node n3 connects to both cliques weakly
    affinities.insert(("n2".into(), "n3".into()), 0.3);
    affinities.insert(("n3".into(), "n4".into()), 0.3);

    let labels = louvain(&entities, &affinities, 1.0);

    assert_eq!(labels[&"n0".to_string()], labels[&"n1".to_string()]);
    assert_eq!(labels[&"n0".to_string()], labels[&"n2".to_string()]);
    assert_eq!(labels[&"n4".to_string()], labels[&"n5".to_string()]);
    assert_eq!(labels[&"n4".to_string()], labels[&"n6".to_string()]);
    assert_ne!(labels[&"n0".to_string()], labels[&"n4".to_string()]);
}
