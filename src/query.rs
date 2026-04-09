// Query engine: vector search + graph expansion + graph-enhanced pipeline
use crate::cli::cluster::{ClusterEdge, ClusterResult};
use crate::cli::map::EntityDetail;
use crate::resolve::Candidate;
use crate::store::{ChunkRecord, Store};
use anyhow::Result;
use ordered_float::OrderedFloat;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct QueryResult {
    pub results: Vec<ChunkResult>,
    pub graph_context: GraphContext,
    pub stats: QueryStats,
    // Enrichment (all optional, omitted when not actionable):
    pub coverage: Option<Coverage>,
    pub seed_resolution: Option<SeedResolution>,
}

#[derive(Debug)]
pub struct ChunkResult {
    pub file_path: String,
    pub line_range: (usize, usize),
    pub language: String,
    pub score: f32,
    pub content: String,
}

#[derive(Debug)]
pub struct GraphContext {
    pub entities: Vec<EntityInfo>,
    pub relationships: Vec<RelationshipInfo>,
}

#[derive(Debug)]
pub struct EntityInfo {
    pub name: String,           // Store key (uppercase qualified name)
    pub display_name: String,   // Original-case name from metadata; falls back to `name`
    pub entity_type: String,
    pub description: String,
}

#[derive(Debug)]
pub struct RelationshipInfo {
    pub source: String,
    pub target: String,
    pub relationship_type: String,
    pub keywords: String,
}

#[derive(Debug)]
pub struct QueryStats {
    pub chunks_searched: u64,
    pub query_ms: u64,
}

// ---------------------------------------------------------------------------
// Response enrichment types
// ---------------------------------------------------------------------------

/// Truncation info -- present only when results were capped.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    pub returned: usize,
    pub total: usize,
}

/// Seed resolution outcome -- present only when resolution was non-trivial.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SeedResolution {
    /// Original input (the string the caller provided).
    pub input: Option<String>,
    /// The resolved entity name, if any.
    pub resolved: Option<String>,
    /// Close-match suggestions when resolution found nothing.
    pub suggestions: Vec<String>,
    /// Alternatives when multiple candidates matched and top-ranked was chosen.
    pub alternatives: Vec<String>,
    /// Origin of the resolution: "question" when derived from an NL question.
    pub from: Option<String>,
}

impl SeedResolution {
    /// True if this struct carries no actionable info and should be omitted.
    pub fn is_trivial(&self) -> bool {
        self.input.is_none()
            && self.resolved.is_none()
            && self.suggestions.is_empty()
            && self.alternatives.is_empty()
            && self.from.is_none()
    }
}

// ---------------------------------------------------------------------------
// New pipeline result types
// ---------------------------------------------------------------------------

/// Internal search hit from graph-reranked vector search.
#[derive(Debug, Clone)]
pub struct RankedChunk {
    pub chunk_id: u64,
    pub file_path: String,
    pub line_range: (usize, usize),
    pub entity_keys: Vec<String>,
    pub score: f32,
}

/// Internal path hop from weighted Dijkstra.
#[derive(Debug, Clone)]
pub struct WeightedHop {
    pub from_key: String,
    pub to_key: String,
    pub edge_type: String,
    pub edge_cost: f64,
    pub forward: bool,
}

// ---------------------------------------------------------------------------
// SeedMatch / MatchQuality
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum MatchQuality {
    Exact,
    NameExact,
    Prefix,
    WordBoundary,
    Substring,
}

#[derive(Debug, Clone)]
pub struct SeedMatch {
    pub entity_name: String,
    pub match_quality: MatchQuality,
}

// ---------------------------------------------------------------------------
// EdgeDirection (kept for internal use and backward compat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum EdgeDirection {
    Outward,
    Inward,
}

// ---------------------------------------------------------------------------
// QueryEngine
// ---------------------------------------------------------------------------

pub struct QueryEngine<'a> {
    store: &'a Store,
}

impl<'a> QueryEngine<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    // -----------------------------------------------------------------------
    // Graph expansion (kept -- used by view.rs and others)
    // -----------------------------------------------------------------------

    /// Expand the graph starting from all entities found in the given file paths.
    ///
    /// `hops` controls how many relationship hops to follow. Each hop discovers
    /// neighbor entities reachable via any relationship edge.
    pub fn expand_graph(&self, file_paths: &[&str], hops: usize) -> GraphContext {
        // Seed: collect all entities from the given file paths
        let mut all_entities: HashMap<String, EntityInfo> = HashMap::new();
        let mut all_relationships: HashMap<String, RelationshipInfo> = HashMap::new();

        // Frontier holds entity names to expand in the current hop
        let mut frontier: Vec<String> = Vec::new();

        for file_path in file_paths {
            if let Ok(entities) = self.store.get_entities_for_file(file_path) {
                for (name, record) in entities {
                    if !all_entities.contains_key(&name) {
                        let display_name = display_name_for(&name, &record.metadata);
                        all_entities.insert(
                            name.clone(),
                            EntityInfo {
                                name: name.clone(),
                                display_name,
                                entity_type: record.entity_type,
                                description: record.description,
                            },
                        );
                        frontier.push(name);
                    }
                }
            }
        }

        // BFS for each hop
        for _ in 0..hops {
            if frontier.is_empty() {
                break;
            }

            let current_frontier = std::mem::take(&mut frontier);
            let mut next_frontier: Vec<String> = Vec::new();
            // Track which entities we've already queued for the next frontier
            let mut queued: HashSet<String> = HashSet::new();

            for entity_name in &current_frontier {
                if let Ok(rels) = self.store.get_relationships_for_entity(entity_name) {
                    for rel in rels {
                        // Build relationship key for deduplication
                        let rel_key = format!("{}\0{}", rel.src_name, rel.tgt_name);
                        all_relationships.entry(rel_key).or_insert_with(|| RelationshipInfo {
                            source: rel.src_name.clone(),
                            target: rel.tgt_name.clone(),
                            relationship_type: rel.relationship_type.clone(),
                            keywords: rel.keywords.clone(),
                        });

                        // Discover the neighbor entity (the "other" side of the edge)
                        let neighbor_name = if rel.src_name == *entity_name {
                            rel.tgt_name.clone()
                        } else {
                            rel.src_name.clone()
                        };

                        if !all_entities.contains_key(&neighbor_name)
                            && !queued.contains(&neighbor_name)
                        {
                            // Try to load the neighbor entity record
                            if let Ok(Some(record)) = self.store.get_entity(&neighbor_name) {
                                let display_name =
                                    display_name_for(&neighbor_name, &record.metadata);
                                all_entities.insert(
                                    neighbor_name.clone(),
                                    EntityInfo {
                                        name: neighbor_name.clone(),
                                        display_name,
                                        entity_type: record.entity_type,
                                        description: record.description,
                                    },
                                );
                                next_frontier.push(neighbor_name.clone());
                                queued.insert(neighbor_name);
                            }
                        }
                    }
                }
            }

            frontier = next_frontier;
        }

        GraphContext {
            entities: all_entities.into_values().collect(),
            relationships: all_relationships.into_values().collect(),
        }
    }

    /// Expand the graph starting from specific entity names.
    ///
    /// Unlike `expand_graph` (which seeds from all entities in given files),
    /// this method takes pre-selected entity names as seeds for precision.
    pub fn expand_graph_from_entities(&self, seed_entities: &[&str], hops: usize) -> GraphContext {
        let mut all_entities: HashMap<String, EntityInfo> = HashMap::new();
        let mut all_relationships: HashMap<String, RelationshipInfo> = HashMap::new();
        let mut frontier: Vec<String> = Vec::new();

        // Seed from provided entity names
        for &name in seed_entities {
            if let Ok(Some(record)) = self.store.get_entity(name) {
                let display_name = display_name_for(name, &record.metadata);
                all_entities.insert(
                    name.to_string(),
                    EntityInfo {
                        name: name.to_string(),
                        display_name,
                        entity_type: record.entity_type,
                        description: record.description,
                    },
                );
                frontier.push(name.to_string());
            }
        }

        // BFS for each hop
        for _ in 0..hops {
            if frontier.is_empty() {
                break;
            }

            let current_frontier = std::mem::take(&mut frontier);
            let mut next_frontier: Vec<String> = Vec::new();
            let mut queued: HashSet<String> = HashSet::new();

            for entity_name in &current_frontier {
                if let Ok(rels) = self.store.get_relationships_for_entity(entity_name) {
                    for rel in rels {
                        // Skip DEFINES relationships -- redundant with search results
                        if rel.relationship_type == "DEFINES" {
                            continue;
                        }

                        let rel_key = format!("{}\0{}", rel.src_name, rel.tgt_name);
                        all_relationships.entry(rel_key).or_insert_with(|| RelationshipInfo {
                            source: rel.src_name.clone(),
                            target: rel.tgt_name.clone(),
                            relationship_type: rel.relationship_type.clone(),
                            keywords: rel.keywords.clone(),
                        });

                        let neighbor_name = if rel.src_name == *entity_name {
                            rel.tgt_name.clone()
                        } else {
                            rel.src_name.clone()
                        };

                        if !all_entities.contains_key(&neighbor_name)
                            && !queued.contains(&neighbor_name)
                        {
                            if let Ok(Some(record)) = self.store.get_entity(&neighbor_name) {
                                let display_name =
                                    display_name_for(&neighbor_name, &record.metadata);
                                all_entities.insert(
                                    neighbor_name.clone(),
                                    EntityInfo {
                                        name: neighbor_name.clone(),
                                        display_name,
                                        entity_type: record.entity_type,
                                        description: record.description,
                                    },
                                );
                                next_frontier.push(neighbor_name.clone());
                                queued.insert(neighbor_name);
                            }
                        }
                    }
                }
            }

            frontier = next_frontier;
        }

        GraphContext {
            entities: all_entities.into_values().collect(),
            relationships: all_relationships.into_values().collect(),
        }
    }

    /// Like `expand_graph_from_entities`, but caps the total entity count.
    ///
    /// If BFS produces more entities than `max_entities`, seeds are always kept
    /// and non-seed entities are ranked by the weight of the relationship that
    /// introduced them, keeping only the top entries.
    pub fn expand_graph_from_entities_capped(
        &self,
        seed_entities: &[&str],
        hops: usize,
        max_entities: usize,
    ) -> GraphContext {
        let full = self.expand_graph_from_entities(seed_entities, hops);

        if full.entities.len() <= max_entities {
            return full;
        }

        // Partition into seeds and non-seeds
        let seed_set: HashSet<&str> = seed_entities.iter().copied().collect();
        let mut seeds: Vec<EntityInfo> = Vec::new();
        let mut non_seeds: Vec<EntityInfo> = Vec::new();

        for entity in full.entities {
            if seed_set.contains(entity.name.as_str()) {
                seeds.push(entity);
            } else {
                non_seeds.push(entity);
            }
        }

        // Rank non-seeds by the max relationship weight connecting them.
        let mut entity_weights: HashMap<String, f64> = HashMap::new();
        for rel in &full.relationships {
            if let Ok(rels) = self.store.get_relationships_for_entity(&rel.source) {
                for r in &rels {
                    if r.tgt_name == rel.target && r.relationship_type == rel.relationship_type {
                        let non_seed = if seed_set.contains(rel.source.as_str()) {
                            rel.target.clone()
                        } else if seed_set.contains(rel.target.as_str()) {
                            rel.source.clone()
                        } else {
                            continue;
                        };
                        let entry = entity_weights.entry(non_seed).or_insert(0.0);
                        if r.weight > *entry {
                            *entry = r.weight;
                        }
                    }
                }
            }
        }

        // Sort non-seeds by weight descending
        non_seeds.sort_by(|a, b| {
            let wa = entity_weights.get(&a.name).unwrap_or(&0.0);
            let wb = entity_weights.get(&b.name).unwrap_or(&0.0);
            wb.partial_cmp(wa).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Keep seeds + top non-seeds up to max
        let remaining = max_entities.saturating_sub(seeds.len());
        non_seeds.truncate(remaining);

        let kept_names: HashSet<String> = seeds
            .iter()
            .chain(non_seeds.iter())
            .map(|e| e.name.clone())
            .collect();

        // Filter relationships to only those between kept entities
        let relationships: Vec<RelationshipInfo> = full
            .relationships
            .into_iter()
            .filter(|r| kept_names.contains(&r.source) && kept_names.contains(&r.target))
            .collect();

        let mut entities = seeds;
        entities.extend(non_seeds);

        GraphContext {
            entities,
            relationships,
        }
    }

    // -----------------------------------------------------------------------
    // Seed resolution
    // -----------------------------------------------------------------------

    /// Resolve a seed string to a ranked list of entity matches.
    ///
    /// Resolution order:
    /// 1. Exact match on the provided string.
    /// 2. Uppercase normalization (entity keys are stored uppercase).
    /// 3. Case-insensitive name-exact match (name portion before "::").
    /// 4. Ranked fuzzy match: prefix (3) > word-boundary (2) > substring (1).
    ///
    /// Within each tier, FUNCTION entities sort before TYPE/MODULE.
    pub fn resolve_seed(&self, seed: &str) -> Vec<SeedMatch> {
        // 1. Exact match
        if let Ok(Some(_)) = self.store.get_entity(seed) {
            return vec![SeedMatch {
                entity_name: seed.to_string(),
                match_quality: MatchQuality::Exact,
            }];
        }

        // 2. Uppercase normalization
        let upper = seed.to_uppercase();
        if let Ok(Some(_)) = self.store.get_entity(&upper) {
            return vec![SeedMatch {
                entity_name: upper,
                match_quality: MatchQuality::Exact,
            }];
        }

        // 3 & 4: Use find_entities_by_name for candidate list
        let seed_upper = seed.to_uppercase();
        let entities = match self.store.find_entities_by_name(&seed_upper) {
            Ok(e) => e,
            Err(_) => return vec![],
        };
        if entities.is_empty() {
            return vec![];
        }

        // 3. Case-insensitive name match (name portion before "::")
        let name_exact: Vec<SeedMatch> = entities
            .iter()
            .filter(|e| {
                e.split("::").next().map(|name| name == seed_upper).unwrap_or(false)
            })
            .map(|e| SeedMatch {
                entity_name: e.clone(),
                match_quality: MatchQuality::NameExact,
            })
            .collect();

        if !name_exact.is_empty() {
            let mut sorted = name_exact;
            self.sort_by_entity_type(&mut sorted);
            return sorted;
        }

        // 4. Ranked fuzzy match
        let mut scored: Vec<(SeedMatch, u8)> = entities
            .iter()
            .map(|e| {
                let name_part = e.split("::").next().unwrap_or(e);
                let (quality, score) = if name_part.starts_with(&seed_upper) {
                    (MatchQuality::Prefix, 3)
                } else if is_word_boundary_match(name_part, &seed_upper) {
                    (MatchQuality::WordBoundary, 2)
                } else {
                    (MatchQuality::Substring, 1)
                };
                (SeedMatch { entity_name: e.clone(), match_quality: quality }, score)
            })
            .collect();

        scored.sort_by(|a, b| b.1.cmp(&a.1));

        let mut results: Vec<SeedMatch> = scored.into_iter().map(|(m, _)| m).take(10).collect();
        self.sort_by_entity_type(&mut results);
        results
    }

    fn sort_by_entity_type(&self, matches: &mut [SeedMatch]) {
        matches.sort_by(|a, b| {
            let type_a = self.store.get_entity_type(&a.entity_name).ok().flatten();
            let type_b = self.store.get_entity_type(&b.entity_name).ok().flatten();
            entity_type_rank(&type_a).cmp(&entity_type_rank(&type_b))
        });
    }

    // -----------------------------------------------------------------------
    // entities_to_chunks
    // -----------------------------------------------------------------------

    /// Convert entity names to the code chunks that contain them.
    ///
    /// For each entity, finds the chunk whose line range covers the entity's
    /// line number. Deduplicates by file+range key.
    pub fn entities_to_chunks(&self, entity_names: &[String]) -> Vec<ChunkResult> {
        let mut results: Vec<ChunkResult> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for name in entity_names {
            if let Ok(Some(entity)) = self.store.get_entity(name) {
                let line = entity
                    .metadata_value()
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                if let Ok(Some(file_idx)) = self.store.get_file_index(&entity.file_path) {
                    for &cid in &file_idx.chunk_ids {
                        if let Ok(Some(chunk_rec)) = self.store.get_chunk(cid) {
                            if line >= chunk_rec.line_range.0 && line <= chunk_rec.line_range.1 {
                                let key = format!(
                                    "{}:{}-{}",
                                    chunk_rec.file_path,
                                    chunk_rec.line_range.0,
                                    chunk_rec.line_range.1
                                );
                                if seen.insert(key) {
                                    results.push(ChunkResult {
                                        file_path: chunk_rec.file_path,
                                        line_range: chunk_rec.line_range,
                                        language: chunk_rec.language,
                                        score: 1.0,
                                        content: chunk_rec.content,
                                    });
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
        results
    }

    /// Suggest entity names similar to `seed` when exact resolution fails.
    ///
    /// Ranks candidates by substring + trigram-similarity against the seed.
    /// Returns up to `max` suggestions.
    pub fn suggest_similar_entities(&self, seed: &str, max: usize) -> Vec<String> {
        let needle = seed.to_lowercase();
        let mut scored: Vec<(String, f32)> = Vec::new();

        let all_names = match self.store.all_entity_names() {
            Ok(names) => names,
            Err(_) => return vec![],
        };

        for name in all_names {
            let name_key = name.split("::").next().unwrap_or(&name).to_lowercase();

            // Score: substring match -> 1.0, shared trigram overlap -> 0..1
            let score = if name_key.contains(&needle) || needle.contains(&name_key) {
                1.0
            } else {
                trigram_similarity(&needle, &name_key)
            };

            if score > 0.3 {
                scored.push((name, score));
            }
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(max).map(|(n, _)| n).collect()
    }

    /// Rank cluster members by intra-cluster degree, most-connected first.
    fn rank_cluster_members(&self, members: &[String]) -> Vec<String> {
        let member_set: HashSet<&str> = members.iter().map(|s| s.as_str()).collect();
        let mut ranked: Vec<(String, usize)> = members
            .iter()
            .map(|name| {
                let degree = self
                    .store
                    .get_relationships_for_entity(name)
                    .ok()
                    .map(|rels| {
                        rels.iter()
                            .filter(|r| {
                                member_set.contains(r.src_name.as_str())
                                    && member_set.contains(r.tgt_name.as_str())
                            })
                            .count()
                    })
                    .unwrap_or(0);
                (name.clone(), degree)
            })
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        ranked.into_iter().map(|(n, _)| n).collect()
    }

    // -----------------------------------------------------------------------
    // New pipeline: graph_rerank (Stage 2)
    // -----------------------------------------------------------------------

    /// Three-stage vector search pipeline (Stage 2: Graph Rerank).
    ///
    /// Takes raw vector hits `(chunk_id, similarity)` and `top_k`.
    /// For each candidate chunk, loads it, resolves its entities via line-range
    /// overlap, computes connectivity and cluster bonus scores.
    ///
    /// Final score = 0.6 * vector_similarity + 0.3 * connectivity + 0.1 * cluster_bonus
    ///
    /// Returns `Vec<RankedChunk>` sorted by final score, truncated to top_k.
    pub fn graph_rerank(&self, hits: &[(u64, f32)], top_k: usize, test_penalty: f64) -> Vec<RankedChunk> {
        if hits.is_empty() {
            return Vec::new();
        }

        // Load all candidate chunks with their entities
        let mut candidates: Vec<(u64, ChunkRecord, Vec<String>, f32)> = Vec::new();
        for &(chunk_id, similarity) in hits {
            if let Ok(Some(chunk)) = self.store.get_chunk(chunk_id) {
                let entity_keys = self.entities_overlapping_chunk(&chunk);
                candidates.push((chunk_id, chunk, entity_keys, similarity));
            }
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // Build a set of all entity keys across all candidates for connectivity scoring
        let all_entity_keys: HashSet<&str> = candidates
            .iter()
            .flat_map(|(_, _, keys, _)| keys.iter().map(|k| k.as_str()))
            .collect();

        // Pre-compute: for each entity, which other candidate entities does it connect to?
        let mut entity_connections: HashMap<String, HashSet<String>> = HashMap::new();
        for key in &all_entity_keys {
            if let Ok(rels) = self.store.get_relationships_for_entity(key) {
                let connected: HashSet<String> = rels
                    .iter()
                    .filter_map(|r| {
                        let other = if r.src_name == *key {
                            &r.tgt_name
                        } else {
                            &r.src_name
                        };
                        if all_entity_keys.contains(other.as_str()) && other.as_str() != *key {
                            Some(other.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                entity_connections.insert(key.to_string(), connected);
            }
        }

        // Pre-compute cluster IDs for all entities
        let mut entity_clusters: HashMap<&str, u32> = HashMap::new();
        for key in &all_entity_keys {
            if let Ok(Some(cluster_id)) = self.store.get_entity_cluster(key) {
                entity_clusters.insert(key, cluster_id);
            }
        }

        // Collect all cluster IDs across all candidates for cluster bonus
        let all_cluster_ids: HashSet<u32> = entity_clusters.values().copied().collect();

        // Score each candidate
        let max_connections = candidates.len().max(1) as f32;
        let mut ranked: Vec<RankedChunk> = candidates
            .iter()
            .map(|(chunk_id, chunk, entity_keys, similarity)| {
                // Connectivity: how many of this chunk's entities connect to entities
                // in OTHER candidate chunks?
                let mut connection_count = 0usize;
                for key in entity_keys {
                    if let Some(connected) = entity_connections.get(key) {
                        // Count connections to entities NOT in this chunk
                        for other_key in connected.iter() {
                            if !entity_keys.contains(other_key) {
                                connection_count += 1;
                            }
                        }
                    }
                }
                let connectivity = (connection_count as f32 / max_connections).min(1.0);

                // Cluster bonus: does this chunk share a cluster with other candidates?
                let chunk_clusters: HashSet<u32> = entity_keys
                    .iter()
                    .filter_map(|k| entity_clusters.get(k.as_str()).copied())
                    .collect();
                let cluster_bonus = if chunk_clusters.is_empty() || all_cluster_ids.len() <= 1 {
                    0.0f32
                } else {
                    // Fraction of cluster overlap with the full set
                    let overlap = chunk_clusters.intersection(&all_cluster_ids).count();
                    (overlap as f32 / all_cluster_ids.len() as f32).min(1.0)
                };

                let final_score = 0.6 * similarity + 0.3 * connectivity + 0.1 * cluster_bonus;

                let is_test = is_test_path(&chunk.file_path) || entity_keys.iter().any(|k| {
                    self.store.get_entity(k).ok().flatten()
                        .and_then(|rec| {
                            let meta = rec.metadata_value();
                            meta.get("name").and_then(|n| n.as_str()).map(|n| n.to_string())
                        })
                        .map(|name| is_test_entity_name(&name))
                        .unwrap_or(false)
                });
                let final_score = if is_test && test_penalty > 0.0 {
                    final_score * (1.0 - test_penalty as f32)
                } else {
                    final_score
                };

                RankedChunk {
                    chunk_id: *chunk_id,
                    file_path: chunk.file_path.clone(),
                    line_range: chunk.line_range,
                    entity_keys: entity_keys.clone(),
                    score: final_score,
                }
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(top_k);
        ranked
    }

    // -----------------------------------------------------------------------
    // New pipeline: neighbor_pull_in (Stage 3)
    // -----------------------------------------------------------------------

    /// Stage 3: Takes ranked chunks and expands 1 hop via CALLS edges only
    /// from seed entities. Returns new `Vec<RankedChunk>` for chunks not
    /// already in the hit list.
    pub fn neighbor_pull_in(&self, hits: &[RankedChunk], max_related: usize) -> Vec<RankedChunk> {
        if hits.is_empty() || max_related == 0 {
            return Vec::new();
        }

        // Collect all entity keys and chunk IDs from current hits
        let existing_chunk_ids: HashSet<u64> = hits.iter().map(|h| h.chunk_id).collect();
        let seed_entity_keys: HashSet<&str> = hits
            .iter()
            .flat_map(|h| h.entity_keys.iter().map(|k| k.as_str()))
            .collect();

        let mut pulled: Vec<RankedChunk> = Vec::new();
        let mut seen_chunks: HashSet<u64> = existing_chunk_ids.clone();

        for entity_key in &seed_entity_keys {
            if pulled.len() >= max_related {
                break;
            }

            // Walk 1 hop via CALLS edges only
            if let Ok(rels) = self.store.get_relationships_for_entity(entity_key) {
                for rel in rels {
                    if rel.relationship_type != "CALLS" {
                        continue;
                    }
                    if pulled.len() >= max_related {
                        break;
                    }

                    let neighbor = if rel.src_name == *entity_key {
                        &rel.tgt_name
                    } else {
                        &rel.src_name
                    };

                    // Don't pull in entities already in the hit set
                    if seed_entity_keys.contains(neighbor.as_str()) {
                        continue;
                    }

                    if let Some(ranked) = self.entity_to_ranked_chunk(neighbor) {
                        if seen_chunks.insert(ranked.chunk_id) {
                            pulled.push(ranked);
                        }
                    }
                }
            }
        }

        pulled.truncate(max_related);
        pulled
    }

    // -----------------------------------------------------------------------
    // build_map
    // -----------------------------------------------------------------------

    /// Build an entity detail map for the given entity key.
    ///
    /// Returns `Option<EntityDetail>` with calls/called_by/references/referenced_by
    /// categorized and truncated to `max_per_category`.
    pub fn build_map(&self, entity_key: &str, max_per_category: usize) -> Option<EntityDetail> {
        let rec = self.store.get_entity(entity_key).ok()??;
        let meta = rec.metadata_value();

        let name = meta
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_else(|| entity_key.split("::").next().unwrap_or(entity_key))
            .to_string();
        let kind = meta
            .get("kind")
            .and_then(|k| k.as_str())
            .unwrap_or(&rec.entity_type)
            .to_string();
        let line = meta
            .get("line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;
        let signature = meta
            .get("signature")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();

        let rels = self.store.get_relationships_for_entity(entity_key).ok()?;

        let mut calls: Vec<Candidate> = Vec::new();
        let mut called_by: Vec<Candidate> = Vec::new();
        let mut references: Vec<Candidate> = Vec::new();
        let mut referenced_by: Vec<Candidate> = Vec::new();
        let mut accepts: Vec<Candidate> = Vec::new();
        let mut accepted_by: Vec<Candidate> = Vec::new();
        let mut returns: Vec<Candidate> = Vec::new();
        let mut returned_by: Vec<Candidate> = Vec::new();
        let mut field_of: Vec<Candidate> = Vec::new();
        let mut has_fields: Vec<Candidate> = Vec::new();
        let mut implements: Vec<Candidate> = Vec::new();
        let mut implemented_by: Vec<Candidate> = Vec::new();

        for rel in &rels {
            let is_source = rel.src_name == entity_key;
            let other_key = if is_source { &rel.tgt_name } else { &rel.src_name };

            if let Some(mut candidate) = self.entity_key_to_candidate(other_key) {
                candidate.ambiguous = rel.ambiguous;
                match (rel.relationship_type.as_str(), is_source) {
                    ("CALLS", true) => {
                        if calls.len() < max_per_category {
                            calls.push(candidate);
                        }
                    }
                    ("CALLS", false) => {
                        if called_by.len() < max_per_category {
                            called_by.push(candidate);
                        }
                    }
                    ("REFERENCES", true) => {
                        if references.len() < max_per_category {
                            references.push(candidate);
                        }
                    }
                    ("REFERENCES", false) => {
                        if referenced_by.len() < max_per_category {
                            referenced_by.push(candidate);
                        }
                    }
                    ("ACCEPTS", true) => {
                        if accepts.len() < max_per_category {
                            accepts.push(candidate);
                        }
                    }
                    ("ACCEPTS", false) => {
                        if accepted_by.len() < max_per_category {
                            accepted_by.push(candidate);
                        }
                    }
                    ("RETURNS", true) => {
                        if returns.len() < max_per_category {
                            returns.push(candidate);
                        }
                    }
                    ("RETURNS", false) => {
                        if returned_by.len() < max_per_category {
                            returned_by.push(candidate);
                        }
                    }
                    ("FIELD_OF", true) => {
                        if field_of.len() < max_per_category {
                            field_of.push(candidate);
                        }
                    }
                    ("FIELD_OF", false) => {
                        if has_fields.len() < max_per_category {
                            has_fields.push(candidate);
                        }
                    }
                    ("IMPLEMENTS", true) => {
                        if implements.len() < max_per_category {
                            implements.push(candidate);
                        }
                    }
                    ("IMPLEMENTS", false) => {
                        if implemented_by.len() < max_per_category {
                            implemented_by.push(candidate);
                        }
                    }
                    _ => {}
                }
            }
        }

        // Get cluster label
        let cluster_label = self
            .store
            .get_entity_cluster(entity_key)
            .ok()
            .flatten()
            .and_then(|cid| self.store.get_cluster_label(cid).ok().flatten());

        Some(EntityDetail {
            name,
            kind,
            file_path: rec.file_path,
            line,
            signature,
            cluster_label,
            calls,
            called_by,
            references,
            referenced_by,
            accepts,
            accepted_by,
            returns,
            returned_by,
            field_of,
            has_fields,
            implements,
            implemented_by,
        })
    }

    // -----------------------------------------------------------------------
    // find_weighted_path (weighted Dijkstra)
    // -----------------------------------------------------------------------

    /// Weighted Dijkstra: CALLS edges cost 1.0, all others cost 10.0.
    ///
    /// Returns the sequence of hops from `from` to `to`, or an empty Vec
    /// if no path exists within `max_hops` edges.
    pub fn find_weighted_path(
        &self,
        from: &str,
        to: &str,
        max_hops: usize,
    ) -> Vec<WeightedHop> {
        if from == to {
            return Vec::new();
        }

        // (cost, node, hops_used)
        let mut heap: BinaryHeap<(Reverse<OrderedFloat<f64>>, String, usize)> = BinaryHeap::new();
        // node -> (best_cost, predecessor, edge_type, edge_cost, forward)
        let mut dist: HashMap<String, (f64, Option<String>, String, f64, bool)> = HashMap::new();
        // Track which nodes have been fully processed
        let mut finalized: HashSet<String> = HashSet::new();

        dist.insert(from.to_string(), (0.0, None, String::new(), 0.0, true));
        heap.push((Reverse(OrderedFloat(0.0)), from.to_string(), 0));

        while let Some((Reverse(OrderedFloat(cost)), current, hops)) = heap.pop() {
            // Skip if already finalized
            if finalized.contains(&current) {
                continue;
            }
            // Skip if we've found a better path already
            if let Some((best, _, _, _, _)) = dist.get(&current) {
                if cost > *best {
                    continue;
                }
            }
            finalized.insert(current.clone());

            if current == to {
                break;
            }

            if hops >= max_hops {
                continue;
            }

            if let Ok(rels) = self.store.get_relationships_for_entity(&current) {
                for rel in rels {
                    let forward = rel.src_name == current;
                    let neighbor = if forward {
                        rel.tgt_name.clone()
                    } else {
                        rel.src_name.clone()
                    };

                    if finalized.contains(&neighbor) {
                        continue;
                    }

                    let edge_cost = if rel.relationship_type == "CALLS" {
                        1.0
                    } else {
                        10.0
                    };
                    let new_cost = cost + edge_cost;

                    let is_better = dist
                        .get(&neighbor)
                        .map(|(best, _, _, _, _)| new_cost < *best)
                        .unwrap_or(true);

                    if is_better {
                        dist.insert(
                            neighbor.clone(),
                            (new_cost, Some(current.clone()), rel.relationship_type.clone(), edge_cost, forward),
                        );
                        heap.push((
                            Reverse(OrderedFloat(new_cost)),
                            neighbor,
                            hops + 1,
                        ));
                    }
                }
            }
        }

        // Reconstruct path
        if !dist.contains_key(to) || !finalized.contains(to) {
            return Vec::new();
        }

        let mut path: Vec<WeightedHop> = Vec::new();
        let mut current = to.to_string();
        while current != from {
            let (_, predecessor, edge_type, edge_cost, forward) = match dist.get(&current) {
                Some(v) => v,
                None => break,
            };
            let from_key = match predecessor {
                Some(p) => p.clone(),
                None => break,
            };
            path.push(WeightedHop {
                from_key: from_key.clone(),
                to_key: current.clone(),
                edge_type: edge_type.clone(),
                edge_cost: *edge_cost,
                forward: *forward,
            });
            current = from_key;
        }

        path.reverse();
        path
    }

    // -----------------------------------------------------------------------
    // get_cluster_detail
    // -----------------------------------------------------------------------

    /// Get cluster details for rendering.
    ///
    /// Takes cluster_id, label, max_members, max_relationships.
    /// Returns `ClusterResult` with members, relationships, and truncation info.
    pub fn get_cluster_detail(
        &self,
        cluster_id: u32,
        label: &str,
        max_members: usize,
        max_relationships: usize,
    ) -> Result<ClusterResult> {
        let all_members = self.store.get_cluster_members(cluster_id)?;
        let total_members = all_members.len();

        // Rank members by intra-cluster degree
        let ranked = self.rank_cluster_members(&all_members);
        let kept: Vec<&str> = ranked.iter().take(max_members).map(|s| s.as_str()).collect();

        let mut members: Vec<Candidate> = Vec::new();
        for key in &kept {
            if let Some(candidate) = self.entity_key_to_candidate(key) {
                members.push(candidate);
            }
        }

        // Collect intra-cluster relationships among kept members
        let kept_set: HashSet<&str> = kept.iter().copied().collect();
        let mut relationships: Vec<ClusterEdge> = Vec::new();
        let mut seen_rels: HashSet<(String, String)> = HashSet::new();

        for key in &kept {
            if let Ok(rels) = self.store.get_relationships_for_entity(key) {
                for rel in rels {
                    if kept_set.contains(rel.src_name.as_str())
                        && kept_set.contains(rel.tgt_name.as_str())
                    {
                        let pair = (rel.src_name.clone(), rel.tgt_name.clone());
                        if seen_rels.insert(pair) {
                            relationships.push(ClusterEdge {
                                source: display_name_for(&rel.src_name, ""),
                                target: display_name_for(&rel.tgt_name, ""),
                                edge_type: rel.relationship_type.clone(),
                            });
                        }
                    }
                }
            }
        }

        let total_relationships = relationships.len();
        let truncated_members = if total_members > max_members {
            Some(total_members - max_members)
        } else {
            None
        };
        let truncated_relationships = if total_relationships > max_relationships {
            relationships.truncate(max_relationships);
            Some(total_relationships - max_relationships)
        } else {
            None
        };

        Ok(ClusterResult {
            label: label.to_string(),
            members,
            relationships,
            truncated_members,
            truncated_relationships,
        })
    }

    // -----------------------------------------------------------------------
    // Helper methods (public -- needed by main.rs handlers)
    // -----------------------------------------------------------------------

    /// Convert a single entity key to a Candidate.
    pub fn entity_key_to_candidate(&self, key: &str) -> Option<Candidate> {
        let rec = self.store.get_entity(key).ok()??;
        let meta = rec.metadata_value();
        Some(Candidate {
            name: meta
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_else(|| key.split("::").next().unwrap_or(key))
                .to_string(),
            kind: meta
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or(&rec.entity_type)
                .to_string(),
            file_path: rec.file_path.clone(),
            line: meta
                .get("line")
                .and_then(|l| l.as_u64())
                .unwrap_or(0) as usize,
            ambiguous: false,
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Find entities whose line overlaps a chunk's line range.
    fn entities_overlapping_chunk(&self, chunk: &ChunkRecord) -> Vec<String> {
        let entities = match self.store.get_entities_for_file(&chunk.file_path) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        entities
            .into_iter()
            .filter(|(_, rec)| {
                let line = rec
                    .metadata_value()
                    .get("line")
                    .and_then(|l| l.as_u64())
                    .unwrap_or(0) as usize;
                line >= chunk.line_range.0 && line <= chunk.line_range.1
            })
            .map(|(name, _)| name)
            .collect()
    }

    /// Find the chunk containing an entity and return it as a RankedChunk.
    fn entity_to_ranked_chunk(&self, entity_key: &str) -> Option<RankedChunk> {
        let entity = self.store.get_entity(entity_key).ok()??;
        let line = entity
            .metadata_value()
            .get("line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as usize;

        let file_idx = self.store.get_file_index(&entity.file_path).ok()??;
        for &cid in &file_idx.chunk_ids {
            if let Ok(Some(chunk)) = self.store.get_chunk(cid) {
                if line >= chunk.line_range.0 && line <= chunk.line_range.1 {
                    let entity_keys = self.entities_overlapping_chunk(&chunk);
                    return Some(RankedChunk {
                        chunk_id: cid,
                        file_path: chunk.file_path,
                        line_range: chunk.line_range,
                        entity_keys,
                        score: 0.5, // default score for pulled-in chunks
                    });
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // BFS shortest path (kept for backward compat)
    // -----------------------------------------------------------------------

    /// Find the shortest path between two entities using BFS, walking edges
    /// in both directions.
    pub fn find_path(
        &self,
        from: &str,
        to: &str,
        edge_types: &[&str],
        skip_hubs: bool,
    ) -> Option<GraphContext> {
        let mut visited: HashMap<String, Option<String>> = HashMap::new();
        let mut queue: VecDeque<String> = VecDeque::new();

        visited.insert(from.to_string(), None);
        queue.push_back(from.to_string());

        while let Some(current) = queue.pop_front() {
            if current == to {
                return Some(self.reconstruct_path(from, to, &visited));
            }

            if let Ok(rels) = self.store.get_relationships_for_entity(&current) {
                for rel in rels {
                    if !edge_types.is_empty()
                        && !edge_types
                            .iter()
                            .any(|t| t.eq_ignore_ascii_case(&rel.relationship_type))
                    {
                        continue;
                    }

                    // Walk both directions
                    let neighbor = if rel.src_name == current {
                        rel.tgt_name.clone()
                    } else {
                        rel.src_name.clone()
                    };

                    if visited.contains_key(&neighbor) {
                        continue;
                    }

                    if skip_hubs {
                        if let Ok(Some(rec)) = self.store.get_entity(&neighbor) {
                            if rec.is_hub {
                                continue;
                            }
                        }
                    }

                    visited.insert(neighbor.clone(), Some(current.clone()));
                    queue.push_back(neighbor);
                }
            }
        }

        None
    }

    fn reconstruct_path(
        &self,
        from: &str,
        to: &str,
        visited: &HashMap<String, Option<String>>,
    ) -> GraphContext {
        let mut path_names: Vec<String> = Vec::new();
        let mut current = to.to_string();
        while current != from {
            path_names.push(current.clone());
            current = match visited.get(&current).and_then(|p| p.clone()) {
                Some(parent) => parent,
                None => break,
            };
        }
        path_names.push(from.to_string());
        path_names.reverse();

        let mut entities = Vec::new();
        for name in &path_names {
            if let Ok(Some(rec)) = self.store.get_entity(name) {
                let display_name = display_name_for(name, &rec.metadata);
                entities.push(EntityInfo {
                    name: name.clone(),
                    display_name,
                    entity_type: rec.entity_type,
                    description: rec.description,
                });
            }
        }

        // Collect relationships between consecutive path members
        let mut relationships = Vec::new();
        for pair in path_names.windows(2) {
            if let Ok(rels) = self.store.get_relationships_for_entity(&pair[0]) {
                for rel in rels {
                    if (rel.src_name == pair[0] && rel.tgt_name == pair[1])
                        || (rel.src_name == pair[1] && rel.tgt_name == pair[0])
                    {
                        relationships.push(RelationshipInfo {
                            source: rel.src_name,
                            target: rel.tgt_name,
                            relationship_type: rel.relationship_type,
                            keywords: rel.keywords,
                        });
                        break;
                    }
                }
            }
        }

        GraphContext {
            entities,
            relationships,
        }
    }
}

// ---------------------------------------------------------------------------
// Display name resolution
// ---------------------------------------------------------------------------

/// Extract original-case name from EntityRecord.metadata JSON, falling back
/// to the best-effort extraction from the store key.
pub fn display_name_for(key: &str, metadata_json: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(metadata_json) {
        if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
            return n.to_string();
        }
    }
    // Fallback: take the portion before "::", keep as-is (better than ALL-CAPS).
    key.split("::").next().unwrap_or(key).to_string()
}

// ---------------------------------------------------------------------------
// TOON formatting
// ---------------------------------------------------------------------------

/// Format a single value according to TOON escaping rules.
/// Quote the value if it contains commas, double quotes, colons, brackets,
/// braces, backslashes, or newlines. Escape internal quotes as `\"` and
/// newlines as `\n`.
fn toon_value(s: &str) -> String {
    let needs_quoting = s
        .chars()
        .any(|c| matches!(c, ',' | '"' | ':' | '[' | ']' | '{' | '}' | '\\' | '\n'));

    if needs_quoting {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Seed resolution helpers
// ---------------------------------------------------------------------------

fn is_word_boundary_match(name: &str, query: &str) -> bool {
    if let Some(pos) = name.find(query) {
        if pos == 0 { return true; }
        let prev = name.as_bytes()[pos - 1];
        prev == b'_'
    } else {
        false
    }
}

fn entity_type_rank(entity_type: &Option<String>) -> u8 {
    match entity_type.as_deref() {
        Some("FUNCTION") => 0,
        Some("TYPE") => 1,
        Some("TRAIT") => 2,
        Some("CONSTANT") => 3,
        Some("MODULE") => 4,
        _ => 5,
    }
}

/// Format a QueryResult as TOON (compact, token-efficient data format).
pub fn format_toon(result: &QueryResult) -> String {
    let mut out = String::new();

    // Results header
    out.push_str(&format!(
        "results[{}]{{file_path,line_range,language,score,content}}:\n",
        result.results.len()
    ));

    for chunk in &result.results {
        let line_range = format!("{}-{}", chunk.line_range.0, chunk.line_range.1);
        let score_str = format!("{:.2}", chunk.score);
        out.push_str(&format!(
            "  {},{},{},{},{}\n",
            toon_value(&chunk.file_path),
            toon_value(&line_range),
            toon_value(&chunk.language),
            toon_value(&score_str),
            toon_value(&chunk.content),
        ));
    }

    // Graph context -- omit entirely if both entities and relationships are empty
    let has_entities = !result.graph_context.entities.is_empty();
    let has_relationships = !result.graph_context.relationships.is_empty();

    if has_entities || has_relationships {
        out.push_str("graph_context:\n");

        out.push_str(&format!(
            "  entities[{}]{{name,type,description}}:\n",
            result.graph_context.entities.len()
        ));
        for entity in &result.graph_context.entities {
            out.push_str(&format!(
                "    {},{},{}\n",
                toon_value(&entity.display_name),
                toon_value(&entity.entity_type),
                toon_value(&entity.description),
            ));
        }

        let name_to_display: HashMap<&str, &str> = result
            .graph_context
            .entities
            .iter()
            .map(|e| (e.name.as_str(), e.display_name.as_str()))
            .collect();

        out.push_str(&format!(
            "  relationships[{}]{{source,target,type,keywords}}:\n",
            result.graph_context.relationships.len()
        ));
        for rel in &result.graph_context.relationships {
            let src_disp = name_to_display
                .get(rel.source.as_str())
                .copied()
                .unwrap_or(&rel.source);
            let tgt_disp = name_to_display
                .get(rel.target.as_str())
                .copied()
                .unwrap_or(&rel.target);
            out.push_str(&format!(
                "    {},{},{},{}\n",
                toon_value(src_disp),
                toon_value(tgt_disp),
                toon_value(&rel.relationship_type),
                toon_value(&rel.keywords),
            ));
        }
    }

    // Coverage -- only when truncation occurred
    if let Some(cov) = &result.coverage {
        out.push_str("coverage:\n");
        out.push_str(&format!("  returned: {}\n", cov.returned));
        out.push_str(&format!("  total: {}\n", cov.total));
    }

    // Seed resolution -- only when non-trivial
    if let Some(sr) = &result.seed_resolution {
        out.push_str("seed_resolution:\n");
        if let Some(input) = &sr.input {
            out.push_str(&format!("  input: {}\n", toon_value(input)));
        }
        if let Some(resolved) = &sr.resolved {
            out.push_str(&format!("  resolved: {}\n", toon_value(resolved)));
        }
        if !sr.suggestions.is_empty() {
            out.push_str(&format!(
                "  suggestions[{}]: {}\n",
                sr.suggestions.len(),
                sr.suggestions
                    .iter()
                    .map(|s| toon_value(s))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if !sr.alternatives.is_empty() {
            out.push_str(&format!(
                "  alternatives[{}]: {}\n",
                sr.alternatives.len(),
                sr.alternatives
                    .iter()
                    .map(|s| toon_value(s))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(from) = &sr.from {
            out.push_str(&format!("  from: {}\n", toon_value(from)));
        }
    }

    out
}

/// Check if a file path looks like a test file.
pub fn is_test_file(file_path: &str) -> bool {
    let p = file_path.to_lowercase();
    p.starts_with("tests/")
        || p.starts_with("test/")
        || p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("_test.")
        || p.contains(".test.")
        || p.split('/').next_back().is_some_and(|f| f.starts_with("test_"))
        || p.contains("_spec.")
        || p.contains(".spec.")
        || p.starts_with("spec/")
        || p.contains("/spec/")
}

fn trigram_similarity(a: &str, b: &str) -> f32 {
    fn trigrams(s: &str) -> HashSet<String> {
        let chars: Vec<char> = format!("  {s}  ").chars().collect();
        let mut set = HashSet::new();
        for win in chars.windows(3) {
            set.insert(win.iter().collect::<String>());
        }
        set
    }
    let ta = trigrams(a);
    let tb = trigrams(b);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let intersection = ta.intersection(&tb).count() as f32;
    let union = ta.union(&tb).count() as f32;
    intersection / union
}

fn is_test_path(path: &str) -> bool {
    path.split('/').any(|seg|
        seg == "tests" || seg == "test" || seg == "__tests__" ||
        seg == "testdata" || seg == "test_data" || seg == "fixtures"
    )
}

fn is_test_entity_name(name: &str) -> bool {
    name.starts_with("test_") || name.starts_with("Test")
}
