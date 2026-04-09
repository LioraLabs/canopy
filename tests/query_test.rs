use canopy::query::{
    format_toon, ChunkResult, EntityInfo, GraphContext, MatchQuality, QueryEngine,
    QueryResult, QueryStats, RankedChunk, RelationshipInfo, WeightedHop,
};
use canopy::store::{ChunkRecord, Store};
use tempfile::TempDir;

fn tmp_store() -> (Store, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("test.redb");
    let store = Store::open(&path).expect("open store");
    (store, dir)
}

// ---------------------------------------------------------------------------
// test_format_toon_basic
// ---------------------------------------------------------------------------

#[test]
fn test_format_toon_basic() {
    let result = QueryResult {
        results: vec![ChunkResult {
            file_path: "src/main.rs".to_string(),
            line_range: (1, 20),
            language: "rust".to_string(),
            score: 0.95,
            content: "fn main() {}".to_string(),
        }],
        graph_context: GraphContext {
            entities: vec![],
            relationships: vec![],
        },
        stats: QueryStats {
            chunks_searched: 100,
            query_ms: 12,
        },
        coverage: None,
        seed_resolution: None,
    };

    let output = format_toon(&result);

    // Should have results header with count
    assert!(
        output.contains("results[1]{file_path,line_range,language,score,content}:"),
        "expected results header, got:\n{output}"
    );

    // Should contain the chunk data
    assert!(output.contains("src/main.rs"), "expected file path");
    assert!(output.contains("1-20"), "expected line range");
    assert!(output.contains("rust"), "expected language");
    assert!(output.contains("0.95"), "expected score");
    assert!(output.contains("fn main() {}"), "expected content");

    // graph_context should be omitted when both sections are empty
    assert!(
        !output.contains("graph_context:"),
        "graph_context should be omitted when empty"
    );

    // Stats must NOT appear in format_toon (MCP output is token-pinching)
    assert!(!output.contains("stats:"), "stats should be absent from format_toon output");
    assert!(!output.contains("chunks_searched"), "chunks_searched should be absent");
    assert!(!output.contains("query_ms"), "query_ms should be absent");
}

// ---------------------------------------------------------------------------
// test_format_toon_with_graph_context
// ---------------------------------------------------------------------------

#[test]
fn test_format_toon_with_graph_context() {
    let result = QueryResult {
        results: vec![
            ChunkResult {
                file_path: "src/renderer/blob.rs".to_string(),
                line_range: (45, 92),
                language: "rust".to_string(),
                score: 0.87,
                content: "impl BlobRenderer { }".to_string(),
            },
            ChunkResult {
                file_path: "src/types/blob.rs".to_string(),
                line_range: (1, 20),
                language: "rust".to_string(),
                score: 0.78,
                content: "pub struct BlobCssStyle { }".to_string(),
            },
        ],
        graph_context: GraphContext {
            entities: vec![
                EntityInfo {
                    name: "BlobRenderer".to_string(),
                    display_name: "BlobRenderer".to_string(),
                    entity_type: "TYPE".to_string(),
                    description: "Draw source for blob rendering".to_string(),
                },
                EntityInfo {
                    name: "gpu_pipeline".to_string(),
                    display_name: "gpu_pipeline".to_string(),
                    entity_type: "MODULE".to_string(),
                    description: "GPU pipeline management".to_string(),
                },
            ],
            relationships: vec![RelationshipInfo {
                source: "BlobRenderer".to_string(),
                target: "BlobDrawSource".to_string(),
                relationship_type: "REFERENCES".to_string(),
                keywords: "draw, render".to_string(),
            }],
        },
        stats: QueryStats {
            chunks_searched: 1240,
            query_ms: 45,
        },
        coverage: None,
        seed_resolution: None,
    };

    let output = format_toon(&result);

    // Results header with 2 items
    assert!(
        output.contains("results[2]{file_path,line_range,language,score,content}:"),
        "expected results[2] header"
    );

    // graph_context section present
    assert!(output.contains("graph_context:"), "expected graph_context section");

    // Entities section with 2 items
    assert!(
        output.contains("entities[2]{name,type,description}:"),
        "expected entities[2] header"
    );
    assert!(output.contains("BlobRenderer"), "expected BlobRenderer entity");
    assert!(output.contains("gpu_pipeline"), "expected gpu_pipeline entity");
    assert!(output.contains("TYPE"), "expected entity type TYPE");
    assert!(output.contains("MODULE"), "expected entity type MODULE");

    // Relationships section with 1 item
    assert!(
        output.contains("relationships[1]{source,target,type,keywords}:"),
        "expected relationships[1] header"
    );
    assert!(output.contains("BlobRenderer"), "expected source entity in rel");
    assert!(output.contains("BlobDrawSource"), "expected target entity in rel");
    assert!(output.contains("REFERENCES"), "expected relationship type");

    // Keywords contain a comma -- should be quoted
    assert!(
        output.contains("\"draw, render\""),
        "keywords with comma should be quoted, got:\n{output}"
    );

    // Stats must NOT appear in format_toon (MCP output is token-pinching)
    assert!(!output.contains("chunks_searched"), "chunks_searched should be absent");
    assert!(!output.contains("query_ms"), "query_ms should be absent");
}

// ---------------------------------------------------------------------------
// test_format_toon_quotes_commas_in_content
// ---------------------------------------------------------------------------

#[test]
fn test_format_toon_quotes_commas_in_content() {
    let result = QueryResult {
        results: vec![ChunkResult {
            file_path: "src/lib.rs".to_string(),
            line_range: (5, 10),
            language: "rust".to_string(),
            score: 0.80,
            content: "fn foo(a: i32, b: i32) -> i32 { a + b }".to_string(),
        }],
        graph_context: GraphContext {
            entities: vec![],
            relationships: vec![],
        },
        stats: QueryStats {
            chunks_searched: 50,
            query_ms: 5,
        },
        coverage: None,
        seed_resolution: None,
    };

    let output = format_toon(&result);

    // Content with commas must be quoted
    assert!(
        output.contains("\"fn foo(a: i32, b: i32) -> i32 { a + b }\""),
        "content containing commas should be double-quoted, got:\n{output}"
    );
}

// ---------------------------------------------------------------------------
// test_graph_expansion
// ---------------------------------------------------------------------------

#[test]
fn test_graph_expansion() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity(
            "EntityA",
            "FUNCTION",
            "Entity A description",
            "test",
            "src/a.rs",
            serde_json::json!({}),
            None,
            None,
        )
        .expect("insert EntityA");

    store
        .insert_entity(
            "EntityB",
            "STRUCT",
            "Entity B description",
            "test",
            "src/b.rs",
            serde_json::json!({}),
            None,
            None,
        )
        .expect("insert EntityB");

    store
        .insert_relationship(
            "EntityA",
            "EntityB",
            "CALLS",
            "invoke, dispatch",
            0.8,
            "A calls B",
            "test",
            false,
        )
        .expect("insert relationship");

    let engine = QueryEngine::new(&store);

    let ctx = engine.expand_graph(&["src/a.rs"], 1);

    let entity_names: Vec<&str> = ctx.entities.iter().map(|e| e.name.as_str()).collect();
    assert!(
        entity_names.contains(&"EntityA"),
        "EntityA should be in entities (seed), got: {:?}",
        entity_names
    );
    assert!(
        entity_names.contains(&"EntityB"),
        "EntityB should be discovered via 1-hop, got: {:?}",
        entity_names
    );

    assert_eq!(ctx.relationships.len(), 1, "expected 1 relationship");
    let rel = &ctx.relationships[0];
    assert_eq!(rel.source, "EntityA");
    assert_eq!(rel.target, "EntityB");
    assert_eq!(rel.relationship_type, "CALLS");

    let entity_a = ctx.entities.iter().find(|e| e.name == "EntityA").unwrap();
    assert_eq!(entity_a.entity_type, "FUNCTION");
    assert_eq!(entity_a.description, "Entity A description");

    let entity_b = ctx.entities.iter().find(|e| e.name == "EntityB").unwrap();
    assert_eq!(entity_b.entity_type, "STRUCT");
    assert_eq!(entity_b.description, "Entity B description");
}

// ---------------------------------------------------------------------------
// test_expand_graph_from_entity_names
// ---------------------------------------------------------------------------

#[test]
fn test_expand_graph_from_entity_names() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("FuncA", "FUNCTION", "function A", "test", "src/a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("FuncB", "FUNCTION", "function B", "test", "src/b.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("FuncC", "FUNCTION", "function C", "test", "src/c.rs", serde_json::json!({}), None, None)
        .unwrap();

    store
        .insert_relationship("FuncA", "FuncB", "REFERENCES", "calls", 0.8, "A refs B", "test", false)
        .unwrap();
    store
        .insert_relationship("FuncB", "FuncC", "REFERENCES", "calls", 0.8, "B refs C", "test", false)
        .unwrap();

    let engine = QueryEngine::new(&store);

    let ctx = engine.expand_graph_from_entities(&["FuncA"], 1);

    let entity_names: Vec<&str> = ctx.entities.iter().map(|e| e.name.as_str()).collect();
    assert!(entity_names.contains(&"FuncA"), "seed entity should be included");
    assert!(entity_names.contains(&"FuncB"), "1-hop neighbor should be included");
    assert!(!entity_names.contains(&"FuncC"), "2-hop neighbor should NOT be included at 1 hop");
}

// ---------------------------------------------------------------------------
// test_expand_graph_filters_defines
// ---------------------------------------------------------------------------

#[test]
fn test_expand_graph_filters_defines() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("MODULE_A", "MODULE", "module A", "test", "src/a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("FuncA", "FUNCTION", "function A", "test", "src/a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("FuncB", "FUNCTION", "function B", "test", "src/b.rs", serde_json::json!({}), None, None)
        .unwrap();

    store
        .insert_relationship("MODULE_A", "FuncA", "DEFINES", "defines", 1.0, "module defines func", "test", false)
        .unwrap();
    store
        .insert_relationship("FuncA", "FuncB", "REFERENCES", "calls", 0.8, "A refs B", "test", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let ctx = engine.expand_graph_from_entities(&["FuncA"], 1);

    let entity_names: Vec<&str> = ctx.entities.iter().map(|e| e.name.as_str()).collect();
    assert!(entity_names.contains(&"FuncB"), "REFERENCES neighbor should be found");
    assert!(!entity_names.contains(&"MODULE_A"), "DEFINES neighbor should be filtered out");

    assert_eq!(ctx.relationships.len(), 1);
    assert_eq!(ctx.relationships[0].relationship_type, "REFERENCES");
}

// ---------------------------------------------------------------------------
// test_expand_graph_caps_entities
// ---------------------------------------------------------------------------

#[test]
fn test_expand_graph_caps_entities() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("Hub", "FUNCTION", "hub function", "test", "src/hub.rs", serde_json::json!({}), None, None)
        .unwrap();

    for i in 0..15 {
        let name = format!("Spoke{}", i);
        let file = format!("src/spoke{}.rs", i);
        store
            .insert_entity(&name, "FUNCTION", &format!("spoke {}", i), "test", &file, serde_json::json!({}), None, None)
            .unwrap();
        store
            .insert_relationship(
                "Hub",
                &name,
                "REFERENCES",
                "calls",
                0.8 - (i as f64 * 0.01),
                &format!("Hub refs {}", name),
                "test",
                false,
            )
            .unwrap();
    }

    let engine = QueryEngine::new(&store);

    let ctx = engine.expand_graph_from_entities_capped(&["Hub"], 1, 5);

    assert!(
        ctx.entities.len() <= 5,
        "entity count should be capped at 5, got {}",
        ctx.entities.len()
    );

    let entity_names: Vec<&str> = ctx.entities.iter().map(|e| e.name.as_str()).collect();
    assert!(entity_names.contains(&"Hub"), "seed must be in results");
}

// ---------------------------------------------------------------------------
// resolve_seed tests
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_seed_exact_match_legacy() {
    let (store, _dir) = tmp_store();
    store
        .insert_entity(
            "MYFUNC::SRC/A.RS",
            "FUNCTION",
            "desc",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 1}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let resolved = engine.resolve_seed("MYFUNC::SRC/A.RS");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].entity_name, "MYFUNC::SRC/A.RS");
    assert_eq!(resolved[0].match_quality, MatchQuality::Exact);
}

#[test]
fn test_resolve_seed_uppercase_normalization() {
    let (store, _dir) = tmp_store();
    store
        .insert_entity(
            "MYFUNC::SRC/A.RS",
            "FUNCTION",
            "desc",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 1}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let resolved = engine.resolve_seed("myfunc::src/a.rs");
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].entity_name, "MYFUNC::SRC/A.RS");
    assert_eq!(resolved[0].match_quality, MatchQuality::Exact);
}

#[test]
fn test_resolve_seed_fuzzy_substring() {
    let (store, _dir) = tmp_store();
    store
        .insert_entity(
            "MYFUNC::SRC/A.RS",
            "FUNCTION",
            "desc",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 1}),
            None,
            None,
        )
        .unwrap();
    store
        .insert_entity(
            "MYFUNC::SRC/B.RS",
            "FUNCTION",
            "desc",
            "canopy",
            "src/b.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let resolved = engine.resolve_seed("MYFUNC");
    assert_eq!(resolved.len(), 2);
    assert!(resolved.iter().any(|m| m.entity_name == "MYFUNC::SRC/A.RS"));
    assert!(resolved.iter().any(|m| m.entity_name == "MYFUNC::SRC/B.RS"));
    assert!(resolved.iter().all(|m| m.match_quality == MatchQuality::NameExact));
}

#[test]
fn test_resolve_seed_no_match() {
    let (store, _dir) = tmp_store();
    let engine = QueryEngine::new(&store);
    let resolved = engine.resolve_seed("DOES_NOT_EXIST");
    assert!(resolved.is_empty());
}

#[test]
fn test_resolve_seed_exact_match() {
    let (store, _dir) = tmp_store();
    store.insert_entity("DETECT_HUBS::SRC/STORE.RS", "FUNCTION", "fn detect_hubs()", "test", "src/store.rs", serde_json::json!({}), None, None).unwrap();
    let engine = QueryEngine::new(&store);
    let matches = engine.resolve_seed("DETECT_HUBS::SRC/STORE.RS");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].entity_name, "DETECT_HUBS::SRC/STORE.RS");
    assert_eq!(matches[0].match_quality, MatchQuality::Exact);
}

#[test]
fn test_resolve_seed_name_exact() {
    let (store, _dir) = tmp_store();
    store.insert_entity("DETECT_HUBS::SRC/STORE.RS", "FUNCTION", "fn detect_hubs()", "test", "src/store.rs", serde_json::json!({}), None, None).unwrap();
    let engine = QueryEngine::new(&store);
    let matches = engine.resolve_seed("detect_hubs");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].entity_name, "DETECT_HUBS::SRC/STORE.RS");
    assert_eq!(matches[0].match_quality, MatchQuality::NameExact);
}

#[test]
fn test_resolve_seed_name_exact_multiple_files() {
    let (store, _dir) = tmp_store();
    store.insert_entity("RUN::SRC/A.RS", "FUNCTION", "fn run()", "test", "src/a.rs", serde_json::json!({}), None, None).unwrap();
    store.insert_entity("RUN::SRC/B.RS", "FUNCTION", "fn run()", "test", "src/b.rs", serde_json::json!({}), None, None).unwrap();
    let engine = QueryEngine::new(&store);
    let matches = engine.resolve_seed("run");
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|m| m.match_quality == MatchQuality::NameExact));
}

#[test]
fn test_resolve_seed_prefix_match() {
    let (store, _dir) = tmp_store();
    store.insert_entity("DETECT_HUBS::SRC/STORE.RS", "FUNCTION", "fn detect_hubs()", "test", "src/store.rs", serde_json::json!({}), None, None).unwrap();
    store.insert_entity("DETECT_CYCLES::SRC/GRAPH.RS", "FUNCTION", "fn detect_cycles()", "test", "src/graph.rs", serde_json::json!({}), None, None).unwrap();
    store.insert_entity("HUB_DETECTOR::SRC/STORE.RS", "TYPE", "struct HubDetector", "test", "src/store.rs", serde_json::json!({}), None, None).unwrap();
    let engine = QueryEngine::new(&store);
    let matches = engine.resolve_seed("detect");
    assert!(matches.len() >= 2);
    assert!(matches.iter().any(|m| m.entity_name == "DETECT_HUBS::SRC/STORE.RS"));
    assert!(matches.iter().any(|m| m.entity_name == "DETECT_CYCLES::SRC/GRAPH.RS"));
}

#[test]
fn test_resolve_seed_function_preferred_over_type() {
    let (store, _dir) = tmp_store();
    store.insert_entity("STORE::SRC/STORE.RS", "TYPE", "struct Store", "test", "src/store.rs", serde_json::json!({}), None, None).unwrap();
    store.insert_entity("STORE::SRC/MAIN.RS", "FUNCTION", "fn store()", "test", "src/main.rs", serde_json::json!({}), None, None).unwrap();
    let engine = QueryEngine::new(&store);
    let matches = engine.resolve_seed("store");
    assert!(matches.len() == 2);
    assert_eq!(matches[0].entity_name, "STORE::SRC/MAIN.RS", "FUNCTION should be first");
}

#[test]
fn test_seed_resolution_is_trivial() {
    let empty = canopy::query::SeedResolution::default();
    assert!(empty.is_trivial());

    let with_suggestions = canopy::query::SeedResolution {
        suggestions: vec!["Foo".to_string()],
        ..Default::default()
    };
    assert!(!with_suggestions.is_trivial());
}

// ---------------------------------------------------------------------------
// find_path tests
// ---------------------------------------------------------------------------

#[test]
fn test_find_path() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();

    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("B::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 10}), None, None)
        .unwrap();
    store
        .insert_entity("C::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 20}), None, None)
        .unwrap();

    store
        .insert_relationship("A::F", "B::F", "CALLS", "calls", 1.0, "", "canopy", false)
        .unwrap();
    store
        .insert_relationship("B::F", "C::F", "CALLS", "calls", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let path = engine.find_path("A::F", "C::F", &[], false);
    assert!(path.is_some(), "path should be found");
    let path = path.unwrap();
    assert_eq!(path.entities.len(), 3, "path should have 3 entities (A, B, C)");
    assert_eq!(path.relationships.len(), 2, "path should have 2 relationships");

    assert_eq!(path.entities[0].name, "A::F");
    assert_eq!(path.entities[2].name, "C::F");
}

#[test]
fn test_find_path_no_path() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("B::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 10}), None, None)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let path = engine.find_path("A::F", "B::F", &[], false);
    assert!(path.is_none(), "no path should be found when entities are disconnected");
}

#[test]
fn test_find_path_direct() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("X::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("Y::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();

    store
        .insert_relationship("X::F", "Y::F", "CALLS", "calls", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let path = engine.find_path("X::F", "Y::F", &[], false);
    assert!(path.is_some());
    let path = path.unwrap();
    assert_eq!(path.entities.len(), 2);
    assert_eq!(path.relationships.len(), 1);
}

// ---------------------------------------------------------------------------
// entities_to_chunks tests
// ---------------------------------------------------------------------------

#[test]
fn test_entities_to_chunks_basic() {
    let (store, _dir) = tmp_store();

    store
        .insert_chunk(ChunkRecord {
            file_path: "src/a.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec![],
            line_range: (1, 20),
            parent_scope: "".to_string(),
            content: "fn my_func() {}".to_string(),
        }, &[0.1, 0.2, 0.3])
        .unwrap();

    store
        .insert_entity(
            "MY_FUNC::SRC/A.RS",
            "FUNCTION",
            "my func",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let chunks = engine.entities_to_chunks(&["MY_FUNC::SRC/A.RS".to_string()]);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].file_path, "src/a.rs");
    assert_eq!(chunks[0].line_range, (1, 20));
    assert_eq!(chunks[0].score, 1.0);
}

#[test]
fn test_entities_to_chunks_no_overlap() {
    let (store, _dir) = tmp_store();

    store
        .insert_chunk(ChunkRecord {
            file_path: "src/a.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec![],
            line_range: (50, 100),
            parent_scope: "".to_string(),
            content: "fn later() {}".to_string(),
        }, &[0.1, 0.2, 0.3])
        .unwrap();

    store
        .insert_entity(
            "EARLY::SRC/A.RS",
            "FUNCTION",
            "early func",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let chunks = engine.entities_to_chunks(&["EARLY::SRC/A.RS".to_string()]);
    assert_eq!(chunks.len(), 0, "entity at line 5 should not match chunk at 50-100");
}

#[test]
fn test_entities_to_chunks_deduplicates() {
    let (store, _dir) = tmp_store();

    store
        .insert_chunk(ChunkRecord {
            file_path: "src/a.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec![],
            line_range: (1, 50),
            parent_scope: "".to_string(),
            content: "fn a() {} fn b() {}".to_string(),
        }, &[0.1, 0.2, 0.3])
        .unwrap();

    store
        .insert_entity(
            "FUNC_A::SRC/A.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();
    store
        .insert_entity(
            "FUNC_B::SRC/A.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 20}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let chunks = engine.entities_to_chunks(&[
        "FUNC_A::SRC/A.RS".to_string(),
        "FUNC_B::SRC/A.RS".to_string(),
    ]);
    assert_eq!(chunks.len(), 1, "both entities in the same chunk should produce 1 result");
}

// ---------------------------------------------------------------------------
// format_toon enrichment tests
// ---------------------------------------------------------------------------

#[test]
fn test_format_toon_emits_enrichment_fields() {
    let result = QueryResult {
        results: vec![],
        graph_context: GraphContext { entities: vec![], relationships: vec![] },
        stats: QueryStats { chunks_searched: 0, query_ms: 0 },
        coverage: Some(canopy::query::Coverage { returned: 15, total: 47 }),
        seed_resolution: Some(canopy::query::SeedResolution {
            input: Some("Foo".into()),
            suggestions: vec!["Foobar".into(), "FooBaz".into()],
            ..Default::default()
        }),
    };
    let output = format_toon(&result);

    assert!(output.contains("coverage:"), "expected coverage section, got:\n{output}");
    assert!(output.contains("returned: 15"), "expected returned: 15");
    assert!(output.contains("total: 47"), "expected total: 47");

    assert!(output.contains("seed_resolution:"), "expected seed_resolution section");
    assert!(output.contains("Foo"), "expected input Foo");
    assert!(output.contains("Foobar"), "expected suggestion Foobar");
}

#[test]
fn test_format_toon_omits_empty_enrichment() {
    let result = QueryResult {
        results: vec![],
        graph_context: GraphContext { entities: vec![], relationships: vec![] },
        stats: QueryStats { chunks_searched: 0, query_ms: 0 },
        coverage: None,
        seed_resolution: None,
    };
    let output = format_toon(&result);
    assert!(!output.contains("coverage:"), "coverage should be omitted when None");
    assert!(!output.contains("seed_resolution:"), "seed_resolution should be omitted when None");
}

#[test]
fn test_seed_resolution_from_question_field() {
    let sr = canopy::query::SeedResolution {
        resolved: Some("handle_login::auth.rs".into()),
        from: Some("question".into()),
        ..Default::default()
    };
    assert_eq!(sr.from.as_deref(), Some("question"));
    assert_eq!(sr.resolved.as_deref(), Some("handle_login::auth.rs"));
    assert!(!sr.is_trivial());
}

// ---------------------------------------------------------------------------
// suggest_similar_entities tests
// ---------------------------------------------------------------------------

#[test]
fn test_resolve_seed_suggestions_on_empty() {
    let (store, _dir) = tmp_store();

    for name in [
        "TimelineTrack::f.rs",
        "Playhead::f.rs",
        "TimelineCursor::f.rs",
        "Unrelated::g.rs",
    ] {
        store
            .insert_entity(
                name,
                "TYPE",
                "t",
                "s",
                "f.rs",
                serde_json::json!({}),
                None,
                Some("pub".into()),
            )
            .unwrap();
    }

    let engine = QueryEngine::new(&store);
    let suggestions = engine.suggest_similar_entities("TimelinePlayhead", 3);

    assert!(suggestions.len() <= 3);
    assert!(
        suggestions.iter().any(|s| s.contains("TIMELINETRACK")
            || s.contains("PLAYHEAD")
            || s.contains("TimelineTrack")
            || s.contains("Playhead")),
        "expected TimelineTrack or Playhead in suggestions, got {:?}",
        suggestions
    );
}

// ---------------------------------------------------------------------------
// New pipeline tests: graph_rerank
// ---------------------------------------------------------------------------

#[test]
fn test_graph_rerank_basic() {
    let (store, _dir) = tmp_store();

    // Insert two chunks in different files
    let cid1 = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn func_a() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    let cid2 = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/b.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn func_b() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    // Insert entities at line 5 (within the chunks)
    store
        .insert_entity(
            "FUNC_A::SRC/A.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();
    store
        .insert_entity(
            "FUNC_B::SRC/B.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/b.rs",
            serde_json::json!({"line": 5}),
            None,
            None,
        )
        .unwrap();

    // Create a relationship so they have connectivity
    store
        .insert_relationship(
            "FUNC_A::SRC/A.RS",
            "FUNC_B::SRC/B.RS",
            "CALLS",
            "",
            1.0,
            "",
            "canopy",
            false,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);

    // Simulate raw vector hits with equal similarity
    let hits: Vec<(u64, f32)> = vec![(cid1, 0.9), (cid2, 0.8)];
    let ranked = engine.graph_rerank(&hits, 2, 0.0);

    assert_eq!(ranked.len(), 2);
    // Both chunks should have entity_keys populated
    assert!(!ranked[0].entity_keys.is_empty(), "first ranked chunk should have entities");
    assert!(!ranked[1].entity_keys.is_empty(), "second ranked chunk should have entities");
    // Scores should be > 0
    assert!(ranked[0].score > 0.0);
    assert!(ranked[1].score > 0.0);
    // Should be sorted by score descending
    assert!(ranked[0].score >= ranked[1].score);
}

#[test]
fn test_graph_rerank_empty_input() {
    let (store, _dir) = tmp_store();
    let engine = QueryEngine::new(&store);
    let ranked = engine.graph_rerank(&[], 10, 0.0);
    assert!(ranked.is_empty());
}

#[test]
fn test_graph_rerank_truncates_to_top_k() {
    let (store, _dir) = tmp_store();

    // Insert 5 chunks
    let mut chunk_ids = Vec::new();
    for i in 0..5 {
        let cid = store
            .insert_chunk(
                ChunkRecord {
                    file_path: format!("src/{i}.rs"),
                    language: "rust".to_string(),
                    node_kinds: vec![],
                    line_range: (1, 20),
                    parent_scope: "".to_string(),
                    content: format!("fn func_{i}() {{}}"),
                },
                &[0.1, 0.2, 0.3],
            )
            .unwrap();
        chunk_ids.push(cid);
    }

    let engine = QueryEngine::new(&store);
    let hits: Vec<(u64, f32)> = chunk_ids.iter().map(|id| (*id, 0.8)).collect();
    let ranked = engine.graph_rerank(&hits, 2, 0.0);
    assert_eq!(ranked.len(), 2, "should truncate to top_k=2");
}

#[test]
fn test_graph_rerank_connectivity_boosts_score() {
    let (store, _dir) = tmp_store();

    // Chunk A has an entity connected to chunk B's entity
    let cid_a = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn a() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    let cid_b = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/b.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn b() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    // Chunk C has no connections to A or B
    let cid_c = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/c.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn c() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    store
        .insert_entity("A::SRC/A.RS", "FUNCTION", "", "canopy", "src/a.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();
    store
        .insert_entity("B::SRC/B.RS", "FUNCTION", "", "canopy", "src/b.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();
    store
        .insert_entity("C::SRC/C.RS", "FUNCTION", "", "canopy", "src/c.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();

    // A and B are connected
    store
        .insert_relationship("A::SRC/A.RS", "B::SRC/B.RS", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);

    // All have same vector similarity (0.5), but A and B should get connectivity boost
    let hits: Vec<(u64, f32)> = vec![(cid_a, 0.5), (cid_b, 0.5), (cid_c, 0.5)];
    let ranked = engine.graph_rerank(&hits, 3, 0.0);

    assert_eq!(ranked.len(), 3);

    // Find scores for A/B vs C
    let score_a = ranked.iter().find(|r| r.file_path == "src/a.rs").unwrap().score;
    let score_b = ranked.iter().find(|r| r.file_path == "src/b.rs").unwrap().score;
    let score_c = ranked.iter().find(|r| r.file_path == "src/c.rs").unwrap().score;

    // A and B should be boosted above C due to connectivity
    assert!(
        score_a > score_c,
        "connected chunk A ({}) should score higher than isolated C ({})",
        score_a, score_c
    );
    assert!(
        score_b > score_c,
        "connected chunk B ({}) should score higher than isolated C ({})",
        score_b, score_c
    );
}

#[test]
fn test_graph_rerank_penalizes_test_paths() {
    let (store, _dir) = tmp_store();

    let prod_cid = store.insert_chunk(
        ChunkRecord {
            file_path: "src/handler.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec![],
            line_range: (1, 20),
            parent_scope: "".to_string(),
            content: "fn handle_request() {}".to_string(),
        },
        &[0.1, 0.2, 0.3],
    ).unwrap();

    let test_cid = store.insert_chunk(
        ChunkRecord {
            file_path: "tests/handler_test.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec![],
            line_range: (1, 20),
            parent_scope: "".to_string(),
            content: "fn test_handle_request() {}".to_string(),
        },
        &[0.1, 0.2, 0.3],
    ).unwrap();

    store.insert_entity("HANDLE_REQUEST::SRC/HANDLER.RS", "FUNCTION", "", "canopy", "src/handler.rs", serde_json::json!({"name": "handle_request", "line": 5}), None, None).unwrap();
    store.insert_entity("TEST_HANDLE_REQUEST::TESTS/HANDLER_TEST.RS", "FUNCTION", "", "canopy", "tests/handler_test.rs", serde_json::json!({"name": "test_handle_request", "line": 5}), None, None).unwrap();

    let engine = QueryEngine::new(&store);
    let hits: Vec<(u64, f32)> = vec![(prod_cid, 0.9), (test_cid, 0.9)];

    // With penalty 0.3: test chunk score *= 0.7
    let ranked = engine.graph_rerank(&hits, 2, 0.3);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].file_path, "src/handler.rs");
    assert!(ranked[0].score > ranked[1].score);

    // With penalty 0.0: no demotion, scores should be equal
    let ranked_no_penalty = engine.graph_rerank(&hits, 2, 0.0);
    assert!((ranked_no_penalty[0].score - ranked_no_penalty[1].score).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// New pipeline tests: neighbor_pull_in
// ---------------------------------------------------------------------------

#[test]
fn test_neighbor_pull_in_basic() {
    let (store, _dir) = tmp_store();

    // Chunk for seed entity
    let cid_a = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn a() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    // Chunk for neighbor entity
    store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/b.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn b() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    store
        .insert_entity("A::SRC/A.RS", "FUNCTION", "", "canopy", "src/a.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();
    store
        .insert_entity("B::SRC/B.RS", "FUNCTION", "", "canopy", "src/b.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();

    // A calls B
    store
        .insert_relationship("A::SRC/A.RS", "B::SRC/B.RS", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);

    let seed_hits = vec![RankedChunk {
        chunk_id: cid_a,
        file_path: "src/a.rs".to_string(),
        line_range: (1, 20),
        entity_keys: vec!["A::SRC/A.RS".to_string()],
        score: 0.9,
    }];

    let pulled = engine.neighbor_pull_in(&seed_hits, 5);
    assert_eq!(pulled.len(), 1, "should pull in B via CALLS edge");
    assert_eq!(pulled[0].file_path, "src/b.rs");
}

#[test]
fn test_neighbor_pull_in_only_calls_edges() {
    let (store, _dir) = tmp_store();

    let cid_a = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn a() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/b.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn b() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    store
        .insert_entity("A::SRC/A.RS", "FUNCTION", "", "canopy", "src/a.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();
    store
        .insert_entity("B::SRC/B.RS", "FUNCTION", "", "canopy", "src/b.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();

    // A references B (not CALLS)
    store
        .insert_relationship("A::SRC/A.RS", "B::SRC/B.RS", "REFERENCES", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);

    let seed_hits = vec![RankedChunk {
        chunk_id: cid_a,
        file_path: "src/a.rs".to_string(),
        line_range: (1, 20),
        entity_keys: vec!["A::SRC/A.RS".to_string()],
        score: 0.9,
    }];

    let pulled = engine.neighbor_pull_in(&seed_hits, 5);
    assert!(pulled.is_empty(), "should not pull in via non-CALLS edges");
}

#[test]
fn test_neighbor_pull_in_respects_max() {
    let (store, _dir) = tmp_store();

    let cid_a = store
        .insert_chunk(
            ChunkRecord {
                file_path: "src/a.rs".to_string(),
                language: "rust".to_string(),
                node_kinds: vec![],
                line_range: (1, 20),
                parent_scope: "".to_string(),
                content: "fn a() {}".to_string(),
            },
            &[0.1, 0.2, 0.3],
        )
        .unwrap();

    store
        .insert_entity("A::SRC/A.RS", "FUNCTION", "", "canopy", "src/a.rs", serde_json::json!({"line": 5}), None, None)
        .unwrap();

    // Create 5 neighbors via CALLS
    for i in 0..5 {
        let file = format!("src/n{i}.rs");
        let name = format!("N{i}::SRC/N{i}.RS");
        store
            .insert_chunk(
                ChunkRecord {
                    file_path: file.clone(),
                    language: "rust".to_string(),
                    node_kinds: vec![],
                    line_range: (1, 20),
                    parent_scope: "".to_string(),
                    content: format!("fn n{i}() {{}}"),
                },
                &[0.1, 0.2, 0.3],
            )
            .unwrap();
        store
            .insert_entity(&name, "FUNCTION", "", "canopy", &file, serde_json::json!({"line": 5}), None, None)
            .unwrap();
        store
            .insert_relationship("A::SRC/A.RS", &name, "CALLS", "", 1.0, "", "canopy", false)
            .unwrap();
    }

    let engine = QueryEngine::new(&store);

    let seed_hits = vec![RankedChunk {
        chunk_id: cid_a,
        file_path: "src/a.rs".to_string(),
        line_range: (1, 20),
        entity_keys: vec!["A::SRC/A.RS".to_string()],
        score: 0.9,
    }];

    let pulled = engine.neighbor_pull_in(&seed_hits, 2);
    assert_eq!(pulled.len(), 2, "should respect max_related=2");
}

// ---------------------------------------------------------------------------
// New pipeline tests: build_map
// ---------------------------------------------------------------------------

#[test]
fn test_build_map_basic() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity(
            "HANDLE::SRC/A.RS",
            "FUNCTION",
            "fn handle()",
            "canopy",
            "src/a.rs",
            serde_json::json!({"name": "handle", "kind": "function", "line": 10, "signature": "fn handle()"}),
            None,
            None,
        )
        .unwrap();

    store
        .insert_entity(
            "CALLEE::SRC/B.RS",
            "FUNCTION",
            "fn callee()",
            "canopy",
            "src/b.rs",
            serde_json::json!({"name": "callee", "kind": "function", "line": 5}),
            None,
            None,
        )
        .unwrap();

    store
        .insert_entity(
            "CALLER::SRC/C.RS",
            "FUNCTION",
            "fn caller()",
            "canopy",
            "src/c.rs",
            serde_json::json!({"name": "caller", "kind": "function", "line": 1}),
            None,
            None,
        )
        .unwrap();

    // handle calls callee
    store
        .insert_relationship("HANDLE::SRC/A.RS", "CALLEE::SRC/B.RS", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();
    // caller calls handle
    store
        .insert_relationship("CALLER::SRC/C.RS", "HANDLE::SRC/A.RS", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let detail = engine.build_map("HANDLE::SRC/A.RS", 10);
    assert!(detail.is_some());

    let d = detail.unwrap();
    assert_eq!(d.name, "handle");
    assert_eq!(d.kind, "function");
    assert_eq!(d.file_path, "src/a.rs");
    assert_eq!(d.line, 10);
    assert_eq!(d.signature, "fn handle()");

    assert_eq!(d.calls.len(), 1, "should have 1 callee");
    assert_eq!(d.calls[0].name, "callee");

    assert_eq!(d.called_by.len(), 1, "should have 1 caller");
    assert_eq!(d.called_by[0].name, "caller");
}

#[test]
fn test_build_map_nonexistent_entity() {
    let (store, _dir) = tmp_store();
    let engine = QueryEngine::new(&store);
    let detail = engine.build_map("DOES_NOT_EXIST", 10);
    assert!(detail.is_none());
}

#[test]
fn test_build_map_respects_max_per_category() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity(
            "HUB::SRC/HUB.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/hub.rs",
            serde_json::json!({"name": "hub", "kind": "function", "line": 1}),
            None,
            None,
        )
        .unwrap();

    // Create 10 callees
    for i in 0..10 {
        let name = format!("TARGET{i}::SRC/T{i}.RS");
        store
            .insert_entity(
                &name,
                "FUNCTION",
                "",
                "canopy",
                &format!("src/t{i}.rs"),
                serde_json::json!({"name": format!("target{i}"), "kind": "function", "line": 1}),
                None,
                None,
            )
            .unwrap();
        store
            .insert_relationship("HUB::SRC/HUB.RS", &name, "CALLS", "", 1.0, "", "canopy", false)
            .unwrap();
    }

    let engine = QueryEngine::new(&store);
    let detail = engine.build_map("HUB::SRC/HUB.RS", 3).unwrap();
    assert_eq!(detail.calls.len(), 3, "should respect max_per_category=3");
}

#[test]
fn test_build_map_with_cluster_label() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity(
            "FUNC::SRC/A.RS",
            "FUNCTION",
            "",
            "canopy",
            "src/a.rs",
            serde_json::json!({"name": "func", "kind": "function", "line": 1}),
            None,
            None,
        )
        .unwrap();

    // Set up cluster
    let mut cluster_map = std::collections::HashMap::new();
    cluster_map.insert("FUNC::SRC/A.RS".to_string(), 42);
    store.store_clusters(&cluster_map).unwrap();

    let mut labels = std::collections::HashMap::new();
    labels.insert(42, "authentication".to_string());
    store.store_cluster_meta(&labels).unwrap();

    let engine = QueryEngine::new(&store);
    let detail = engine.build_map("FUNC::SRC/A.RS", 10).unwrap();
    assert_eq!(detail.cluster_label.as_deref(), Some("authentication"));
}

// ---------------------------------------------------------------------------
// New pipeline tests: find_weighted_path
// ---------------------------------------------------------------------------

#[test]
fn test_find_weighted_path_calls_only() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("B::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 10}), None, None)
        .unwrap();
    store
        .insert_entity("C::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 20}), None, None)
        .unwrap();

    store
        .insert_relationship("A::F", "B::F", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();
    store
        .insert_relationship("B::F", "C::F", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let hops = engine.find_weighted_path("A::F", "C::F", 5);

    assert_eq!(hops.len(), 2, "path A->B->C should have 2 hops");
    assert_eq!(hops[0].from_key, "A::F");
    assert_eq!(hops[0].to_key, "B::F");
    assert_eq!(hops[0].edge_type, "CALLS");
    assert!((hops[0].edge_cost - 1.0).abs() < 0.01);

    assert_eq!(hops[1].from_key, "B::F");
    assert_eq!(hops[1].to_key, "C::F");
}

#[test]
fn test_find_weighted_path_prefers_calls() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("B::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 10}), None, None)
        .unwrap();
    store
        .insert_entity("C::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 20}), None, None)
        .unwrap();

    // Direct REFERENCES path A->C (cost 10)
    store
        .insert_relationship("A::F", "C::F", "REFERENCES", "", 1.0, "", "canopy", false)
        .unwrap();
    // Indirect CALLS path A->B->C (cost 1+1=2)
    store
        .insert_relationship("A::F", "B::F", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();
    store
        .insert_relationship("B::F", "C::F", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let hops = engine.find_weighted_path("A::F", "C::F", 5);

    // Should prefer the CALLS path (cost 2) over REFERENCES (cost 10)
    assert_eq!(hops.len(), 2, "should take 2-hop CALLS path");
    assert!(
        hops.iter().all(|h| h.edge_type == "CALLS"),
        "all hops should be CALLS"
    );
}

#[test]
fn test_find_weighted_path_no_path() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();
    store
        .insert_entity("B::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 10}), None, None)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let hops = engine.find_weighted_path("A::F", "B::F", 5);
    assert!(hops.is_empty(), "no path should be found");
}

#[test]
fn test_find_weighted_path_respects_max_hops() {
    let (store, _dir) = tmp_store();

    // Chain: A -> B -> C -> D (3 hops)
    for (name, line) in [("A::F", 1), ("B::F", 10), ("C::F", 20), ("D::F", 30)] {
        store
            .insert_entity(name, "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": line}), None, None)
            .unwrap();
    }
    store.insert_relationship("A::F", "B::F", "CALLS", "", 1.0, "", "canopy", false).unwrap();
    store.insert_relationship("B::F", "C::F", "CALLS", "", 1.0, "", "canopy", false).unwrap();
    store.insert_relationship("C::F", "D::F", "CALLS", "", 1.0, "", "canopy", false).unwrap();

    let engine = QueryEngine::new(&store);

    // With max_hops=2, A->D (3 hops) should not be found
    let hops = engine.find_weighted_path("A::F", "D::F", 2);
    assert!(hops.is_empty(), "3-hop path should not be found with max_hops=2");

    // With max_hops=3, it should be found
    let hops = engine.find_weighted_path("A::F", "D::F", 3);
    assert_eq!(hops.len(), 3, "3-hop path should be found with max_hops=3");
}

#[test]
fn test_find_weighted_path_same_node() {
    let (store, _dir) = tmp_store();
    store
        .insert_entity("A::F", "FUNCTION", "", "canopy", "f.rs", serde_json::json!({"line": 1}), None, None)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let hops = engine.find_weighted_path("A::F", "A::F", 5);
    assert!(hops.is_empty(), "path from node to itself should be empty");
}

// ---------------------------------------------------------------------------
// New pipeline tests: get_cluster_detail
// ---------------------------------------------------------------------------

#[test]
fn test_get_cluster_detail_basic() {
    let (store, _dir) = tmp_store();

    // Create entities
    for (name, file) in [("A::SRC/A.RS", "src/a.rs"), ("B::SRC/B.RS", "src/b.rs"), ("C::SRC/C.RS", "src/c.rs")] {
        store
            .insert_entity(
                name,
                "FUNCTION",
                "",
                "canopy",
                file,
                serde_json::json!({"name": name.split("::").next().unwrap().to_lowercase(), "kind": "function", "line": 1}),
                None,
                None,
            )
            .unwrap();
    }

    // Assign all to cluster 7
    let mut cluster_map = std::collections::HashMap::new();
    cluster_map.insert("A::SRC/A.RS".to_string(), 7);
    cluster_map.insert("B::SRC/B.RS".to_string(), 7);
    cluster_map.insert("C::SRC/C.RS".to_string(), 7);
    store.store_clusters(&cluster_map).unwrap();

    // Add intra-cluster relationship
    store
        .insert_relationship("A::SRC/A.RS", "B::SRC/B.RS", "CALLS", "", 1.0, "", "canopy", false)
        .unwrap();

    let engine = QueryEngine::new(&store);
    let result = engine.get_cluster_detail(7, "test-cluster", 10, 10).unwrap();

    assert_eq!(result.label, "test-cluster");
    assert_eq!(result.members.len(), 3);
    assert!(!result.relationships.is_empty());
    assert!(result.truncated_members.is_none());
}

#[test]
fn test_get_cluster_detail_truncates() {
    let (store, _dir) = tmp_store();

    let mut cluster_map = std::collections::HashMap::new();
    for i in 0..10 {
        let name = format!("F{i}::f.rs");
        store
            .insert_entity(
                &name,
                "FUNCTION",
                "",
                "canopy",
                "f.rs",
                serde_json::json!({"name": format!("f{i}"), "kind": "function", "line": i}),
                None,
                None,
            )
            .unwrap();
        cluster_map.insert(name, 3);
    }
    store.store_clusters(&cluster_map).unwrap();

    let engine = QueryEngine::new(&store);
    let result = engine.get_cluster_detail(3, "big-cluster", 5, 10).unwrap();

    assert_eq!(result.members.len(), 5);
    assert_eq!(result.truncated_members, Some(5), "should report 5 truncated members");
}

// ---------------------------------------------------------------------------
// New pipeline tests: entity_key_to_candidate
// ---------------------------------------------------------------------------

#[test]
fn test_entity_key_to_candidate() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity(
            "MYFUNC::SRC/A.RS",
            "FUNCTION",
            "my func",
            "canopy",
            "src/a.rs",
            serde_json::json!({"name": "my_func", "kind": "function", "line": 42}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let candidate = engine.entity_key_to_candidate("MYFUNC::SRC/A.RS");
    assert!(candidate.is_some());

    let c = candidate.unwrap();
    assert_eq!(c.name, "my_func");
    assert_eq!(c.kind, "function");
    assert_eq!(c.file_path, "src/a.rs");
    assert_eq!(c.line, 42);
}

#[test]
fn test_entity_key_to_candidate_missing() {
    let (store, _dir) = tmp_store();
    let engine = QueryEngine::new(&store);
    let candidate = engine.entity_key_to_candidate("NONEXISTENT");
    assert!(candidate.is_none());
}

#[test]
fn test_entity_key_to_candidate_fallback_name() {
    let (store, _dir) = tmp_store();

    // Entity without "name" in metadata
    store
        .insert_entity(
            "MYFUNC::SRC/A.RS",
            "FUNCTION",
            "my func",
            "canopy",
            "src/a.rs",
            serde_json::json!({"line": 10}),
            None,
            None,
        )
        .unwrap();

    let engine = QueryEngine::new(&store);
    let c = engine.entity_key_to_candidate("MYFUNC::SRC/A.RS").unwrap();
    // Fallback: name portion before "::"
    assert_eq!(c.name, "MYFUNC");
}
