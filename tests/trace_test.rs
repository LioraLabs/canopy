use canopy::cli::trace::{TraceResult, TraceHop, format_trace_result};
use canopy::resolve::{Resolution, Candidate};

#[test]
fn test_format_trace_result_calls_only() {
    let result = TraceResult {
        hops: vec![
            TraceHop {
                from: Candidate { name: "resolve_seed".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 90, ambiguous: false },
                to: Candidate { name: "execute_strategy".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 180, ambiguous: false },
                edge_type: "CALLS".to_string(),
                edge_cost: 1.0,
                forward: true,
            },
            TraceHop {
                from: Candidate { name: "execute_strategy".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 180, ambiguous: false },
                to: Candidate { name: "expand_graph".to_string(), kind: "function".to_string(), file_path: "src/query.rs".to_string(), line: 388, ambiguous: false },
                edge_type: "CALLS".to_string(),
                edge_cost: 1.0,
                forward: true,
            },
        ],
        resolution_from: Resolution::Exact("RESOLVE_SEED::SRC/QUERY.RS".to_string()),
        resolution_to: Resolution::Exact("EXPAND_GRAPH::SRC/QUERY.RS".to_string()),
    };

    let output = format_trace_result(&result);
    assert!(output.contains("resolve_seed -> execute_strategy -> expand_graph"));
    assert!(output.contains("--CALLS-->"));
}

#[test]
fn test_format_trace_result_with_fallback_edge() {
    let result = TraceResult {
        hops: vec![
            TraceHop {
                from: Candidate { name: "foo".to_string(), kind: "function".to_string(), file_path: "src/a.rs".to_string(), line: 1, ambiguous: false },
                to: Candidate { name: "Bar".to_string(), kind: "type".to_string(), file_path: "src/b.rs".to_string(), line: 10, ambiguous: false },
                edge_type: "REFERENCES".to_string(),
                edge_cost: 10.0,
                forward: true,
            },
        ],
        resolution_from: Resolution::Exact("FOO::SRC/A.RS".to_string()),
        resolution_to: Resolution::Exact("BAR::SRC/B.RS".to_string()),
    };

    let output = format_trace_result(&result);
    assert!(output.contains("--REFERENCES-->"));
}

#[test]
fn test_format_trace_result_backward_edge() {
    let result = TraceResult {
        hops: vec![
            TraceHop {
                from: Candidate { name: "handler".to_string(), kind: "function".to_string(), file_path: "src/a.rs".to_string(), line: 1, ambiguous: false },
                to: Candidate { name: "build".to_string(), kind: "function".to_string(), file_path: "src/b.rs".to_string(), line: 10, ambiguous: false },
                edge_type: "CALLS".to_string(),
                edge_cost: 1.0,
                forward: false,
            },
        ],
        resolution_from: Resolution::Exact("HANDLER::SRC/A.RS".to_string()),
        resolution_to: Resolution::Exact("BUILD::SRC/B.RS".to_string()),
    };

    let output = format_trace_result(&result);
    assert!(output.contains("<--CALLS--"));
    assert!(!output.contains("--CALLS-->"));
}

#[test]
fn test_format_empty_trace() {
    let result = TraceResult {
        hops: vec![],
        resolution_from: Resolution::Exact("A::SRC/A.RS".to_string()),
        resolution_to: Resolution::Exact("B::SRC/B.RS".to_string()),
    };

    let output = format_trace_result(&result);
    assert!(output.contains("No trace found"));
}
