use canopy::store::{
    load_vector_index, save_vector_index, ChunkRecord, Store, VectorIndex,
};
use std::collections::HashMap;
use tempfile::TempDir;

fn tmp_store() -> (Store, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("test.redb");
    let store = Store::open(&path).expect("open store");
    (store, dir)
}

fn sample_chunk(file_path: &str) -> ChunkRecord {
    ChunkRecord {
        file_path: file_path.to_string(),
        language: "rust".to_string(),
        node_kinds: vec!["function_item".to_string()],
        line_range: (1, 10),
        parent_scope: String::new(),
        content: "fn hello() {}".to_string(),
    }
}

// ---------------------------------------------------------------------------
// test_store_create_and_open
// ---------------------------------------------------------------------------

#[test]
fn test_store_create_and_open() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("store.redb");

    assert!(!path.exists(), "db should not exist yet");
    {
        let _store = Store::open(&path).expect("create store");
        assert!(path.exists(), "db file should exist after open");
    } // store dropped, lock released

    // Reopen to verify it can be opened a second time
    let _store2 = Store::open(&path).expect("reopen store");
}

// ---------------------------------------------------------------------------
// test_insert_and_get_chunk
// ---------------------------------------------------------------------------

#[test]
fn test_insert_and_get_chunk() {
    let (store, _dir) = tmp_store();
    let rec = sample_chunk("src/main.rs");
    let id = store.insert_chunk(rec, &[0.1, 0.2, 0.3]).expect("insert chunk");
    assert!(id > 0);

    let got = store.get_chunk(id).expect("get chunk").expect("should exist");
    assert_eq!(got.file_path, "src/main.rs");
    assert_eq!(got.language, "rust");
    assert_eq!(got.node_kinds, vec!["function_item"]);
    assert_eq!(got.line_range, (1, 10));
    assert_eq!(got.content, "fn hello() {}");
}

// ---------------------------------------------------------------------------
// test_insert_and_get_entity
// ---------------------------------------------------------------------------

#[test]
fn test_insert_and_get_entity() {
    let (store, _dir) = tmp_store();
    store
        .insert_entity(
            "FOO::SRC/MAIN.RS",
            "FUNCTION",
            "does foo stuff",
            "canopy",
            "src/main.rs",
            serde_json::json!({"line": 1}),
            None,
            None,
        )
        .expect("insert entity");

    let got = store
        .get_entity("FOO::SRC/MAIN.RS")
        .expect("get entity")
        .expect("should exist");
    assert_eq!(got.entity_type, "FUNCTION");
    assert_eq!(got.description, "does foo stuff");
    assert_eq!(got.source_id, "canopy");
    assert_eq!(got.file_path, "src/main.rs");
    assert_eq!(got.metadata_value()["line"], 1);

    // Non-existent entity
    assert!(store.get_entity("DOES::NOT::EXIST").unwrap().is_none());
}

// ---------------------------------------------------------------------------
// test_insert_and_get_relationship
// ---------------------------------------------------------------------------

#[test]
fn test_insert_and_get_relationship() {
    let (store, _dir) = tmp_store();

    store
        .insert_relationship(
            "SRC_ENTITY",
            "TGT_ENTITY",
            "references",
            "uses, calls",
            0.9,
            "SRC_ENTITY references TGT_ENTITY",
            "canopy",
            false,
        )
        .expect("insert relationship");

    // Query from src side
    let rels = store
        .get_relationships_for_entity("SRC_ENTITY")
        .expect("query src");
    assert_eq!(rels.len(), 1);
    assert_eq!(rels[0].src_name, "SRC_ENTITY");
    assert_eq!(rels[0].tgt_name, "TGT_ENTITY");
    assert_eq!(rels[0].relationship_type, "references");
    assert!((rels[0].weight - 0.9).abs() < 1e-9);

    // Query from tgt side — same edge should appear
    let rels_tgt = store
        .get_relationships_for_entity("TGT_ENTITY")
        .expect("query tgt");
    assert_eq!(rels_tgt.len(), 1);
    assert_eq!(rels_tgt[0].src_name, "SRC_ENTITY");
}

// ---------------------------------------------------------------------------
// test_file_index_tracking
// ---------------------------------------------------------------------------

#[test]
fn test_file_index_tracking() {
    let (store, _dir) = tmp_store();
    let file = "src/foo.rs";

    let cid1 = store.insert_chunk(sample_chunk(file), &[0.1, 0.2, 0.3]).unwrap();
    let cid2 = store.insert_chunk(sample_chunk(file), &[0.1, 0.2, 0.3]).unwrap();
    store
        .insert_entity("FOO_ENTITY", "FUNCTION", "desc", "canopy", file, serde_json::json!({}), None, None)
        .unwrap();

    let idx = store.get_file_index(file).unwrap().expect("index exists");
    assert!(idx.chunk_ids.contains(&cid1));
    assert!(idx.chunk_ids.contains(&cid2));
    assert!(idx.entity_names.contains(&"FOO_ENTITY".to_string()));
}

// ---------------------------------------------------------------------------
// test_delete_file_data
// ---------------------------------------------------------------------------

#[test]
fn test_delete_file_data() {
    let (store, _dir) = tmp_store();
    let file = "src/delete_me.rs";

    let cid = store.insert_chunk(sample_chunk(file), &[0.1, 0.2, 0.3]).unwrap();
    store
        .insert_entity("DEL_ENTITY", "FUNCTION", "desc", "canopy", file, serde_json::json!({}), None, None)
        .unwrap();

    // Add a relationship between DEL_ENTITY and an external entity
    store
        .insert_relationship(
            "DEL_ENTITY",
            "OTHER_ENTITY",
            "references",
            "uses",
            1.0,
            "del references other",
            "canopy",
            false,
        )
        .unwrap();

    // Verify data exists
    assert!(store.get_chunk(cid).unwrap().is_some());
    assert!(store.get_entity("DEL_ENTITY").unwrap().is_some());
    assert_eq!(
        store.get_relationships_for_entity("DEL_ENTITY").unwrap().len(),
        1
    );

    store.delete_file_data(file).expect("delete file data");

    // Chunk gone
    assert!(store.get_chunk(cid).unwrap().is_none());
    // Entity gone
    assert!(store.get_entity("DEL_ENTITY").unwrap().is_none());
    // Relationship gone
    assert!(store
        .get_relationships_for_entity("DEL_ENTITY")
        .unwrap()
        .is_empty());
    // OTHER_ENTITY's rel_by_entity index should also be cleaned up
    assert!(store
        .get_relationships_for_entity("OTHER_ENTITY")
        .unwrap()
        .is_empty());
    // File index gone
    assert!(store.get_file_index(file).unwrap().is_none());
}

// ---------------------------------------------------------------------------
// test_meta_get_set
// ---------------------------------------------------------------------------

#[test]
fn test_meta_get_set() {
    let (store, _dir) = tmp_store();

    // Get non-existent key returns None
    assert!(store.get_meta("last_sha").unwrap().is_none());

    store.set_meta("last_sha", "abc123def456").unwrap();
    assert_eq!(
        store.get_meta("last_sha").unwrap().as_deref(),
        Some("abc123def456")
    );

    // Overwrite
    store.set_meta("last_sha", "newsha").unwrap();
    assert_eq!(
        store.get_meta("last_sha").unwrap().as_deref(),
        Some("newsha")
    );
}

// ---------------------------------------------------------------------------
// test_all_embeddings
// ---------------------------------------------------------------------------

#[test]
fn test_all_embeddings() {
    let (store, _dir) = tmp_store();

    let cid1 = store.insert_chunk(sample_chunk("a.rs"), &[0.1, 0.2, 0.3]).unwrap();
    let cid2 = store.insert_chunk(sample_chunk("b.rs"), &[0.1, 0.2, 0.3]).unwrap();

    let chunk_embs = store.all_chunk_embeddings().unwrap();
    assert_eq!(chunk_embs.len(), 2);
    let ids: Vec<u64> = chunk_embs.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&cid1));
    assert!(ids.contains(&cid2));
    for (_, emb) in &chunk_embs {
        assert_eq!(emb, &vec![0.1_f32, 0.2, 0.3]);
    }
}

// ---------------------------------------------------------------------------
// test_clear_all
// ---------------------------------------------------------------------------

#[test]
fn test_clear_all() {
    let (store, _dir) = tmp_store();

    store.insert_chunk(sample_chunk("x.rs"), &[0.1, 0.2, 0.3]).unwrap();
    store
        .insert_entity("E1", "FUNCTION", "desc", "canopy", "x.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_relationship("E1", "E2", "references", "uses", 1.0, "e1 -> e2", "canopy", false)
        .unwrap();
    store.set_meta("foo", "bar").unwrap();

    store.clear_all().expect("clear all");

    let stats = store.stats().unwrap();
    assert_eq!(stats.chunk_count, 0);
    assert_eq!(stats.entity_count, 0);
    assert_eq!(stats.relationship_count, 0);

    // Meta should also be cleared
    assert!(store.get_meta("foo").unwrap().is_none());
}

// ---------------------------------------------------------------------------
// test_get_entities_for_file
// ---------------------------------------------------------------------------

#[test]
fn test_get_entities_for_file() {
    let (store, _dir) = tmp_store();

    store
        .insert_entity("E_A1", "FUNCTION", "in a", "canopy", "a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("E_A2", "STRUCT", "also in a", "canopy", "a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("E_B1", "FUNCTION", "in b", "canopy", "b.rs", serde_json::json!({}), None, None)
        .unwrap();

    let a_entities = store.get_entities_for_file("a.rs").unwrap();
    assert_eq!(a_entities.len(), 2);
    let names: Vec<&str> = a_entities.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"E_A1"));
    assert!(names.contains(&"E_A2"));

    let b_entities = store.get_entities_for_file("b.rs").unwrap();
    assert_eq!(b_entities.len(), 1);
    assert_eq!(b_entities[0].0, "E_B1");

    // File with no entities
    let c_entities = store.get_entities_for_file("c.rs").unwrap();
    assert!(c_entities.is_empty());
}

// ---------------------------------------------------------------------------
// test_stats
// ---------------------------------------------------------------------------

#[test]
fn test_stats() {
    let (store, _dir) = tmp_store();

    let stats = store.stats().unwrap();
    assert_eq!(stats.chunk_count, 0);
    assert_eq!(stats.entity_count, 0);

    store.insert_chunk(sample_chunk("a.rs"), &[0.1, 0.2, 0.3]).unwrap();
    store.insert_chunk(sample_chunk("b.rs"), &[0.1, 0.2, 0.3]).unwrap();
    store
        .insert_entity("E1", "FUNCTION", "desc", "canopy", "a.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_entity("E2", "FUNCTION", "desc", "canopy", "b.rs", serde_json::json!({}), None, None)
        .unwrap();
    store
        .insert_relationship("E1", "E2", "references", "uses", 1.0, "e1 -> e2", "canopy", false)
        .unwrap();

    let stats = store.stats().unwrap();
    assert_eq!(stats.chunk_count, 2);
    assert_eq!(stats.entity_count, 2);
    assert_eq!(stats.relationship_count, 1);
}

// ---------------------------------------------------------------------------
// test_id_monotonic (IDs don't collide after deletion)
// ---------------------------------------------------------------------------

#[test]
fn test_id_monotonic() {
    let (store, _dir) = tmp_store();

    let id1 = store.insert_chunk(sample_chunk("a.rs"), &[0.1, 0.2, 0.3]).unwrap();
    let id2 = store.insert_chunk(sample_chunk("a.rs"), &[0.1, 0.2, 0.3]).unwrap();
    assert_ne!(id1, id2, "IDs must be unique");
    assert!(id2 > id1, "IDs should be monotonically increasing");

    store.delete_file_data("a.rs").unwrap();

    // After deletion, new IDs must still be distinct from previous ones
    let id3 = store.insert_chunk(sample_chunk("a.rs"), &[0.1, 0.2, 0.3]).unwrap();
    assert_ne!(id3, id1);
    assert_ne!(id3, id2);
}

// ---------------------------------------------------------------------------
// test_vector_index_build_and_search
// ---------------------------------------------------------------------------

#[test]
fn test_vector_index_build_and_search() {
    // 3D unit vectors along each axis plus a diagonal
    let points = vec![
        (1u64, vec![1.0f32, 0.0, 0.0]), // closest to query [1,0,0]
        (2u64, vec![0.0f32, 1.0, 0.0]),
        (3u64, vec![0.0f32, 0.0, 1.0]),
        (4u64, vec![0.0f32, 1.0, 1.0]),
    ];

    let index = VectorIndex::build(&points);
    let results = index.search(&[1.0, 0.0, 0.0], 2);

    assert_eq!(results.len(), 2, "should return exactly 2 results");

    // The closest point to [1,0,0] under cosine distance should be ID 1
    assert_eq!(results[0].0, 1u64, "nearest neighbor should be point 1");
    // Distance to itself should be ~0
    assert!(results[0].1 < 1e-5, "distance to identical vector should be ~0");

    // Results must be sorted ascending by distance
    assert!(
        results[0].1 <= results[1].1,
        "results must be ordered by distance ascending"
    );
}

// ---------------------------------------------------------------------------
// test_save_load_chunk_only_vector_index
// ---------------------------------------------------------------------------

#[test]
fn test_save_load_chunk_only_vector_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vectors");

    let points = vec![
        (1u64, vec![1.0f32, 0.0, 0.0]),
        (2u64, vec![0.0, 1.0, 0.0]),
    ];
    let chunk_index = VectorIndex::build(&points);

    save_vector_index(&chunk_index, &path).unwrap();
    let loaded = load_vector_index(&path).unwrap();

    let results = loaded.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
}

// ---------------------------------------------------------------------------
// test_vector_index_save_and_load
// ---------------------------------------------------------------------------

#[test]
fn test_vector_index_save_and_load() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vectors.bin");

    let chunk_points = vec![
        (10u64, vec![1.0f32, 0.0, 0.0]),
        (20u64, vec![0.0f32, 1.0, 0.0]),
        (30u64, vec![0.0f32, 0.0, 1.0]),
    ];

    let chunk_index = VectorIndex::build(&chunk_points);

    save_vector_index(&chunk_index, &path).expect("save");

    let loaded_chunks = load_vector_index(&path).expect("load");

    // Search chunk index
    let results = loaded_chunks.search(&[1.0, 0.0, 0.0], 1);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 10u64, "loaded chunk nearest to [1,0,0] should be id 10");
}

// ---------------------------------------------------------------------------
// test_vector_index_save_atomic
// ---------------------------------------------------------------------------

#[test]
fn test_vector_index_save_atomic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("vectors.bin");
    let tmp_path = path.with_extension("idx.tmp");

    let index = VectorIndex::build(&[(1u64, vec![1.0f32, 0.0])]);

    save_vector_index(&index, &path).expect("save");

    // Temporary file must be gone after atomic rename
    assert!(
        !tmp_path.exists(),
        ".idx.tmp file should not remain after successful save"
    );
    // Final file must exist
    assert!(path.exists(), "index file should exist after save");
}

// ---------------------------------------------------------------------------
// test_vector_index_empty
// ---------------------------------------------------------------------------

#[test]
fn test_vector_index_empty() {
    let index = VectorIndex::build(&[]);
    let results = index.search(&[1.0, 0.0, 0.0], 5);
    assert!(results.is_empty(), "search on empty index should return no results");
}

// ---------------------------------------------------------------------------
// test_vector_index_rebuild_from_store
// ---------------------------------------------------------------------------

#[test]
fn test_vector_index_rebuild_from_store() {
    let (store, _dir) = tmp_store();

    // Insert chunks with distinct embeddings. Use unit vectors for determinism.
    let target_id = store
        .insert_chunk(ChunkRecord {
            file_path: "src/target.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec!["function_item".to_string()],
            line_range: (1, 5),
            parent_scope: String::new(),
            content: "fn target() {}".to_string(),
        }, &[1.0f32, 0.0, 0.0])
        .unwrap();

    store
        .insert_chunk(ChunkRecord {
            file_path: "src/other.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec!["function_item".to_string()],
            line_range: (1, 5),
            parent_scope: String::new(),
            content: "fn other() {}".to_string(),
        }, &[0.0f32, 1.0, 0.0])
        .unwrap();

    store
        .insert_chunk(ChunkRecord {
            file_path: "src/another.rs".to_string(),
            language: "rust".to_string(),
            node_kinds: vec!["function_item".to_string()],
            line_range: (1, 5),
            parent_scope: String::new(),
            content: "fn another() {}".to_string(),
        }, &[0.0f32, 0.0, 1.0])
        .unwrap();

    let embeddings = store.all_chunk_embeddings().unwrap();
    assert_eq!(embeddings.len(), 3);

    let index = VectorIndex::build(&embeddings);
    let results = index.search(&[1.0, 0.0, 0.0], 1);

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].0, target_id,
        "nearest neighbor of [1,0,0] should be the target chunk"
    );
    assert!(results[0].1 < 1e-5, "distance to identical embedding should be ~0");
}

// ---------------------------------------------------------------------------
// test_hub_detection
// ---------------------------------------------------------------------------

#[test]
fn test_hub_detection() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store.redb")).unwrap();

    // Create entities
    store.insert_entity("HUB_NODE", "TYPE", "", "canopy", "a.rs",
        serde_json::json!({}), None, None).unwrap();
    store.insert_entity("LEAF_A", "FUNCTION", "", "canopy", "a.rs",
        serde_json::json!({}), None, None).unwrap();
    store.insert_entity("LEAF_B", "FUNCTION", "", "canopy", "b.rs",
        serde_json::json!({}), None, None).unwrap();
    store.insert_entity("LEAF_C", "FUNCTION", "", "canopy", "c.rs",
        serde_json::json!({}), None, None).unwrap();

    // All leaves reference HUB_NODE
    store.insert_relationship("LEAF_A", "HUB_NODE", "CALLS", "calls", 1.0, "", "canopy", false).unwrap();
    store.insert_relationship("LEAF_B", "HUB_NODE", "CALLS", "calls", 1.0, "", "canopy", false).unwrap();
    store.insert_relationship("LEAF_C", "HUB_NODE", "CALLS", "calls", 1.0, "", "canopy", false).unwrap();

    // With 4 entities and threshold min(4*0.1, 50) = 0.4, in-degree 3 > 0 → hub
    store.detect_hubs().unwrap();

    let hub = store.get_entity("HUB_NODE").unwrap().unwrap();
    assert!(hub.is_hub);
    let leaf = store.get_entity("LEAF_A").unwrap().unwrap();
    assert!(!leaf.is_hub);

    let hub_names = store.get_hub_entity_names().unwrap();
    assert_eq!(hub_names.len(), 1);
    assert_eq!(hub_names[0], "HUB_NODE");

    let all_names = store.all_entity_names().unwrap();
    assert_eq!(all_names.len(), 4);
}

// ---------------------------------------------------------------------------
// test_get_chunk
// ---------------------------------------------------------------------------

#[test]
fn test_get_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();

    let chunk = ChunkRecord {
        file_path: "src/test.rs".to_string(),
        language: "rust".to_string(),
        node_kinds: vec!["function_item".to_string()],
        line_range: (10, 50),
        parent_scope: "".to_string(),
        content: "fn test() {}".to_string(),
    };
    let id = store.insert_chunk(chunk, &[0.1, 0.2, 0.3]).unwrap();

    let loaded = store.get_chunk(id).unwrap().unwrap();
    assert_eq!(loaded.file_path, "src/test.rs");
    assert_eq!(loaded.line_range, (10, 50));
}

// ---------------------------------------------------------------------------
// test_get_cluster_label
// ---------------------------------------------------------------------------

#[test]
fn test_get_cluster_label() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();

    let mut labels = HashMap::new();
    labels.insert(1u32, "graph-traversal".to_string());
    store.store_cluster_meta(&labels).unwrap();

    let label = store.get_cluster_label(1).unwrap();
    assert_eq!(label, Some("graph-traversal".to_string()));

    let missing = store.get_cluster_label(999).unwrap();
    assert_eq!(missing, None);
}

// ---------------------------------------------------------------------------
// test_find_cluster_by_label
// ---------------------------------------------------------------------------

#[test]
fn test_find_cluster_by_label() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();

    let mut labels = HashMap::new();
    labels.insert(1u32, "graph-traversal".to_string());
    labels.insert(2u32, "auth-system".to_string());
    store.store_cluster_meta(&labels).unwrap();

    let found = store.find_cluster_by_label("graph-traversal").unwrap();
    assert_eq!(found, Some((1, "graph-traversal".to_string())));

    let partial = store.find_cluster_by_label("graph").unwrap();
    assert_eq!(partial, Some((1, "graph-traversal".to_string())));

    let missing = store.find_cluster_by_label("nonexistent").unwrap();
    assert_eq!(missing, None);
}

// ---------------------------------------------------------------------------
// test_list_clusters
// ---------------------------------------------------------------------------

#[test]
fn test_list_clusters() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();

    // Insert cluster metadata
    let mut labels = std::collections::HashMap::new();
    labels.insert(1u32, "auth-system".to_string());
    labels.insert(2u32, "graph-traversal".to_string());
    store.store_cluster_meta(&labels).unwrap();

    // Insert cluster memberships
    let mut mapping = std::collections::HashMap::new();
    mapping.insert("ENTITY_A::SRC/A.RS".to_string(), 1u32);
    mapping.insert("ENTITY_B::SRC/B.RS".to_string(), 1u32);
    mapping.insert("ENTITY_C::SRC/C.RS".to_string(), 2u32);
    store.store_clusters(&mapping).unwrap();

    let clusters = store.list_clusters().unwrap();
    assert_eq!(clusters.len(), 2);

    // Sorted by member count descending, then label as tiebreaker
    assert_eq!(clusters[0].1, "auth-system");
    assert_eq!(clusters[0].2, 2); // 2 members
    assert_eq!(clusters[1].1, "graph-traversal");
    assert_eq!(clusters[1].2, 1); // 1 member
}

// ---------------------------------------------------------------------------
// test_list_clusters_sorted_by_member_count_desc
// ---------------------------------------------------------------------------

#[test]
fn test_list_clusters_sorted_by_member_count_desc() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("test.db").as_path()).unwrap();

    let mut labels = std::collections::HashMap::new();
    labels.insert(1u32, "alpha-cluster".to_string());  // alphabetically first
    labels.insert(2u32, "zeta-cluster".to_string());   // alphabetically last
    store.store_cluster_meta(&labels).unwrap();

    let mut mapping = std::collections::HashMap::new();
    // zeta-cluster has more members
    mapping.insert("E1::SRC/A.RS".to_string(), 1u32);
    mapping.insert("E2::SRC/B.RS".to_string(), 2u32);
    mapping.insert("E3::SRC/C.RS".to_string(), 2u32);
    mapping.insert("E4::SRC/D.RS".to_string(), 2u32);
    store.store_clusters(&mapping).unwrap();

    let clusters = store.list_clusters().unwrap();
    assert_eq!(clusters.len(), 2);

    // zeta-cluster should be first despite being alphabetically last (3 members vs 1)
    assert_eq!(clusters[0].1, "zeta-cluster");
    assert_eq!(clusters[0].2, 3);
    assert_eq!(clusters[1].1, "alpha-cluster");
    assert_eq!(clusters[1].2, 1);
}

// ---------------------------------------------------------------------------
// test_get_all_cluster_assignments
// ---------------------------------------------------------------------------

#[test]
fn test_get_all_cluster_assignments() {
    let (store, _dir) = tmp_store();

    let mut mapping = std::collections::HashMap::new();
    mapping.insert("ENTITY_A".to_string(), 0u32);
    mapping.insert("ENTITY_B".to_string(), 0u32);
    mapping.insert("ENTITY_C".to_string(), 1u32);
    store.store_clusters(&mapping).unwrap();

    let result = store.get_all_cluster_assignments().unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result["ENTITY_A"], 0);
    assert_eq!(result["ENTITY_B"], 0);
    assert_eq!(result["ENTITY_C"], 1);
}

#[test]
fn test_get_all_cluster_labels() {
    let (store, _dir) = tmp_store();

    let mut labels = std::collections::HashMap::new();
    labels.insert(0u32, "STORE::SRC/STORE.RS".to_string());
    labels.insert(1u32, "CONFIG::SRC/CONFIG.RS".to_string());
    store.store_cluster_meta(&labels).unwrap();

    let result = store.get_all_cluster_labels().unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[&0], "STORE::SRC/STORE.RS");
    assert_eq!(result[&1], "CONFIG::SRC/CONFIG.RS");
}

// ---------------------------------------------------------------------------
// test_embedding_table_separation
// ---------------------------------------------------------------------------

#[test]
fn test_embedding_table_separation() {
    let (store, _dir) = tmp_store();

    // Insert chunk with known embedding
    let cid = store.insert_chunk(sample_chunk("a.rs"), &[1.0, 2.0, 3.0]).unwrap();

    // Verify record retrieval does NOT contain embeddings (compile-time guarantee)
    let chunk = store.get_chunk(cid).unwrap().unwrap();
    assert_eq!(chunk.content, "fn hello() {}");

    // Verify embeddings are retrievable separately
    let chunk_embs = store.all_chunk_embeddings().unwrap();
    assert_eq!(chunk_embs.len(), 1);
    assert_eq!(chunk_embs[0].0, cid);
    assert_eq!(chunk_embs[0].1, vec![1.0f32, 2.0, 3.0]);

    // Verify deletion cleans up embedding tables too
    store.delete_file_data("a.rs").unwrap();
    assert!(store.all_chunk_embeddings().unwrap().is_empty());
}
