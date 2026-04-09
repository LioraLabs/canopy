use canopy::cli::search::{SearchResult, SearchHit, format_search_result};
use canopy::resolve::Candidate;

#[test]
fn test_format_search_result_basic() {
    let result = SearchResult {
        hits: vec![
            SearchHit {
                file_path: "src/query.rs".to_string(),
                line_range: (388, 466),
                symbols: vec![
                    Candidate {
                        name: "expand_graph_from_entities_capped".to_string(),
                        kind: "function".to_string(),
                        file_path: "src/query.rs".to_string(),
                        line: 388,
                        ambiguous: false,
                    },
                ],
            },
            SearchHit {
                file_path: "src/store.rs".to_string(),
                line_range: (47, 54),
                symbols: vec![],
            },
        ],
        related: vec![],
        truncated: None,
    };

    let output = format_search_result(&result);
    assert!(output.contains("src/query.rs:388-466"));
    assert!(output.contains("expand_graph_from_entities_capped [function]"));
    assert!(output.contains("src/store.rs:47-54"));
}

#[test]
fn test_format_search_result_with_related() {
    let result = SearchResult {
        hits: vec![
            SearchHit {
                file_path: "src/query.rs".to_string(),
                line_range: (388, 466),
                symbols: vec![],
            },
        ],
        related: vec![
            SearchHit {
                file_path: "src/embed.rs".to_string(),
                line_range: (30, 55),
                symbols: vec![
                    Candidate {
                        name: "embed_one".to_string(),
                        kind: "function".to_string(),
                        file_path: "src/embed.rs".to_string(),
                        line: 30,
                        ambiguous: false,
                    },
                ],
            },
        ],
        truncated: None,
    };

    let output = format_search_result(&result);
    assert!(output.contains("related (graph-discovered):"));
    assert!(output.contains("src/embed.rs:30-55"));
    assert!(output.contains("embed_one [function]"));
}

#[test]
fn test_format_search_result_truncated() {
    let result = SearchResult {
        hits: vec![],
        related: vec![],
        truncated: Some(25),
    };

    let output = format_search_result(&result);
    assert!(output.contains("... and 25 more"));
}
