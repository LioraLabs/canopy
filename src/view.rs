use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        Html,
    },
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

type SseStream = Pin<Box<dyn futures::Stream<Item = Result<Event, Infallible>> + Send>>;
type SseResult = Result<Sse<SseStream>, (StatusCode, String)>;

use crate::config::Config;
use crate::embed::EmbedClient;
use crate::query::{
    format_toon, ChunkResult, QueryEngine, QueryResult,
    QueryStats,
};
use crate::resolve;
use crate::store::{load_vector_index, Store, VectorIndex};
use crate::synthesis::SynthesisClient;

// ---------------------------------------------------------------------------
// Graph data types (serialized to JSON for the frontend)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    pub clusters: HashMap<String, u32>,
    pub cluster_labels: HashMap<u32, String>,
}

#[derive(Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub file: String,
    pub line: i64,
    pub description: String,
}

#[derive(Serialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    #[serde(rename = "type")]
    pub rel_type: String,
    pub weight: f64,
}

// ---------------------------------------------------------------------------
// SSE response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ResultsPayload {
    matches: Vec<String>,
    chunks: Vec<ChunkResponse>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    symbols: Vec<SymbolPayload>,
    stats: StatsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<CoveragePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_resolution: Option<SeedResolutionPayload>,
}

#[derive(Serialize)]
struct SymbolPayload {
    name: String,
    kind: String,
    file_path: String,
    line: usize,
}

#[derive(Serialize)]
struct CoveragePayload {
    returned: usize,
    total: usize,
}

#[derive(Serialize, Default)]
struct SeedResolutionPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    alternatives: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
}

#[derive(Serialize)]
struct ChunkResponse {
    file_path: String,
    line_range: (usize, usize),
    language: String,
    score: f32,
    content: String,
}

#[derive(Serialize)]
struct StatsResponse {
    chunks_searched: u64,
    query_ms: u64,
}

// ---------------------------------------------------------------------------
// Map/trace/cluster response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct MapPayload {
    entity: MapEntityPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_resolution: Option<SeedResolutionPayload>,
}

#[derive(Serialize)]
struct MapEntityPayload {
    name: String,
    kind: String,
    file_path: String,
    line: usize,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cluster_label: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    calls: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    called_by: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    references: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    referenced_by: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accepts: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accepted_by: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    returns: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    returned_by: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    field_of: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    has_fields: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    implements: Vec<CandidatePayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    implemented_by: Vec<CandidatePayload>,
}

#[derive(Serialize)]
struct CandidatePayload {
    name: String,
    kind: String,
    file_path: String,
    line: usize,
}

#[derive(Serialize)]
struct TracePayload {
    hops: Vec<TraceHopPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_resolution: Option<SeedResolutionPayload>,
}

#[derive(Serialize)]
struct TraceHopPayload {
    from: CandidatePayload,
    to: CandidatePayload,
    edge_type: String,
    edge_cost: f64,
    forward: bool,
}

#[derive(Serialize)]
struct ClusterPayload {
    label: String,
    members: Vec<CandidatePayload>,
    relationships: Vec<ClusterEdgePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated_members: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated_relationships: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed_resolution: Option<SeedResolutionPayload>,
}

#[derive(Serialize)]
struct ClusterEdgePayload {
    source: String,
    target: String,
    edge_type: String,
}

#[derive(Serialize)]
struct SymbolSearchResult {
    name: String,
    kind: String,
    file_path: String,
    line: i64,
}

#[derive(Deserialize)]
struct SymbolsQuery {
    q: String,
    #[serde(default = "default_symbol_limit")]
    limit: usize,
}

fn default_symbol_limit() -> usize {
    10
}

// ---------------------------------------------------------------------------
// Graph serialization
// ---------------------------------------------------------------------------

/// Build the full graph as a JSON string in the format 3d-force-graph expects.
///
/// Reads all entities and relationships from the store. Deduplicates
/// relationships (they're indexed from both sides in rel_by_entity).
pub fn build_graph_json(store: &Store) -> Result<String> {
    let entity_names = store.all_entity_names()?;

    let mut nodes = Vec::new();
    for name in &entity_names {
        let record = match store.get_entity(name) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("warn: skipping entity {name}: {e}");
                continue;
            }
        };
        let meta = record.metadata_value();
        let line = meta.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
        let file = record.file_path.clone();

        // Extract human-readable name: first segment before "::"
        let short_name = name.split("::").next().unwrap_or(name).to_string();

        nodes.push(GraphNode {
            id: name.clone(),
            name: short_name,
            entity_type: record.entity_type,
            file,
            line,
            description: record.description,
        });
    }

    // Collect relationships, deduplicating by (src, tgt) pair
    let mut seen_links: HashSet<(String, String)> = HashSet::new();
    let mut links = Vec::new();
    for name in &entity_names {
        let rels = match store.get_relationships_for_entity(name) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warn: skipping relationships for {name}: {e}");
                continue;
            }
        };
        for rel in rels {
            let key = (rel.src_name.clone(), rel.tgt_name.clone());
            if seen_links.contains(&key) {
                continue;
            }
            seen_links.insert(key);
            links.push(GraphLink {
                source: rel.src_name,
                target: rel.tgt_name,
                rel_type: rel.relationship_type,
                weight: rel.weight,
            });
        }
    }

    // Read cluster data from store
    let clusters = store.get_all_cluster_assignments().unwrap_or_default();
    let cluster_labels = store.get_all_cluster_labels().unwrap_or_default();

    let graph = GraphData { nodes, links, clusters, cluster_labels };
    Ok(serde_json::to_string(&graph)?)
}

// ---------------------------------------------------------------------------
// Server state
// ---------------------------------------------------------------------------

struct ViewState {
    store_path: std::path::PathBuf,
    embed_client: EmbedClient,
    chunk_index: VectorIndex,
    config: Config,
    html: String,
    synthesis: Option<Arc<SynthesisClient>>,
}

// ---------------------------------------------------------------------------
// Request/response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct QueryRequest {
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    seed: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    synthesize: bool,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    min_score: Option<f64>,
    #[serde(default)]
    graph_hops: Option<usize>,
    #[serde(default)]
    max_graph_entities: Option<usize>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_index(State(state): State<Arc<ViewState>>) -> Html<String> {
    Html(state.html.clone())
}

async fn handle_query(
    State(state): State<Arc<ViewState>>,
    Json(req): Json<QueryRequest>,
) -> SseResult {
    let strategy = req.strategy.as_deref().unwrap_or("search");

    match strategy {
        "map" => handle_map_query(state, req).await,
        "trace" => handle_trace_query(state, req).await,
        "cluster" => handle_cluster_query(state, req).await,
        _ => handle_search_query(state, req).await,
    }
}

// ---------------------------------------------------------------------------
// Search strategy (vector search + graph expansion)
// ---------------------------------------------------------------------------

async fn handle_search_query(
    state: Arc<ViewState>,
    req: QueryRequest,
) -> SseResult {
    let store = Store::open(&state.store_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let started = Instant::now();
    let question = req.question.clone().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "search strategy requires 'question' parameter".to_string(),
        )
    })?;
    let top_k = req.max_results.unwrap_or(state.config.query.top_k);
    let min_score = req.min_score.unwrap_or(0.3);
    let graph_hops = req.graph_hops.unwrap_or(1);
    let max_graph_entities = req.max_graph_entities.unwrap_or(10);
    let do_synthesize = req.synthesize;

    // 1. Embed the question
    let query_vec = state
        .embed_client
        .embed_one(&question)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // 2. Search chunk index
    let chunk_hits = state.chunk_index.search(&query_vec, top_k);

    // 3. Build chunk results with scores
    let mut scored_chunks: Vec<(u64, f32, ChunkResult)> = Vec::new();
    for (id, dist) in &chunk_hits {
        let score = 1.0 - dist;
        if let Ok(Some(rec)) = store.get_chunk(*id) {
            scored_chunks.push((
                *id,
                score,
                ChunkResult {
                    file_path: rec.file_path,
                    line_range: rec.line_range,
                    language: rec.language,
                    score,
                    content: rec.content,
                },
            ));
        }
    }

    // 4. Threshold filter, sort, truncate
    scored_chunks.retain(|(_id, score, _cr)| *score >= min_score as f32);
    scored_chunks.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored_chunks.truncate(top_k);

    // 5. Seed graph from entities in top result chunks
    let graph_seed_top_n = 3_usize;
    let seed_top_n = graph_seed_top_n.min(scored_chunks.len());
    let mut seed_entity_names: Vec<String> = Vec::new();

    for (_id, _score, chunk) in scored_chunks.iter().take(seed_top_n) {
        if let Ok(entities) = store.get_entities_for_file(&chunk.file_path) {
            for (name, record) in &entities {
                if let Some(line) = record.metadata_value().get("line").and_then(|v| v.as_u64())
                {
                    let line = line as usize;
                    if line >= chunk.line_range.0
                        && line <= chunk.line_range.1
                        && !seed_entity_names.contains(name)
                    {
                        seed_entity_names.push(name.clone());
                    }
                }
            }
        }
    }

    // 6. Expand graph from seed entities
    let seed_refs: Vec<&str> = seed_entity_names.iter().map(|s| s.as_str()).collect();
    let graph_context = QueryEngine::new(&store).expand_graph_from_entities_capped(
        &seed_refs,
        graph_hops,
        max_graph_entities,
    );

    // 7. Build final results
    let results: Vec<ChunkResult> = scored_chunks
        .into_iter()
        .map(|(_id, _score, cr)| cr)
        .collect();

    let db_stats = store
        .stats()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let query_result = QueryResult {
        results,
        graph_context,
        stats: QueryStats {
            chunks_searched: db_stats.chunk_count,
            query_ms: started.elapsed().as_millis() as u64,
        },
        coverage: None,
        seed_resolution: None,
    };

    // Collect entity matches for graph highlighting + build symbols list
    let mut matches: Vec<String> = Vec::new();
    let mut symbols: Vec<SymbolPayload> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let add_entity = |name: &str, store: &Store, seen: &mut HashSet<String>,
                          matches: &mut Vec<String>, symbols: &mut Vec<SymbolPayload>| {
        if !seen.insert(name.to_string()) { return; }
        matches.push(name.to_string());
        if let Ok(Some(rec)) = store.get_entity(name) {
            let meta = rec.metadata_value();
            let line = meta.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            symbols.push(SymbolPayload {
                name: meta.get("name").and_then(|n| n.as_str())
                    .unwrap_or_else(|| name.split("::").next().unwrap_or(name))
                    .to_string(),
                kind: meta.get("kind").and_then(|k| k.as_str())
                    .unwrap_or(&rec.entity_type).to_string(),
                file_path: rec.file_path.clone(),
                line,
            });
        }
    };

    // Add graph context entities
    for e in &query_result.graph_context.entities {
        add_entity(&e.name, &store, &mut seen, &mut matches, &mut symbols);
    }

    // Add entities from hit files
    for c in &query_result.results {
        if let Ok(entities) = store.get_entities_for_file(&c.file_path) {
            for (name, _record) in entities {
                add_entity(&name, &store, &mut seen, &mut matches, &mut symbols);
            }
        }
    }

    // Build the SSE payload
    let payload = ResultsPayload {
        matches,
        chunks: query_result
            .results
            .iter()
            .map(|c| ChunkResponse {
                file_path: c.file_path.clone(),
                line_range: c.line_range,
                language: c.language.clone(),
                score: c.score,
                content: c.content.clone(),
            })
            .collect(),
        symbols,
        stats: StatsResponse {
            chunks_searched: query_result.stats.chunks_searched,
            query_ms: query_result.stats.query_ms,
        },
        coverage: None,
        seed_resolution: None,
    };

    // Prepare synthesis data before moving into the spawned task
    let toon = format_toon(&query_result);
    let synthesis = state.synthesis.clone();

    let (mut tx, rx) = futures::channel::mpsc::channel::<Event>(32);
    tokio::spawn(async move {
        let json = serde_json::to_string(&payload).unwrap_or_default();
        if tx
            .send(Event::default().event("results").data(json))
            .await
            .is_err()
        {
            return;
        }

        // Stream synthesis if requested and configured
        if do_synthesize {
            if let Some(client) = synthesis {
                match client.synthesize_stream(&question, &toon).await {
                    Ok(mut token_rx) => {
                        while let Some(token) = token_rx.next().await {
                            if tx
                                .send(Event::default().event("synthesis").data(token))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[canopy] synthesis stream failed: {e}");
                    }
                }
            }
        }

        let _ = tx.send(Event::default().event("done").data("")).await;
    });

    Ok(Sse::new(Box::pin(rx.map(Ok::<_, Infallible>))))
}

// ---------------------------------------------------------------------------
// Map strategy (symbol detail)
// ---------------------------------------------------------------------------

async fn handle_map_query(
    state: Arc<ViewState>,
    req: QueryRequest,
) -> SseResult {
    let seed = req.seed.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "map strategy requires 'seed' parameter".to_string())
    })?;

    let store = Store::open(&state.store_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let started = Instant::now();
    let max_per_cat = state.config.query.output_caps.map_relationships_per_category;

    let resolver = resolve::FuzzyResolver::new(&store, state.config.query.max_suggestions);
    let resolution = resolver.resolve(&seed);

    let (entity_key, seed_res) = match &resolution {
        resolve::Resolution::Exact(key) => (key.clone(), None),
        resolve::Resolution::Suggestions { input, candidates, .. } => {
            if let Some(top) = candidates.first() {
                (store.find_entities_by_name(&top.name)
                    .ok()
                    .and_then(|names| names.into_iter().next())
                    .unwrap_or_else(|| top.name.clone()),
                Some(SeedResolutionPayload {
                    input: Some(input.clone()),
                    resolved: Some(top.name.clone()),
                    suggestions: vec![],
                    alternatives: candidates.iter().skip(1).map(|c| c.name.clone()).collect(),
                    from: None,
                }))
            } else {
                return Err((StatusCode::NOT_FOUND, format!("No entity found for '{seed}'")));
            }
        }
        resolve::Resolution::NoMatch(_) => {
            return Err((StatusCode::NOT_FOUND, format!("No entity found for '{seed}'")));
        }
    };

    let engine = QueryEngine::new(&store);
    let detail = engine.build_map(&entity_key, max_per_cat).ok_or_else(|| {
        (StatusCode::NOT_FOUND, format!("Entity not found: {entity_key}"))
    })?;

    let query_ms = started.elapsed().as_millis() as u64;

    // Collect all related entity names for graph highlighting
    let mut matches = vec![entity_key.clone()];
    let collect_names = |candidates: &[resolve::Candidate]| -> Vec<String> {
        candidates.iter().map(|c| c.name.clone()).collect()
    };
    for name in [
        collect_names(&detail.calls), collect_names(&detail.called_by),
        collect_names(&detail.references), collect_names(&detail.referenced_by),
    ].into_iter().flatten() {
        if !matches.contains(&name) {
            matches.push(name);
        }
    }

    fn candidates_to_payload(candidates: &[resolve::Candidate]) -> Vec<CandidatePayload> {
        candidates.iter().map(|c| CandidatePayload {
            name: c.name.clone(),
            kind: c.kind.clone(),
            file_path: c.file_path.clone(),
            line: c.line,
        }).collect()
    }

    let payload = MapPayload {
        entity: MapEntityPayload {
            name: detail.name,
            kind: detail.kind,
            file_path: detail.file_path,
            line: detail.line,
            signature: detail.signature,
            cluster_label: detail.cluster_label,
            calls: candidates_to_payload(&detail.calls),
            called_by: candidates_to_payload(&detail.called_by),
            references: candidates_to_payload(&detail.references),
            referenced_by: candidates_to_payload(&detail.referenced_by),
            accepts: candidates_to_payload(&detail.accepts),
            accepted_by: candidates_to_payload(&detail.accepted_by),
            returns: candidates_to_payload(&detail.returns),
            returned_by: candidates_to_payload(&detail.returned_by),
            field_of: candidates_to_payload(&detail.field_of),
            has_fields: candidates_to_payload(&detail.has_fields),
            implements: candidates_to_payload(&detail.implements),
            implemented_by: candidates_to_payload(&detail.implemented_by),
        },
        seed_resolution: seed_res,
    };

    let (mut tx, rx) = futures::channel::mpsc::channel::<Event>(32);
    tokio::spawn(async move {
        let json = serde_json::to_string(&serde_json::json!({
            "matches": matches,
            "stats": { "query_ms": query_ms },
        })).unwrap_or_default();
        let _ = tx.send(Event::default().event("results").data(json)).await;

        let map_json = serde_json::to_string(&payload).unwrap_or_default();
        let _ = tx.send(Event::default().event("map").data(map_json)).await;

        let _ = tx.send(Event::default().event("done").data("")).await;
    });

    Ok(Sse::new(Box::pin(rx.map(Ok::<_, Infallible>))))
}

// ---------------------------------------------------------------------------
// Trace strategy (path between two symbols)
// ---------------------------------------------------------------------------

async fn handle_trace_query(
    state: Arc<ViewState>,
    req: QueryRequest,
) -> SseResult {
    let seed = req.seed.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "trace strategy requires 'seed' parameter".to_string())
    })?;
    let target = req.target.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "trace strategy requires 'target' parameter".to_string())
    })?;

    let store = Store::open(&state.store_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let started = Instant::now();
    let resolver = resolve::FuzzyResolver::new(&store, state.config.query.max_suggestions);

    let from_resolution = resolver.resolve(&seed);
    let to_resolution = resolver.resolve(&target);

    let from_key = match &from_resolution {
        resolve::Resolution::Exact(k) => k.clone(),
        _ => return Err((StatusCode::NOT_FOUND, format!("Cannot resolve seed '{seed}'"))),
    };
    let to_key = match &to_resolution {
        resolve::Resolution::Exact(k) => k.clone(),
        _ => return Err((StatusCode::NOT_FOUND, format!("Cannot resolve target '{target}'"))),
    };

    let engine = QueryEngine::new(&store);
    let weighted_hops = engine.find_weighted_path(&from_key, &to_key, state.config.query.path_max_hops);

    let query_ms = started.elapsed().as_millis() as u64;

    // Collect all entities in the trace path for highlighting
    let mut matches = vec![from_key.clone()];
    if !matches.contains(&to_key) {
        matches.push(to_key.clone());
    }

    let hops: Vec<TraceHopPayload> = weighted_hops
        .iter()
        .map(|wh| {
            if !matches.contains(&wh.from_key) { matches.push(wh.from_key.clone()); }
            if !matches.contains(&wh.to_key) { matches.push(wh.to_key.clone()); }

            let from_c = engine.entity_key_to_candidate(&wh.from_key);
            let to_c = engine.entity_key_to_candidate(&wh.to_key);

            TraceHopPayload {
                from: CandidatePayload {
                    name: from_c.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| wh.from_key.clone()),
                    kind: from_c.as_ref().map(|c| c.kind.clone()).unwrap_or_default(),
                    file_path: from_c.as_ref().map(|c| c.file_path.clone()).unwrap_or_default(),
                    line: from_c.as_ref().map(|c| c.line).unwrap_or(0),
                },
                to: CandidatePayload {
                    name: to_c.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| wh.to_key.clone()),
                    kind: to_c.as_ref().map(|c| c.kind.clone()).unwrap_or_default(),
                    file_path: to_c.as_ref().map(|c| c.file_path.clone()).unwrap_or_default(),
                    line: to_c.as_ref().map(|c| c.line).unwrap_or(0),
                },
                edge_type: wh.edge_type.clone(),
                edge_cost: wh.edge_cost,
                forward: wh.forward,
            }
        })
        .collect();

    let payload = TracePayload {
        hops,
        seed_resolution: None,
    };

    let (mut tx, rx) = futures::channel::mpsc::channel::<Event>(32);
    tokio::spawn(async move {
        let json = serde_json::to_string(&serde_json::json!({
            "matches": matches,
            "stats": { "query_ms": query_ms },
        })).unwrap_or_default();
        let _ = tx.send(Event::default().event("results").data(json)).await;

        let trace_json = serde_json::to_string(&payload).unwrap_or_default();
        let _ = tx.send(Event::default().event("trace").data(trace_json)).await;

        let _ = tx.send(Event::default().event("done").data("")).await;
    });

    Ok(Sse::new(Box::pin(rx.map(Ok::<_, Infallible>))))
}

// ---------------------------------------------------------------------------
// Cluster strategy (cluster detail)
// ---------------------------------------------------------------------------

async fn handle_cluster_query(
    state: Arc<ViewState>,
    req: QueryRequest,
) -> SseResult {
    let seed = req.seed.ok_or_else(|| {
        (StatusCode::BAD_REQUEST, "cluster strategy requires 'seed' parameter".to_string())
    })?;

    let store = Store::open(&state.store_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let started = Instant::now();

    // Try to find cluster by label first, then by entity name
    let (cluster_id, label) = match store.find_cluster_by_label(&seed) {
        Ok(Some((id, label))) => (id, label),
        _ => {
            let resolver = resolve::FuzzyResolver::new(&store, state.config.query.max_suggestions);
            let resolution = resolver.resolve(&seed);
            match &resolution {
                resolve::Resolution::Exact(key) => {
                    let cid = store.get_entity_cluster(key)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Symbol '{seed}' is not in any cluster")))?;
                    let label = store.get_cluster_label(cid)
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                        .unwrap_or_else(|| format!("cluster-{cid}"));
                    (cid, label)
                }
                _ => return Err((StatusCode::NOT_FOUND, format!("Cannot resolve '{seed}'"))),
            }
        }
    };

    let engine = QueryEngine::new(&store);
    let result = engine.get_cluster_detail(
        cluster_id,
        &label,
        state.config.query.output_caps.cluster_members,
        state.config.query.output_caps.cluster_relationships,
    ).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let query_ms = started.elapsed().as_millis() as u64;

    // Collect member entity names for graph highlighting
    let matches: Vec<String> = result.members.iter().map(|m| m.name.clone()).collect();

    let payload = ClusterPayload {
        label: result.label,
        members: result.members.iter().map(|m| CandidatePayload {
            name: m.name.clone(),
            kind: m.kind.clone(),
            file_path: m.file_path.clone(),
            line: m.line,
        }).collect(),
        relationships: result.relationships.iter().map(|r| ClusterEdgePayload {
            source: r.source.clone(),
            target: r.target.clone(),
            edge_type: r.edge_type.clone(),
        }).collect(),
        truncated_members: result.truncated_members,
        truncated_relationships: result.truncated_relationships,
        seed_resolution: None,
    };

    let (mut tx, rx) = futures::channel::mpsc::channel::<Event>(32);
    tokio::spawn(async move {
        let json = serde_json::to_string(&serde_json::json!({
            "matches": matches,
            "stats": { "query_ms": query_ms },
        })).unwrap_or_default();
        let _ = tx.send(Event::default().event("results").data(json)).await;

        let cluster_json = serde_json::to_string(&payload).unwrap_or_default();
        let _ = tx.send(Event::default().event("cluster").data(cluster_json)).await;

        let _ = tx.send(Event::default().event("done").data("")).await;
    });

    Ok(Sse::new(Box::pin(rx.map(Ok::<_, Infallible>))))
}

async fn handle_symbols(
    State(state): State<Arc<ViewState>>,
    axum::extract::Query(params): axum::extract::Query<SymbolsQuery>,
) -> Result<Json<Vec<SymbolSearchResult>>, (StatusCode, String)> {
    if params.q.is_empty() {
        return Ok(Json(vec![]));
    }

    let store = Store::open(&state.store_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let entity_names = store
        .find_entities_by_name(&params.q)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut results: Vec<SymbolSearchResult> = Vec::new();
    for name in entity_names.iter().take(params.limit) {
        if let Ok(Some(record)) = store.get_entity(name) {
            let line = record
                .metadata_value()
                .get("line")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            results.push(SymbolSearchResult {
                name: name.clone(),
                kind: record.entity_type,
                file_path: record.file_path,
                line,
            });
        }
    }

    Ok(Json(results))
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

const VIEW_HTML: &str = include_str!("view.html");

/// Start the view web server.
///
/// Opens the store, builds the graph JSON, injects it into the HTML template,
/// and serves on `127.0.0.1:{port}`.
pub async fn serve(
    repo_root: &Path,
    port: u16,
    config: &Config,
    embed_client: EmbedClient,
) -> Result<()> {
    let store_path = Config::store_path(repo_root);

    // Open store, build graph JSON, then drop store to release the lock
    let graph_json = {
        let store = Store::open(&store_path)?;
        build_graph_json(&store)?
    };
    let synthesis = SynthesisClient::new(&config.query, &config.embedding.url).map(Arc::new);
    let synth_available = synthesis.is_some();
    let html = VIEW_HTML
        .replace("__GRAPH_DATA__", &graph_json)
        .replace("__SYNTHESIS_AVAILABLE__", if synth_available { "true" } else { "false" });
    eprintln!("[canopy] synthesis available: {synth_available}");

    // Load vector indexes for query support
    let vectors_path = Config::vectors_path(repo_root);
    let chunk_index = if vectors_path.exists() {
        load_vector_index(&vectors_path)?
    } else {
        VectorIndex::build(&[])
    };

    let state = Arc::new(ViewState {
        store_path,
        embed_client,
        chunk_index,
        config: config.clone(),
        html,
        synthesis,
    });

    let app = Router::new()
        .route("/", get(handle_index))
        .route("/query", post(handle_query))
        .route("/symbols", get(handle_symbols))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Canopy view running at http://{addr}");
    println!("Press Ctrl+C to stop.");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
