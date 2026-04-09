use canopy::chunker::{Chunk, SymbolDef};
use canopy::graph::*;
use std::collections::HashMap;
use canopy::chunker::{RawFieldDef, RawTypeRef, TypeRefPosition};

fn make_chunk(
    file_path: &str,
    parent_scope: &str,
    defines: Vec<SymbolDef>,
    references: Vec<String>,
) -> Chunk {
    Chunk {
        content: String::new(),
        file_path: file_path.to_string(),
        language: "rust".to_string(),
        node_kinds: vec![],
        line_start: 1,
        line_end: 10,
        parent_scope: parent_scope.to_string(),
        defines,
        free_calls: vec![],
        method_calls: vec![],
        other_refs: references,
        imports: vec![],
        type_refs: vec![],
        field_defs: vec![],
    }
}

fn make_symbol(name: &str, kind: &str, signature: &str) -> SymbolDef {
    SymbolDef {
        name: name.to_string(),
        kind: kind.to_string(),
        line: 1,
        signature: signature.to_string(),
    }
}

#[test]
fn test_entity_metadata_includes_signature() {
    let chunk = make_chunk(
        "src/main.rs",
        "",
        vec![make_symbol("do_stuff", "function", "fn do_stuff() -> bool")],
        vec![],
    );

    let entities = generate_entities(&chunk, "doc-123");
    let e = &entities[0];
    let meta: serde_json::Value = serde_json::from_str(
        &serde_json::to_string(&e.metadata).unwrap()
    ).unwrap();
    assert_eq!(
        meta.get("signature").and_then(|s| s.as_str()),
        Some("fn do_stuff() -> bool")
    );
}

#[test]
fn test_generate_entities_from_chunk() {
    let chunk = make_chunk(
        "src/main.rs",
        "",
        vec![make_symbol("do_stuff", "function", "fn do_stuff() -> bool")],
        vec![],
    );

    let entities = generate_entities(&chunk, "doc-123");
    assert_eq!(entities.len(), 1);

    let e = &entities[0];
    assert_eq!(e.entity_name, "DO_STUFF::SRC/MAIN.RS");
    assert_eq!(e.entity_type, "FUNCTION");
    assert_eq!(e.description, "fn do_stuff() -> bool");
    assert_eq!(e.source_id, "doc-123");
}

#[test]
fn test_generate_module_entity() {
    let e = generate_module_entity("src/lib.rs");
    assert_eq!(e.entity_name, "SRC/LIB.RS");
    assert_eq!(e.entity_type, "MODULE");
    assert_eq!(e.source_id, "canopy");
    assert!(e.description.contains("src/lib.rs"));
}

#[test]
fn test_entity_type_mapping() {
    assert_eq!(entity_type_for_kind("function"), Some("FUNCTION"));
    assert_eq!(entity_type_for_kind("struct"), Some("TYPE"));
    assert_eq!(entity_type_for_kind("class"), Some("TYPE"));
    assert_eq!(entity_type_for_kind("enum"), Some("TYPE"));
    assert_eq!(entity_type_for_kind("record"), Some("TYPE"));
    assert_eq!(entity_type_for_kind("trait"), Some("TRAIT"));
    assert_eq!(entity_type_for_kind("interface"), Some("TRAIT"));
    assert_eq!(entity_type_for_kind("const"), Some("CONSTANT"));
    assert_eq!(entity_type_for_kind("static"), Some("CONSTANT"));
    assert_eq!(entity_type_for_kind("module"), Some("MODULE"));
    assert_eq!(entity_type_for_kind("namespace"), Some("MODULE"));
}

#[test]
fn test_entity_name_normalization() {
    let name = qualified_name("MyStruct", "src/models/user.rs");
    assert_eq!(name, "MYSTRUCT::SRC/MODELS/USER.RS");
}

#[test]
fn test_impl_kind_skipped() {
    let chunk = make_chunk(
        "src/main.rs",
        "",
        vec![make_symbol("MyStruct", "impl", "impl MyStruct")],
        vec![],
    );

    let entities = generate_entities(&chunk, "doc-1");
    assert!(entities.is_empty(), "impl kind should produce no entities");
}

#[test]
fn test_generate_defines_relationships() {
    let entities = vec![
        EntityDef {
            entity_name: "DO_STUFF::SRC/MAIN.RS".to_string(),
            entity_type: "FUNCTION".to_string(),
            description: String::new(),
            source_id: String::new(),
            metadata: serde_json::Value::Null,
            parent: None,
            visibility: None,
        },
        EntityDef {
            entity_name: "MYCONFIG::SRC/MAIN.RS".to_string(),
            entity_type: "TYPE".to_string(),
            description: String::new(),
            source_id: String::new(),
            metadata: serde_json::Value::Null,
            parent: None,
            visibility: None,
        },
    ];

    let rels = generate_defines_relationships("src/main.rs", &entities);
    assert_eq!(rels.len(), 2);
    assert_eq!(rels[0].src_id, "SRC/MAIN.RS");
    assert_eq!(rels[0].tgt_id, "DO_STUFF::SRC/MAIN.RS");
    assert_eq!(rels[0].keywords, "defines, contains");
    assert_eq!(rels[0].weight, 1.0);
    assert_eq!(rels[1].tgt_id, "MYCONFIG::SRC/MAIN.RS");
}

#[test]
fn test_build_symbol_map() {
    let chunks_a = vec![make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("Foo", "struct", "struct Foo")],
        vec![],
    )];
    let chunks_b = vec![make_chunk(
        "src/b.rs",
        "",
        vec![make_symbol("Foo", "function", "fn Foo()")],
        vec![],
    )];

    let file_chunks = vec![
        ("src/a.rs".to_string(), chunks_a),
        ("src/b.rs".to_string(), chunks_b),
    ];

    let mut source_doc_ids = HashMap::new();
    source_doc_ids.insert("src/a.rs".to_string(), vec!["doc-a".to_string()]);
    source_doc_ids.insert("src/b.rs".to_string(), vec!["doc-b".to_string()]);

    let map = build_symbol_map(&file_chunks, &source_doc_ids);

    let foo_entries = map.get("foo").expect("should have 'foo' key");
    assert_eq!(foo_entries.len(), 2);
    assert!(foo_entries.iter().any(|(name, _)| name == "FOO::SRC/A.RS"));
    assert!(foo_entries.iter().any(|(name, _)| name == "FOO::SRC/B.RS"));
}

#[test]
fn test_generate_classified_relationships() {
    // Chunk in file a.rs defines "caller" and references "helper"
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["helper".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("helper".to_string(), vec![("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string())]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].src_id, "CALLER::SRC/A.RS");
    assert_eq!(rels[0].tgt_id, "HELPER::SRC/B.RS");
    assert_eq!(rels[0].relationship_type, "CALLS");
    assert!((rels[0].weight - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_references_no_self_reference() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("foo", "function", "fn foo()")],
        vec!["foo".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("foo".to_string(), vec![("FOO::SRC/A.RS".to_string(), "FUNCTION".to_string())]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "self-references should be skipped");
}

#[test]
fn test_generate_contains_relationships() {
    // Chunk in a.rs with parent_scope "MyStruct" defines a method "do_thing"
    let chunk = make_chunk(
        "src/a.rs",
        "MyStruct",
        vec![make_symbol("do_thing", "function", "fn do_thing(&self)")],
        vec![],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert(
        "mystruct".to_string(),
        vec![("MYSTRUCT::SRC/A.RS".to_string(), "TYPE".to_string())],
    );

    let rels = generate_contains_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].src_id, "MYSTRUCT::SRC/A.RS");
    assert_eq!(rels[0].tgt_id, "DO_THING::SRC/A.RS");
    assert_eq!(rels[0].keywords, "contains, parent");
    assert_eq!(rels[0].weight, 1.0);
}

// ---------------------------------------------------------------------------
// Task 6: EntityDef parent + visibility
// ---------------------------------------------------------------------------

#[test]
fn test_entity_has_parent_and_visibility() {
    let chunk = make_chunk("src/store.rs", "Store",
        vec![make_symbol("insert_chunk", "function", "pub fn insert_chunk(&self)")], vec![]);
    let entities = generate_entities(&chunk, "canopy");
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].parent, Some("Store".to_string()));
    assert_eq!(entities[0].visibility, Some("pub".to_string()));
}

#[test]
fn test_extract_visibility() {
    assert_eq!(extract_visibility("pub fn foo()"), Some("pub".to_string()));
    assert_eq!(extract_visibility("pub(crate) fn bar()"), Some("pub(crate)".to_string()));
    assert_eq!(extract_visibility("pub(super) fn baz()"), Some("pub(super)".to_string()));
    assert_eq!(extract_visibility("fn private()"), Some("private".to_string()));
    assert_eq!(extract_visibility("export function ts_fn()"), Some("export".to_string()));
}

#[test]
fn test_module_entity_visibility() {
    let e = generate_module_entity("src/lib.rs");
    assert_eq!(e.parent, None);
    assert_eq!(e.visibility, Some("pub".to_string()));
}

// ---------------------------------------------------------------------------
// Task 7: New relationship generators
// ---------------------------------------------------------------------------

#[test]
fn test_generate_type_ref_relationships() {
    let mut chunk = make_chunk("src/a.rs", "",
        vec![make_symbol("open", "function", "pub fn open(path: &Path) -> Store")], vec![]);
    chunk.type_refs = vec![
        RawTypeRef { name: "Path".to_string(), position: TypeRefPosition::Parameter },
        RawTypeRef { name: "Store".to_string(), position: TypeRefPosition::ReturnType },
    ];
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("path".to_string(), vec![("PATH::SRC/PATH.RS".to_string(), "TYPE".to_string())]);
    symbol_map.insert("store".to_string(), vec![("STORE::SRC/STORE.RS".to_string(), "TYPE".to_string())]);
    let rels = generate_type_ref_relationships(&chunk, &symbol_map);
    assert!(rels.iter().any(|r| r.relationship_type == "ACCEPTS" && r.tgt_id == "PATH::SRC/PATH.RS"));
    assert!(rels.iter().any(|r| r.relationship_type == "RETURNS" && r.tgt_id == "STORE::SRC/STORE.RS"));
}

#[test]
fn test_generate_field_of_relationships() {
    let mut chunk = make_chunk("src/store.rs", "",
        vec![make_symbol("Store", "struct", "pub struct Store")], vec![]);
    chunk.field_defs = vec![RawFieldDef { field_name: "db".to_string(), type_name: "Database".to_string() }];
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("database".to_string(), vec![("DATABASE::SRC/DB.RS".to_string(), "TYPE".to_string())]);
    let rels = generate_field_of_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "FIELD_OF");
}

#[test]
fn test_generate_implements_relationships() {
    let chunk = make_chunk("src/store.rs", "",
        vec![make_symbol("Store", "impl", "impl Display for Store")], vec![]);
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("display".to_string(), vec![("DISPLAY::SRC/FMT.RS".to_string(), "TRAIT".to_string())]);
    symbol_map.insert("store".to_string(), vec![("STORE::SRC/STORE.RS".to_string(), "TYPE".to_string())]);
    let rels = generate_implements_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].src_id, "STORE::SRC/STORE.RS");
    assert_eq!(rels[0].tgt_id, "DISPLAY::SRC/FMT.RS");
}

#[test]
fn test_parse_impl_signature_with_generics() {
    assert_eq!(parse_impl_signature("impl<T> Iterator for MyIter<T>"), Some(("Iterator".to_string(), "MyIter".to_string())));
    assert_eq!(parse_impl_signature("impl Display for Store"), Some(("Display".to_string(), "Store".to_string())));
    assert_eq!(parse_impl_signature("impl Store"), None);
}

#[test]
fn test_relationship_type_field_on_defines() {
    let entities = vec![EntityDef {
        entity_name: "FOO::SRC/MAIN.RS".to_string(),
        entity_type: "FUNCTION".to_string(),
        description: String::new(),
        source_id: String::new(),
        metadata: serde_json::Value::Null,
        parent: None,
        visibility: None,
    }];
    let rels = generate_defines_relationships("src/main.rs", &entities);
    assert_eq!(rels[0].relationship_type, "DEFINES");
}

#[test]
fn test_method_kind_produces_entity() {
    assert_eq!(entity_type_for_kind("method"), Some("FUNCTION"));
}

// ---------------------------------------------------------------------------
// Task 7: Improved CALLS resolution with receiver type and same-file preference
// ---------------------------------------------------------------------------

#[test]
fn test_relationship_type_field_on_contains() {
    let chunk = make_chunk(
        "src/a.rs", "MyStruct",
        vec![make_symbol("do_thing", "function", "fn do_thing(&self)")],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("mystruct".to_string(), vec![("MYSTRUCT::SRC/A.RS".to_string(), "TYPE".to_string())]);
    let rels = generate_contains_relationships(&chunk, &symbol_map);
    assert_eq!(rels[0].relationship_type, "CONTAINS");
}

// ---------------------------------------------------------------------------
// Task 11: End-to-end — container entities appear in graph
// ---------------------------------------------------------------------------

#[test]
fn test_container_methods_become_entities() {
    let chunk = Chunk {
        content: String::new(),
        file_path: "src/store.rs".to_string(),
        language: "rust".to_string(),
        node_kinds: vec!["impl_item".to_string()],
        line_start: 1,
        line_end: 50,
        parent_scope: "Store".to_string(),
        defines: vec![
            make_symbol("Store", "impl", "impl Store"),
            make_symbol("open", "function", "pub fn open() -> Self"),
            make_symbol("insert_chunk", "function", "pub fn insert_chunk(&self)"),
        ],
        free_calls: vec![],
        method_calls: vec![],
        other_refs: vec!["insert_chunk".to_string()],
        imports: vec![],
        type_refs: vec![],
        field_defs: vec![],
    };

    let entities = generate_entities(&chunk, "canopy");
    assert_eq!(entities.len(), 2, "should have 2 entities (open, insert_chunk), got: {:?}",
        entities.iter().map(|e| &e.entity_name).collect::<Vec<_>>());
    assert!(entities.iter().any(|e| e.entity_name == "OPEN::SRC/STORE.RS"));
    assert!(entities.iter().any(|e| e.entity_name == "INSERT_CHUNK::SRC/STORE.RS"));
    assert!(entities.iter().all(|e| e.parent == Some("Store".to_string())));

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("store".to_string(), vec![("STORE::SRC/STORE.RS".to_string(), "TYPE".to_string())]);
    symbol_map.insert("insert_chunk".to_string(), vec![("INSERT_CHUNK::SRC/STORE.RS".to_string(), "FUNCTION".to_string())]);

    let refs = generate_classified_relationships(&chunk, &symbol_map);
    assert!(refs.iter().any(|r| r.src_id == "OPEN::SRC/STORE.RS"
        && r.tgt_id == "INSERT_CHUNK::SRC/STORE.RS"
        && r.relationship_type == "CALLS"));

    let contains = generate_contains_relationships(&chunk, &symbol_map);
    assert!(contains.iter().any(|r| r.tgt_id == "OPEN::SRC/STORE.RS" && r.src_id == "STORE::SRC/STORE.RS"));
    assert!(contains.iter().any(|r| r.tgt_id == "INSERT_CHUNK::SRC/STORE.RS" && r.src_id == "STORE::SRC/STORE.RS"));
}

#[test]
fn test_references_unique_function_becomes_calls() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["helper".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("helper".to_string(), vec![("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string())]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "CALLS");
    assert_eq!(rels[0].weight, 1.0);
}

#[test]
fn test_references_unique_type_stays_references() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["Config".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("config".to_string(), vec![("CONFIG::SRC/CFG.RS".to_string(), "TYPE".to_string())]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "REFERENCES");
    assert_eq!(rels[0].weight, 1.0);
}

#[test]
fn test_references_ambiguous_weight_scales() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["run".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("run".to_string(), vec![
        ("RUN::SRC/SVC.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/OTHER.RS".to_string(), "FUNCTION".to_string()),
    ]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| (r.weight - 0.5).abs() < f64::EPSILON));
    assert!(rels.iter().all(|r| r.relationship_type == "CALLS"));
}

#[test]
fn test_references_over_threshold_discarded() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["new".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("new".to_string(), vec![
        ("NEW::SRC/A.RS".to_string(), "FUNCTION".to_string()),
        ("NEW::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("NEW::SRC/C.RS".to_string(), "FUNCTION".to_string()),
        ("NEW::SRC/D.RS".to_string(), "FUNCTION".to_string()),
        ("NEW::SRC/E.RS".to_string(), "FUNCTION".to_string()),
    ]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "references with >4 targets should be discarded");
}

#[test]
fn test_references_weight_at_boundary() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["process".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("process".to_string(), vec![
        ("PROCESS::SRC/A.RS".to_string(), "FUNCTION".to_string()),
        ("PROCESS::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("PROCESS::SRC/C.RS".to_string(), "FUNCTION".to_string()),
        ("PROCESS::SRC/D.RS".to_string(), "FUNCTION".to_string()),
    ]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    // 4 targets, none are self (CALLER is the src) = 4 edges
    assert_eq!(rels.len(), 4);
    assert!(rels.iter().all(|r| (r.weight - 0.25).abs() < f64::EPSILON));
}

#[test]
fn test_references_mixed_kinds() {
    let chunk = make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["Process".to_string()],
    );

    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("caller".to_string(), vec![("CALLER::SRC/A.RS".to_string(), "FUNCTION".to_string())]);
    symbol_map.insert("process".to_string(), vec![
        ("PROCESS::SRC/A.RS".to_string(), "FUNCTION".to_string()),
        ("PROCESS::SRC/B.RS".to_string(), "TYPE".to_string()),
    ]);

    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| (r.weight - 0.5).abs() < f64::EPSILON));
    let calls: Vec<_> = rels.iter().filter(|r| r.relationship_type == "CALLS").collect();
    let refs: Vec<_> = rels.iter().filter(|r| r.relationship_type == "REFERENCES").collect();
    assert_eq!(calls.len(), 1, "FUNCTION target should produce CALLS edge");
    assert_eq!(refs.len(), 1, "TYPE target should produce REFERENCES edge");
}

#[test]
fn test_dedup_relationships_strongest_wins() {
    let rels = vec![
        RelationshipDef {
            src_id: "A".to_string(),
            tgt_id: "B".to_string(),
            relationship_type: "REFERENCES".to_string(),
            keywords: "references, uses".to_string(),
            weight: 1.0,
            description: "A references B".to_string(),
            source_id: "canopy".to_string(),
            ambiguous: false,
        },
        RelationshipDef {
            src_id: "A".to_string(),
            tgt_id: "B".to_string(),
            relationship_type: "CALLS".to_string(),
            keywords: "calls, invokes".to_string(),
            weight: 1.0,
            description: "A calls B".to_string(),
            source_id: "canopy".to_string(),
            ambiguous: false,
        },
        RelationshipDef {
            src_id: "A".to_string(),
            tgt_id: "B".to_string(),
            relationship_type: "ACCEPTS".to_string(),
            keywords: "accepts, parameter".to_string(),
            weight: 1.0,
            description: "A accepts B".to_string(),
            source_id: "canopy".to_string(),
            ambiguous: false,
        },
        RelationshipDef {
            src_id: "A".to_string(),
            tgt_id: "C".to_string(),
            relationship_type: "REFERENCES".to_string(),
            keywords: "references, uses".to_string(),
            weight: 1.0,
            description: "A references C".to_string(),
            source_id: "canopy".to_string(),
            ambiguous: false,
        },
    ];

    let deduped = dedup_relationships(rels);
    assert_eq!(deduped.len(), 2, "should have 2 unique (src,tgt) pairs");

    let ab = deduped.iter().find(|r| r.tgt_id == "B").unwrap();
    assert_eq!(ab.relationship_type, "CALLS", "CALLS should beat ACCEPTS and REFERENCES");

    let ac = deduped.iter().find(|r| r.tgt_id == "C").unwrap();
    assert_eq!(ac.relationship_type, "REFERENCES", "sole edge should survive");
}

#[test]
fn test_build_symbol_map_carries_entity_kind() {
    let chunks_a = vec![make_chunk(
        "src/a.rs",
        "",
        vec![make_symbol("Foo", "struct", "struct Foo")],
        vec![],
    )];
    let chunks_b = vec![make_chunk(
        "src/b.rs",
        "",
        vec![make_symbol("bar", "function", "fn bar()")],
        vec![],
    )];

    let file_chunks = vec![
        ("src/a.rs".to_string(), chunks_a),
        ("src/b.rs".to_string(), chunks_b),
    ];

    let map = build_symbol_map(&file_chunks, &HashMap::new());

    let foo_entries = map.get("foo").expect("should have 'foo' key");
    assert_eq!(foo_entries.len(), 1);
    assert_eq!(foo_entries[0].0, "FOO::SRC/A.RS");
    assert_eq!(foo_entries[0].1, "TYPE");

    let bar_entries = map.get("bar").expect("should have 'bar' key");
    assert_eq!(bar_entries.len(), 1);
    assert_eq!(bar_entries[0].0, "BAR::SRC/B.RS");
    assert_eq!(bar_entries[0].1, "FUNCTION");
}

// ---------------------------------------------------------------------------
// Task 5: Bucket-aware classified relationships
// ---------------------------------------------------------------------------

fn make_classified_chunk(
    file_path: &str,
    defines: Vec<SymbolDef>,
    free_calls: Vec<String>,
    method_calls: Vec<String>,
    other_refs: Vec<String>,
) -> Chunk {
    Chunk {
        content: String::new(),
        file_path: file_path.to_string(),
        language: "rust".to_string(),
        node_kinds: vec![],
        line_start: 1,
        line_end: 10,
        parent_scope: String::new(),
        defines,
        free_calls,
        method_calls,
        other_refs,
        imports: vec![],
        type_refs: vec![],
        field_defs: vec![],
    }
}

#[test]
fn test_free_call_single_candidate_resolves() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["helper".to_string()],
        vec![],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("helper".to_string(), vec![
        ("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "CALLS");
    assert!((rels[0].weight - 1.0).abs() < f64::EPSILON);
    assert!(!rels[0].ambiguous);
}

#[test]
fn test_free_call_same_file_overloads_not_ambiguous() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["helper".to_string()],
        vec![],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("helper".to_string(), vec![
        ("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| !r.ambiguous), "same-file overloads should not be ambiguous");
    assert!(rels.iter().all(|r| (r.weight - 1.0).abs() < f64::EPSILON), "same-file overloads should have weight 1.0");
}

#[test]
fn test_free_call_multi_file_is_ambiguous() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["helper".to_string()],
        vec![],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("helper".to_string(), vec![
        ("HELPER::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("HELPER::SRC/C.RS".to_string(), "FUNCTION".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| r.ambiguous), "multi-file candidates should be ambiguous");
    assert!(rels.iter().all(|r| (r.weight - 0.5).abs() < f64::EPSILON));
}

#[test]
fn test_method_call_single_candidate_resolves() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec![],
        vec!["run".to_string()],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("run".to_string(), vec![
        ("RUN::SRC/B.RS".to_string(), "FUNCTION".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "CALLS");
    assert!(!rels[0].ambiguous);
}

#[test]
fn test_method_call_multi_candidate_dropped() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec![],
        vec!["run".to_string()],
        vec![],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("run".to_string(), vec![
        ("RUN::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/C.RS".to_string(), "FUNCTION".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "method calls with 2+ candidates should be dropped");
}

#[test]
fn test_other_ref_type_becomes_references() {
    let chunk = make_classified_chunk(
        "src/a.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec![],
        vec![],
        vec!["Config".to_string()],
    );
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("config".to_string(), vec![
        ("CONFIG::SRC/CFG.RS".to_string(), "TYPE".to_string()),
    ]);
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].relationship_type, "REFERENCES");
    assert!(!rels[0].ambiguous);
}

#[test]
fn test_over_threshold_dropped_all_buckets() {
    let five_targets = vec![
        ("RUN::SRC/A.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/B.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/C.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/D.RS".to_string(), "FUNCTION".to_string()),
        ("RUN::SRC/E.RS".to_string(), "FUNCTION".to_string()),
    ];
    let mut symbol_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    symbol_map.insert("run".to_string(), five_targets);

    // Free calls bucket
    let chunk = make_classified_chunk(
        "src/x.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec!["run".to_string()],
        vec![],
        vec![],
    );
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "free call with >4 candidates should be dropped");

    // Method calls bucket
    let chunk = make_classified_chunk(
        "src/x.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec![],
        vec!["run".to_string()],
        vec![],
    );
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "method call with >4 candidates should be dropped");

    // Other refs bucket
    let chunk = make_classified_chunk(
        "src/x.rs",
        vec![make_symbol("caller", "function", "fn caller()")],
        vec![],
        vec![],
        vec!["run".to_string()],
    );
    let rels = generate_classified_relationships(&chunk, &symbol_map);
    assert!(rels.is_empty(), "other ref with >4 candidates should be dropped");
}
