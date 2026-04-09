// Storage layer: redb tables + record types
use anyhow::{Context, Result};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind, new_index};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

fn encode<T: bitcode::Encode>(val: &T) -> Result<Vec<u8>> {
    Ok(bitcode::encode(val))
}

fn decode<T: bitcode::DecodeOwned>(bytes: &[u8]) -> Result<T> {
    bitcode::decode(bytes).map_err(|e| anyhow::anyhow!("bitcode decode: {e}"))
}

// ---------------------------------------------------------------------------
// Table definitions
// ---------------------------------------------------------------------------

const CHUNKS: TableDefinition<u64, &[u8]> = TableDefinition::new("chunks");
const CHUNK_EMBEDDINGS: TableDefinition<u64, &[u8]> = TableDefinition::new("chunk_embeddings");
const ENTITIES: TableDefinition<&str, &[u8]> = TableDefinition::new("entities");
const RELATIONSHIPS: TableDefinition<&str, &[u8]> = TableDefinition::new("relationships");
const REL_BY_ENTITY: TableDefinition<&str, &[u8]> = TableDefinition::new("rel_by_entity");
const FILE_INDEX: TableDefinition<&str, &[u8]> = TableDefinition::new("file_index");
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");
const CLUSTERS: TableDefinition<&str, &[u8]> = TableDefinition::new("clusters");
const CLUSTER_META: TableDefinition<u64, &[u8]> = TableDefinition::new("cluster_meta");

// ---------------------------------------------------------------------------
// Record types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct ChunkRecord {
    pub file_path: String,
    pub language: String,
    pub node_kinds: Vec<String>,
    pub line_range: (usize, usize),
    pub parent_scope: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, bitcode::Encode, bitcode::Decode)]
pub struct EntityRecord {
    pub entity_type: String,
    pub description: String,
    pub source_id: String,
    pub file_path: String,
    pub metadata: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub is_hub: bool,
}

impl EntityRecord {
    /// Parse the JSON metadata string back to a Value for field access.
    pub fn metadata_value(&self) -> serde_json::Value {
        serde_json::from_str(&self.metadata).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, bitcode::Encode, bitcode::Decode)]
pub struct RelRecord {
    pub src_name: String,
    pub tgt_name: String,
    pub relationship_type: String,
    pub keywords: String,
    pub weight: f64,
    pub description: String,
    pub source_id: String,
    #[serde(default)]
    pub ambiguous: bool,
}

#[derive(Debug, Serialize, Deserialize, Default, bitcode::Encode, bitcode::Decode)]
pub struct FileIndex {
    pub chunk_ids: Vec<u64>,
    pub entity_names: Vec<String>,
}

pub struct StoreStats {
    pub chunk_count: u64,
    pub entity_count: u64,
    pub relationship_count: u64,
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

pub struct Store {
    db: Database,
}

impl Store {
    /// Create or open the redb database at `path`, ensuring all tables exist.
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path)
            .with_context(|| format!("failed to open store at {}", path.display()))?;

        // Ensure all tables are created
        let txn = db.begin_write()?;
        txn.open_table(CHUNKS)?;
        txn.open_table(CHUNK_EMBEDDINGS)?;
        txn.open_table(ENTITIES)?;
        txn.open_table(RELATIONSHIPS)?;
        txn.open_table(REL_BY_ENTITY)?;
        txn.open_table(FILE_INDEX)?;
        txn.open_table(META)?;
        txn.open_table(CLUSTERS)?;
        txn.open_table(CLUSTER_META)?;
        txn.commit()?;

        Ok(Self { db })
    }

    // -----------------------------------------------------------------------
    // ID generation
    // -----------------------------------------------------------------------

    fn next_id(
        &self,
        table_name: &str,
        meta: &mut redb::Table<&str, &str>,
    ) -> Result<u64> {
        let counter_key = format!("_next_id_{}", table_name);
        let current: u64 = match meta.get(counter_key.as_str())? {
            Some(v) => v.value().parse().unwrap_or(0),
            None => 0,
        };
        let next = current + 1;
        meta.insert(counter_key.as_str(), next.to_string().as_str())?;
        Ok(next)
    }

    // -----------------------------------------------------------------------
    // Chunks
    // -----------------------------------------------------------------------

    pub fn insert_chunk(&self, record: ChunkRecord, embedding: &[f32]) -> Result<u64> {
        let txn = self.db.begin_write()?;
        let id = {
            let mut meta = txn.open_table(META)?;
            self.next_id("chunks", &mut meta)?
        };
        let file_path = record.file_path.clone();
        let bytes = encode(&record)?;
        {
            let mut chunks = txn.open_table(CHUNKS)?;
            chunks.insert(id, bytes.as_slice())?;
        }
        // Store embedding as raw f32 bytes
        {
            let mut emb_table = txn.open_table(CHUNK_EMBEDDINGS)?;
            let emb_bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
            emb_table.insert(id, emb_bytes.as_slice())?;
        }
        // Update file_index
        {
            let mut file_index = txn.open_table(FILE_INDEX)?;
            let mut idx = load_file_index(&file_index, &file_path)?;
            idx.chunk_ids.push(id);
            let idx_bytes = encode(&idx)?;
            file_index.insert(file_path.as_str(), idx_bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(id)
    }

    pub fn get_chunk(&self, id: u64) -> Result<Option<ChunkRecord>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CHUNKS)?;
        match table.get(id)? {
            Some(v) => Ok(Some(decode(v.value())?)),
            None => Ok(None),
        }
    }

    pub fn all_chunk_embeddings(&self) -> Result<Vec<(u64, Vec<f32>)>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CHUNK_EMBEDDINGS)?;
        let mut result = Vec::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            let bytes = v.value();
            let embedding: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            result.push((k.value(), embedding));
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Entities
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn insert_entity(
        &self,
        name: &str,
        entity_type: &str,
        description: &str,
        source_id: &str,
        file_path: &str,
        metadata: serde_json::Value,
        parent: Option<String>,
        visibility: Option<String>,
    ) -> Result<()> {
        let record = EntityRecord {
            entity_type: entity_type.to_string(),
            description: description.to_string(),
            source_id: source_id.to_string(),
            file_path: file_path.to_string(),
            metadata: metadata.to_string(),
            parent,
            visibility,
            is_hub: false,
        };
        let bytes = encode(&record)?;
        let txn = self.db.begin_write()?;
        {
            let mut entities = txn.open_table(ENTITIES)?;
            entities.insert(name, bytes.as_slice())?;
        }
        // Update file_index
        {
            let mut file_index = txn.open_table(FILE_INDEX)?;
            let mut idx = load_file_index(&file_index, file_path)?;
            if !idx.entity_names.iter().any(|n| n == name) {
                idx.entity_names.push(name.to_string());
            }
            let idx_bytes = encode(&idx)?;
            file_index.insert(file_path, idx_bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_entity(&self, name: &str) -> Result<Option<EntityRecord>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ENTITIES)?;
        match table.get(name)? {
            Some(v) => Ok(Some(decode(v.value())?)),
            None => Ok(None),
        }
    }

    pub fn get_entities_for_file(&self, file_path: &str) -> Result<Vec<(String, EntityRecord)>> {
        let txn = self.db.begin_read()?;
        let file_index_table = txn.open_table(FILE_INDEX)?;
        let idx = load_file_index(&file_index_table, file_path)?;

        let entities_table = txn.open_table(ENTITIES)?;
        let mut result = Vec::new();
        for name in &idx.entity_names {
            if let Some(v) = entities_table.get(name.as_str())? {
                let rec: EntityRecord = decode(v.value())?;
                result.push((name.clone(), rec));
            }
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Relationships
    // -----------------------------------------------------------------------

    /// Relationship key format: "src_name\0tgt_name"
    fn rel_key(src: &str, tgt: &str) -> String {
        format!("{}\0{}", src, tgt)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_relationship(
        &self,
        src: &str,
        tgt: &str,
        rel_type: &str,
        keywords: &str,
        weight: f64,
        description: &str,
        source_id: &str,
        ambiguous: bool,
    ) -> Result<()> {
        let key = Self::rel_key(src, tgt);
        let record = RelRecord {
            src_name: src.to_string(),
            tgt_name: tgt.to_string(),
            relationship_type: rel_type.to_string(),
            keywords: keywords.to_string(),
            weight,
            description: description.to_string(),
            source_id: source_id.to_string(),
            ambiguous,
        };
        let bytes = encode(&record)?;

        let txn = self.db.begin_write()?;
        {
            let mut rels = txn.open_table(RELATIONSHIPS)?;
            rels.insert(key.as_str(), bytes.as_slice())?;
        }
        // Update rel_by_entity for src
        {
            let mut rbe = txn.open_table(REL_BY_ENTITY)?;
            add_rel_key_to_entity(&mut rbe, src, &key)?;
            add_rel_key_to_entity(&mut rbe, tgt, &key)?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_relationships_for_entity(&self, name: &str) -> Result<Vec<RelRecord>> {
        let txn = self.db.begin_read()?;
        let rbe = txn.open_table(REL_BY_ENTITY)?;
        let keys = load_rel_keys(&rbe, name)?;

        let rels_table = txn.open_table(RELATIONSHIPS)?;
        let mut result = Vec::new();
        for key in &keys {
            if let Some(v) = rels_table.get(key.as_str())? {
                let rec: RelRecord = decode(v.value())?;
                result.push(rec);
            }
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // File operations
    // -----------------------------------------------------------------------

    pub fn get_file_index(&self, file_path: &str) -> Result<Option<FileIndex>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(FILE_INDEX)?;
        match table.get(file_path)? {
            Some(v) => Ok(Some(decode(v.value())?)),
            None => Ok(None),
        }
    }

    /// Delete all chunks, entities, and relationships for a file.
    pub fn delete_file_data(&self, file_path: &str) -> Result<()> {
        // Read current file index (read txn first, then write)
        let idx = {
            let txn = self.db.begin_read()?;
            let table = txn.open_table(FILE_INDEX)?;
            match table.get(file_path)? {
                Some(v) => decode::<FileIndex>(v.value())?,
                None => return Ok(()), // nothing to delete
            }
        };

        let txn = self.db.begin_write()?;

        // Delete chunks and their embeddings
        {
            let mut chunks = txn.open_table(CHUNKS)?;
            let mut chunk_embs = txn.open_table(CHUNK_EMBEDDINGS)?;
            for &id in &idx.chunk_ids {
                chunks.remove(id)?;
                chunk_embs.remove(id)?;
            }
        }

        // Delete entities and their relationships
        {
            let mut entities = txn.open_table(ENTITIES)?;
            let mut rels = txn.open_table(RELATIONSHIPS)?;
            let mut rbe = txn.open_table(REL_BY_ENTITY)?;

            for name in &idx.entity_names {
                // Find all relationship keys for this entity
                let rel_keys = load_rel_keys(&rbe, name.as_str())?;

                for key in &rel_keys {
                    // Parse the other entity from the key
                    if let Some((src, tgt)) = parse_rel_key(key) {
                        // Remove this key from the OTHER entity's index
                        let other = if src == name { tgt } else { src };
                        remove_rel_key_from_entity(&mut rbe, other, key)?;
                    }
                    // Delete the relationship record
                    rels.remove(key.as_str())?;
                }

                // Delete this entity's rel_by_entity entry
                rbe.remove(name.as_str())?;

                // Delete the entity itself
                entities.remove(name.as_str())?;
            }
        }

        // Delete the file_index entry
        {
            let mut file_index = txn.open_table(FILE_INDEX)?;
            file_index.remove(file_path)?;
        }

        txn.commit()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Meta
    // -----------------------------------------------------------------------

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut meta = txn.open_table(META)?;
            meta.insert(key, value)?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(META)?;
        match table.get(key)? {
            Some(v) => Ok(Some(v.value().to_string())),
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------------
    // Bulk
    // -----------------------------------------------------------------------

    /// Drop and recreate all tables.
    pub fn clear_all(&self) -> Result<()> {
        let txn = self.db.begin_write()?;
        txn.delete_table(CHUNKS)?;
        txn.delete_table(CHUNK_EMBEDDINGS)?;
        txn.delete_table(ENTITIES)?;
        txn.delete_table(RELATIONSHIPS)?;
        txn.delete_table(REL_BY_ENTITY)?;
        txn.delete_table(FILE_INDEX)?;
        txn.delete_table(META)?;
        txn.delete_table(CLUSTERS)?;
        txn.delete_table(CLUSTER_META)?;
        // Recreate all tables
        txn.open_table(CHUNKS)?;
        txn.open_table(CHUNK_EMBEDDINGS)?;
        txn.open_table(ENTITIES)?;
        txn.open_table(RELATIONSHIPS)?;
        txn.open_table(REL_BY_ENTITY)?;
        txn.open_table(FILE_INDEX)?;
        txn.open_table(META)?;
        txn.open_table(CLUSTERS)?;
        txn.open_table(CLUSTER_META)?;
        txn.commit()?;
        Ok(())
    }

    pub fn stats(&self) -> Result<StoreStats> {
        let txn = self.db.begin_read()?;
        let chunks = txn.open_table(CHUNKS)?;
        let entities = txn.open_table(ENTITIES)?;
        let relationships = txn.open_table(RELATIONSHIPS)?;
        Ok(StoreStats {
            chunk_count: chunks.len()?,
            entity_count: entities.len()?,
            relationship_count: relationships.len()?,
        })
    }

    // -----------------------------------------------------------------------
    // Hub detection
    // -----------------------------------------------------------------------

    /// Compute in-degree per entity from the relationships table, then flag
    /// entities whose in-degree exceeds `min(entity_count * 0.1, 50)` as hubs.
    pub fn detect_hubs(&self) -> Result<()> {
        // Read phase: count in-degree per entity
        let (entity_count, in_degree) = {
            let txn = self.db.begin_read()?;
            let entities_table = txn.open_table(ENTITIES)?;
            let rels_table = txn.open_table(RELATIONSHIPS)?;
            let entity_count = entities_table.len()?;
            let mut in_degree: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();
            for entry in rels_table.iter()? {
                let (_k, v) = entry?;
                let rec: RelRecord = decode(v.value())?;
                *in_degree.entry(rec.tgt_name).or_insert(0) += 1;
            }
            (entity_count, in_degree)
        };

        let threshold = ((entity_count as f64) * 0.1).min(50.0) as u64;

        // Write phase: flag hubs
        let txn = self.db.begin_write()?;
        {
            let mut entities = txn.open_table(ENTITIES)?;
            // Collect hub updates first (to avoid borrow conflict on table)
            let mut hub_updates: Vec<(String, Vec<u8>)> = Vec::new();
            for (name, degree) in &in_degree {
                if *degree > threshold {
                    if let Some(v) = entities.get(name.as_str())? {
                        let mut rec: EntityRecord = decode(v.value())?;
                        rec.is_hub = true;
                        hub_updates.push((name.clone(), encode(&rec)?));
                    }
                }
            }
            for (name, bytes) in &hub_updates {
                entities.insert(name.as_str(), bytes.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Return names of all entities flagged as hubs.
    pub fn get_hub_entity_names(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ENTITIES)?;
        let mut hubs = Vec::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            let rec: EntityRecord = decode(v.value())?;
            if rec.is_hub {
                hubs.push(k.value().to_string());
            }
        }
        Ok(hubs)
    }

    /// Return names of all entities in the store.
    pub fn all_entity_names(&self) -> Result<Vec<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ENTITIES)?;
        let mut names = Vec::new();
        for entry in table.iter()? {
            let (k, _) = entry?;
            names.push(k.value().to_string());
        }
        Ok(names)
    }

    /// Return just the entity_type for a given entity name, or None if not found.
    pub fn get_entity_type(&self, name: &str) -> Result<Option<String>> {
        Ok(self.get_entity(name)?.map(|r| r.entity_type))
    }

    /// Find entity names whose key contains `query` as a substring (case-insensitive).
    ///
    /// Entity keys are stored uppercase, so `query` is uppercased before matching.
    pub fn find_entities_by_name(&self, query: &str) -> Result<Vec<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ENTITIES)?;
        let query_upper = query.to_uppercase();
        let mut results = Vec::new();
        for entry in table.iter()? {
            let (k, _) = entry?;
            if k.value().contains(query_upper.as_str()) {
                results.push(k.value().to_string());
            }
        }
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Clusters
    // -----------------------------------------------------------------------

    pub fn store_clusters(&self, mapping: &std::collections::HashMap<String, u32>) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            // Clear existing clusters
            txn.delete_table(CLUSTERS)?;
            let mut table = txn.open_table(CLUSTERS)?;
            for (entity, cluster_id) in mapping {
                let bytes = cluster_id.to_le_bytes();
                table.insert(entity.as_str(), bytes.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    pub fn store_cluster_meta(&self, labels: &std::collections::HashMap<u32, String>) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            txn.delete_table(CLUSTER_META)?;
            let mut table = txn.open_table(CLUSTER_META)?;
            for (cluster_id, label) in labels {
                let bytes = encode(label)?;
                table.insert(*cluster_id as u64, bytes.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_entity_cluster(&self, entity_name: &str) -> Result<Option<u32>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTERS)?;
        match table.get(entity_name)? {
            Some(v) => {
                let bytes = v.value();
                if bytes.len() >= 4 {
                    Ok(Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Get a cluster's label by cluster ID.
    pub fn get_cluster_label(&self, cluster_id: u32) -> Result<Option<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTER_META)?;
        match table.get(cluster_id as u64)? {
            Some(v) => Ok(Some(decode(v.value())?)),
            None => Ok(None),
        }
    }

    /// Find a cluster by label (case-insensitive substring match).
    /// Returns (cluster_id, full_label) if found.
    pub fn find_cluster_by_label(&self, query: &str) -> Result<Option<(u32, String)>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTER_META)?;
        let query_lower = query.to_lowercase();

        for entry in table.iter()? {
            let (k, v) = entry?;
            let label: String = decode(v.value())?;
            if label.to_lowercase().contains(&query_lower) {
                return Ok(Some((k.value() as u32, label)));
            }
        }

        Ok(None)
    }

    pub fn get_cluster_members(&self, cluster_id: u32) -> Result<Vec<String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTERS)?;
        let mut members = Vec::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            let bytes = v.value();
            if bytes.len() >= 4 {
                let id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                if id == cluster_id {
                    members.push(k.value().to_string());
                }
            }
        }
        Ok(members)
    }

    /// Get all cluster assignments as a map from entity name to cluster ID.
    pub fn get_all_cluster_assignments(&self) -> Result<std::collections::HashMap<String, u32>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTERS)?;
        let mut result = std::collections::HashMap::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            let bytes = v.value();
            if bytes.len() >= 4 {
                let id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                result.insert(k.value().to_string(), id);
            }
        }
        Ok(result)
    }

    /// Get all cluster labels as a map from cluster ID to label string.
    pub fn get_all_cluster_labels(&self) -> Result<std::collections::HashMap<u32, String>> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(CLUSTER_META)?;
        let mut result = std::collections::HashMap::new();
        for entry in table.iter()? {
            let (k, v) = entry?;
            let label: String = decode(v.value())?;
            result.insert(k.value() as u32, label);
        }
        Ok(result)
    }

    /// List all clusters with their labels and member counts.
    /// Returns Vec<(cluster_id, label, member_count)> sorted by label.
    pub fn list_clusters(&self) -> Result<Vec<(u32, String, usize)>> {
        let txn = self.db.begin_read()?;

        // Get all labels
        let meta_table = txn.open_table(CLUSTER_META)?;
        let mut clusters: Vec<(u32, String)> = Vec::new();
        for entry in meta_table.iter()? {
            let (k, v) = entry?;
            let label: String = decode(v.value())?;
            clusters.push((k.value() as u32, label));
        }

        // Count members per cluster
        let cluster_table = txn.open_table(CLUSTERS)?;
        let mut counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for entry in cluster_table.iter()? {
            let (_, v) = entry?;
            let bytes = v.value();
            if bytes.len() >= 4 {
                let id = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                *counts.entry(id).or_insert(0) += 1;
            }
        }

        let mut result: Vec<(u32, String, usize)> = clusters.into_iter()
            .map(|(id, label)| {
                let count = counts.get(&id).copied().unwrap_or(0);
                (id, label, count)
            })
            .collect();

        result.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn load_file_index(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    file_path: &str,
) -> Result<FileIndex> {
    match table.get(file_path)? {
        Some(v) => Ok(decode(v.value())?),
        None => Ok(FileIndex::default()),
    }
}

fn load_rel_keys(
    table: &impl ReadableTable<&'static str, &'static [u8]>,
    entity_name: &str,
) -> Result<Vec<String>> {
    match table.get(entity_name)? {
        Some(v) => Ok(decode(v.value())?),
        None => Ok(Vec::new()),
    }
}

fn add_rel_key_to_entity(
    table: &mut redb::Table<&str, &[u8]>,
    entity_name: &str,
    key: &str,
) -> Result<()> {
    let mut keys: Vec<String> = match table.get(entity_name)? {
        Some(v) => decode(v.value())?,
        None => Vec::new(),
    };
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_string());
    }
    let bytes = encode(&keys)?;
    table.insert(entity_name, bytes.as_slice())?;
    Ok(())
}

fn remove_rel_key_from_entity(
    table: &mut redb::Table<&str, &[u8]>,
    entity_name: &str,
    key: &str,
) -> Result<()> {
    let mut keys: Vec<String> = match table.get(entity_name)? {
        Some(v) => decode(v.value())?,
        None => return Ok(()),
    };
    keys.retain(|k| k != key);
    if keys.is_empty() {
        table.remove(entity_name)?;
    } else {
        let bytes = encode(&keys)?;
        table.insert(entity_name, bytes.as_slice())?;
    }
    Ok(())
}

fn parse_rel_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('\0')
}

// ---------------------------------------------------------------------------
// VectorIndex — usearch HNSW-backed nearest-neighbor index
// ---------------------------------------------------------------------------

/// HNSW vector index with native save/load (no rebuild on load).
///
/// Uses usearch for SIMD-accelerated cosine similarity and instant
/// deserialization from disk.
pub struct VectorIndex {
    index: Index,
    dimensions: usize,
}

impl VectorIndex {
    /// Build from a set of `(id, embedding)` pairs. Returns an empty index
    /// when `points` is empty.
    pub fn build(points: &[(u64, Vec<f32>)]) -> Self {
        let dimensions = points.first().map(|(_, v)| v.len()).unwrap_or(0);

        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };

        let index = new_index(&options).expect("failed to create usearch index");

        if !points.is_empty() {
            index.reserve(points.len()).expect("failed to reserve capacity");
            for (id, vec) in points {
                index.add(*id, vec).expect("failed to add vector");
            }
        }

        Self { index, dimensions }
    }

    /// Search for the `k` nearest neighbors to `query`.
    /// Returns `Vec<(id, distance)>` sorted by distance ascending.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        if self.index.size() == 0 {
            return Vec::new();
        }

        let results = match self.index.search(query, k) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        results
            .keys
            .into_iter()
            .zip(results.distances)
            .collect()
    }

    /// Save the index to a file.
    pub fn save(&self, path: &str) -> Result<()> {
        self.index
            .save(path)
            .map_err(|e| anyhow::anyhow!("failed to save index: {e}"))?;
        Ok(())
    }

    /// Load an index from a file.
    pub fn load(path: &str, dimensions: usize) -> Result<Self> {
        let options = IndexOptions {
            dimensions,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };

        let index = new_index(&options).map_err(|e| anyhow::anyhow!("failed to create index: {e}"))?;
        index
            .load(path)
            .map_err(|e| anyhow::anyhow!("failed to load index from {path}: {e}"))?;

        Ok(Self { index, dimensions })
    }
}

// ---------------------------------------------------------------------------
// Save / load for chunk HNSW index
// ---------------------------------------------------------------------------
//
// One usearch file: path.chunks.usearch
// Plus a small header file at `path` with dimensions.

/// Save chunk HNSW index.
///
/// Writes two files:
/// - `path` — header with dimensions
/// - `path.chunks.usearch` — chunk HNSW index
pub fn save_vector_index(
    chunk_index: &VectorIndex,
    path: &Path,
) -> Result<()> {
    let dims = chunk_index.dimensions;
    let chunk_path = format!("{}.chunks.usearch", path.display());
    chunk_index.save(&chunk_path)?;

    let header = format!("{dims}");
    fs::write(path, header.as_bytes())
        .with_context(|| format!("failed to write index header {}", path.display()))?;

    Ok(())
}

/// Load chunk HNSW index.
pub fn load_vector_index(path: &Path) -> Result<VectorIndex> {
    let header = fs::read_to_string(path)
        .with_context(|| format!("failed to read index header {}", path.display()))?;
    let dims: usize = header.trim().parse()
        .with_context(|| "failed to parse dimensions from index header")?;

    let chunk_path = format!("{}.chunks.usearch", path.display());

    let chunk_index = if Path::new(&chunk_path).exists() {
        VectorIndex::load(&chunk_path, dims)?
    } else {
        VectorIndex::build(&[])
    };

    Ok(chunk_index)
}
