use canopy::cli::map::{MapResult, EntityDetail, format_map_result};
use canopy::resolve::{Resolution, Candidate};

#[test]
fn test_format_map_result() {
    let result = MapResult {
        entity: EntityDetail {
            name: "expand_graph".to_string(),
            kind: "function".to_string(),
            file_path: "src/query.rs".to_string(),
            line: 388,
            signature: "pub fn expand_graph(&self, seeds: &[&str], hops: usize) -> GraphContext".to_string(),
            cluster_label: Some("graph-traversal".to_string()),
            calls: vec![
                Candidate {
                    name: "get_relationships_for_entity".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/store.rs".to_string(),
                    line: 408,
                    ambiguous: false,
                },
            ],
            called_by: vec![
                Candidate {
                    name: "cmd_vector_search".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/main.rs".to_string(),
                    line: 566,
                    ambiguous: false,
                },
            ],
            references: vec![],
            referenced_by: vec![],
            accepts: vec![],
            accepted_by: vec![],
            returns: vec![],
            returned_by: vec![],
            field_of: vec![],
            has_fields: vec![],
            implements: vec![],
            implemented_by: vec![],
        },
        resolution: Resolution::Exact("EXPAND_GRAPH::SRC/QUERY.RS".to_string()),
    };

    let output = format_map_result(&result);
    assert!(output.contains("expand_graph [function]"));
    assert!(output.contains("file: src/query.rs:388"));
    assert!(output.contains("signature: pub fn expand_graph"));
    assert!(output.contains("cluster: graph-traversal"));
    assert!(output.contains("calls:"));
    assert!(output.contains("get_relationships_for_entity [function] src/store.rs:408"));
    assert!(output.contains("called_by:"));
    assert!(output.contains("cmd_vector_search [function] src/main.rs:566"));
}

#[test]
fn test_format_map_result_no_cluster() {
    let result = MapResult {
        entity: EntityDetail {
            name: "main".to_string(),
            kind: "function".to_string(),
            file_path: "src/main.rs".to_string(),
            line: 70,
            signature: "async fn main() -> Result<()>".to_string(),
            cluster_label: None,
            calls: vec![],
            called_by: vec![],
            references: vec![],
            referenced_by: vec![],
            accepts: vec![],
            accepted_by: vec![],
            returns: vec![],
            returned_by: vec![],
            field_of: vec![],
            has_fields: vec![],
            implements: vec![],
            implemented_by: vec![],
        },
        resolution: Resolution::Exact("MAIN::SRC/MAIN.RS".to_string()),
    };

    let output = format_map_result(&result);
    assert!(output.contains("main [function]"));
    assert!(!output.contains("cluster:"));
}

#[test]
fn test_format_map_result_ambiguous_suffix() {
    let result = MapResult {
        entity: EntityDetail {
            name: "handler".to_string(),
            kind: "function".to_string(),
            file_path: "src/handler.rs".to_string(),
            line: 10,
            signature: "fn handler()".to_string(),
            cluster_label: None,
            calls: vec![
                Candidate {
                    name: "helper".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/b.rs".to_string(),
                    line: 5,
                    ambiguous: false,
                },
                Candidate {
                    name: "process".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/c.rs".to_string(),
                    line: 20,
                    ambiguous: true,
                },
            ],
            called_by: vec![],
            references: vec![],
            referenced_by: vec![],
            accepts: vec![],
            accepted_by: vec![],
            returns: vec![],
            returned_by: vec![],
            field_of: vec![],
            has_fields: vec![],
            implements: vec![],
            implemented_by: vec![],
        },
        resolution: Resolution::Exact("HANDLER::SRC/HANDLER.RS".to_string()),
    };

    let output = format_map_result(&result);
    // Non-ambiguous candidate should NOT have the suffix
    assert!(output.contains("helper [function] src/b.rs:5"));
    assert!(!output.contains("helper [function] src/b.rs:5 (ambiguous)"));
    // Ambiguous candidate should have the suffix
    assert!(output.contains("process [function] src/c.rs:20 (ambiguous)"));
}

#[test]
fn test_format_map_result_type_relationships() {
    let result = MapResult {
        entity: EntityDetail {
            name: "ObservatoryState".to_string(),
            kind: "type".to_string(),
            file_path: "src/state.rs".to_string(),
            line: 10,
            signature: "pub struct ObservatoryState".to_string(),
            cluster_label: None,
            calls: vec![],
            called_by: vec![],
            references: vec![],
            referenced_by: vec![],
            accepts: vec![],
            accepted_by: vec![
                Candidate {
                    name: "sync_timeline_track".to_string(),
                    kind: "function".to_string(),
                    file_path: "src/sync.rs".to_string(),
                    line: 20,
                    ambiguous: false,
                },
            ],
            returns: vec![],
            returned_by: vec![],
            field_of: vec![],
            has_fields: vec![],
            implements: vec![],
            implemented_by: vec![],
        },
        resolution: Resolution::Exact("OBSERVATORYSTATE::SRC/STATE.RS".to_string()),
    };

    let output = format_map_result(&result);
    assert!(output.contains("accepted_by:"));
    assert!(output.contains("sync_timeline_track [function] src/sync.rs:20"));
}
