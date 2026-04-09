use anyhow::{Context, Result};
use canopy::chunker::{self, Chunk};
use canopy::cli;
use canopy::config::Config;
use canopy::embed::{EmbedClient, EmbedProvider};
use canopy::git;
use canopy::graph;
use canopy::query;
use canopy::resolve;
use canopy::store::{self, ChunkRecord, Store, VectorIndex};
use canopy::synthesis::SynthesisClient;
use clap::{Parser, Subcommand};
use globset::Glob;
use ignore::WalkBuilder;
use indicatif::{HumanDuration, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

#[derive(Parser)]
#[command(
    name = "canopy",
    about = "Semantic code search powered by tree-sitter"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize canopy for a git repository
    Init,
    /// Index changes since last indexed commit
    Index,
    /// Full re-index of the entire repository
    Reindex,
    /// Show project status
    Status,
    /// Remove all canopy data and git hooks
    Clean,
    /// Start web UI to visualize the knowledge graph
    View {
        /// Port to serve on
        #[arg(long, default_value = "8080")]
        port: u16,
    },
    /// Semantic search — find relevant code locations
    Search {
        /// The question to search for
        question: String,
        /// Maximum results to return
        #[arg(long)]
        max_results: Option<usize>,
        /// Run through synthesis model for a natural language answer
        #[arg(long)]
        synthesize: bool,
        /// Filter results to files matching this glob pattern
        #[arg(long)]
        path: Option<String>,
    },
    /// Show full graph detail for a symbol
    Map {
        /// Symbol name to look up
        symbol: String,
    },
    /// Trace execution path between two symbols
    Trace {
        /// Starting symbol
        from: String,
        /// Target symbol
        to: String,
    },
    /// Show subsystem overview for a cluster
    Cluster {
        /// Cluster label or symbol name
        identifier: String,
        /// Filter displayed members to files matching this path glob
        #[arg(long)]
        path: Option<String>,
    },
    /// List all clusters
    Clusters {
        /// Page number (1-indexed)
        #[arg(long, default_value = "1")]
        page: usize,
        /// Filter to clusters with members matching this path glob
        #[arg(long)]
        path: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => cmd_init().await,
        Commands::Index => cmd_index().await,
        Commands::Reindex => cmd_reindex().await,
        Commands::Status => cmd_status().await,
        Commands::Clean => cmd_clean().await,
        Commands::View { port } => cmd_view(port).await,
        Commands::Search { question, max_results, synthesize, path } => {
            cmd_search(&question, max_results, synthesize, path.as_deref()).await
        }
        Commands::Map { symbol } => cmd_map(&symbol),
        Commands::Trace { from, to } => cmd_trace(&from, &to),
        Commands::Cluster { identifier, path } => cmd_cluster(&identifier, path.as_deref()),
        Commands::Clusters { page, path } => cmd_clusters(page, path.as_deref()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_api_key(config: &Config) -> Option<String> {
    config
        .embedding
        .api_key_env
        .as_ref()
        .and_then(|env_var| std::env::var(env_var).ok())
}

fn make_embed_client(config: &Config) -> EmbedClient {
    let provider = EmbedProvider::from_config(&config.embedding, resolve_api_key(config));
    EmbedClient::new(provider)
}

fn build_path_filter(pattern: Option<&str>) -> Option<globset::GlobMatcher> {
    pattern.map(|p| {
        Glob::new(p)
            .unwrap_or_else(|_| Glob::new(&format!("**/{p}/**")).expect("fallback glob"))
            .compile_matcher()
    })
}

fn path_matches(file_path: &str, filter: &Option<globset::GlobMatcher>) -> bool {
    match filter {
        Some(matcher) => matcher.is_match(file_path),
        None => true,
    }
}

fn ensure_gitignore(repo_root: &Path) -> Result<()> {
    let gitignore = repo_root.join(".gitignore");
    let content = std::fs::read_to_string(&gitignore).unwrap_or_default();
    if content.lines().any(|l| l.trim() == ".canopy/" || l.trim() == ".canopy") {
        return Ok(());
    }
    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(".canopy/\n");
    std::fs::write(&gitignore, new_content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_init
// ---------------------------------------------------------------------------

async fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    println!("Git root: {}", repo_root.display());

    // Check for legacy config at repo root
    let legacy_config = repo_root.join(".canopy.toml");
    if legacy_config.exists() {
        eprintln!("Found legacy .canopy.toml at repo root.");
        eprintln!("Run `canopy clean` first, then `canopy init` to upgrade.");
        std::process::exit(1);
    }

    let canopy_dir = Config::canopy_dir(&repo_root);
    if canopy_dir.exists() {
        anyhow::bail!(".canopy/ already exists. Use `canopy reindex` to re-index.");
    }

    let project_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    println!("Initializing canopy for '{project_name}'...");

    // Create .canopy/ directory
    std::fs::create_dir_all(&canopy_dir)?;

    // Create default config
    let config = Config::default_for(&project_name);

    // Save config
    let config_path = Config::config_path(&repo_root);
    config.save(&config_path)?;
    println!("Created .canopy/canopy.toml");

    // Install git hooks
    git::install_hooks(&repo_root)?;
    println!("Installed git hooks (post-commit, post-merge)");

    // Add .canopy/ to .gitignore
    ensure_gitignore(&repo_root)?;

    println!();
    println!("Ready! Run `canopy reindex` to build the index.");
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_index (incremental)
// ---------------------------------------------------------------------------

async fn cmd_index() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let mut config = Config::load(&config_path)?;
    let current_sha = git::head_sha(&repo_root)?;

    if config.indexing.last_sha == current_sha {
        println!("Already up to date ({})", &current_sha[..8]);
        return Ok(());
    }

    if config.indexing.last_sha.is_empty() {
        println!("No previous index. Running full index...");
        let store = Store::open(&Config::store_path(&repo_root))?;
        let embed = make_embed_client(&config);
        do_full_index(&repo_root, &store, &embed, &mut config).await?;
        config.save(&config_path)?;
        return Ok(());
    }

    let changes = git::diff_files(&repo_root, &config.indexing.last_sha, &current_sha)?;
    if changes.is_empty() {
        println!("No file changes detected.");
        config.indexing.last_sha = current_sha.clone();
        config.save(&config_path)?;
        return Ok(());
    }

    println!(
        "Indexing changes from {}..{}",
        &config.indexing.last_sha[..8],
        &current_sha[..8]
    );

    let embed = make_embed_client(&config);

    // Verify embedding dimensions are consistent
    let probed_dims = embed.probe_dimensions().await?;
    if let Some(expected) = config.embedding.dimensions {
        if probed_dims != expected {
            anyhow::bail!(
                "Embedding dimension mismatch: config says {expected}, model returns {probed_dims}. \
                 Did you change the embedding model? Run `canopy reindex` to rebuild."
            );
        }
    }

    let store = Store::open(&Config::store_path(&repo_root))?;

    // Phase 1: Chunk changed files
    let mut files_to_index: Vec<(String, Vec<Chunk>)> = Vec::new();
    let mut deleted_files: Vec<String> = Vec::new();

    for change in &changes {
        match change {
            git::FileChange::Deleted(path) => {
                if chunker::detect_language(path).is_some() {
                    deleted_files.push(path.clone());
                }
            }
            git::FileChange::Added(path) | git::FileChange::Modified(path) => {
                if chunker::detect_language(path).is_none() {
                    continue;
                }

                // Delete old data for this file
                store.delete_file_data(path)?;

                let prefix = match change {
                    git::FileChange::Added(_) => "A",
                    _ => "M",
                };

                match chunk_file(&repo_root, path, &config) {
                    Ok(chunks) => {
                        println!("  {prefix} {path} ({} chunks)", chunks.len());
                        files_to_index.push((path.clone(), chunks));
                    }
                    Err(e) => eprintln!("  E {path}: {e}"),
                }
            }
        }
    }

    // Handle deletions
    for path in &deleted_files {
        store.delete_file_data(path)?;
        println!("  D {path}");
    }

    // Phase 2: Embed and store chunks
    let chunk_count = embed_and_store_chunks(&files_to_index, &store, &embed).await?;

    // Phase 4: Store entities
    let entity_count = store_entities(&files_to_index, &store)?;

    // Phase 5: Store relationships
    let rel_count = store_relationships(&files_to_index, &store)?;

    // Phase 6: Rebuild HNSW indexes from all embeddings in redb
    rebuild_vector_indexes(&store, &repo_root)?;

    // Track cluster drift
    let changed_file_count = files_to_index.len() + deleted_files.len();
    let current_drift = store.get_meta("files_changed_since_cluster")?
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let new_drift = current_drift + changed_file_count;
    store.set_meta("files_changed_since_cluster", &new_drift.to_string())?;

    // Recompute clusters if drift > 20%
    let total_files = store.get_meta("total_indexed_files")?
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    if total_files > 0 && new_drift * 5 > total_files {
        println!("  Recomputing clusters (drift threshold exceeded)...");
        store.detect_hubs()?;
        let hub_names = store.get_hub_entity_names()?;
        let (cluster_entities, affinities) = canopy::cluster::build_affinity_graph(&store, &hub_names)?;
        let mut clusters = canopy::cluster::louvain(&cluster_entities, &affinities, 1.5);
        let hub_assignments = canopy::cluster::attach_hubs_to_clusters(&store, &hub_names, &clusters)?;
        clusters.extend(hub_assignments);
        canopy::cluster::absorb_small_clusters(&store, &mut clusters, 3)?;
        let cluster_labels = canopy::cluster::compute_cluster_labels(&clusters, &store)?;
        store.store_clusters(&clusters)?;
        store.store_cluster_meta(&cluster_labels)?;
        store.set_meta("files_changed_since_cluster", "0")?;
    }

    config.indexing.last_sha = current_sha;
    config.save(&config_path)?;

    println!(
        "Done: {chunk_count} chunks, {entity_count} entities, {rel_count} relationships, {} deleted",
        deleted_files.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_reindex
// ---------------------------------------------------------------------------

async fn cmd_reindex() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let mut config = Config::load(&config_path)?;
    let store = Store::open(&Config::store_path(&repo_root))?;

    println!("Clearing existing data...");
    store.clear_all()?;

    // Delete stale vector index file if present
    let vectors_path = Config::vectors_path(&repo_root);
    if vectors_path.exists() {
        let _ = std::fs::remove_file(&vectors_path);
    }

    println!("Starting full re-index...");
    let embed = make_embed_client(&config);

    // Probe and save dimensions if not set (e.g., after model change)
    let dims = embed.probe_dimensions().await
        .context("Failed to probe embedding dimensions. Is the embedding server running?")?;
    if let Some(expected) = config.embedding.dimensions {
        if dims != expected {
            println!("Embedding dimensions changed ({expected} -> {dims}), updating config.");
        }
    }
    config.embedding.dimensions = Some(dims);
    println!("Embedding model: {} ({dims} dimensions)", config.embedding.model);

    do_full_index(&repo_root, &store, &embed, &mut config).await?;
    config.save(&config_path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_status
// ---------------------------------------------------------------------------

async fn cmd_status() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;

    println!("Canopy project: {}", config.project.name);
    println!(
        "Embedding: {} ({})",
        config.embedding.model,
        config.embedding.dimensions.map_or("unknown".to_string(), |d| format!("{d}d"))
    );

    if config.indexing.last_sha.is_empty() {
        println!("Last indexed: never");
    } else {
        println!("Last indexed: {}", &config.indexing.last_sha[..8]);
    }

    let current_sha = git::head_sha(&repo_root).unwrap_or_default();
    if !current_sha.is_empty() {
        if current_sha == config.indexing.last_sha {
            println!("HEAD: {} (up to date)", &current_sha[..8]);
        } else {
            println!("HEAD: {} (needs indexing)", &current_sha[..8]);
        }
    }

    let store_path = Config::store_path(&repo_root);
    if store_path.exists() {
        let store = Store::open(&store_path)?;
        let stats = store.stats()?;
        println!("Chunks:        {}", stats.chunk_count);
        println!("Entities:      {}", stats.entity_count);
        println!("Relationships: {}", stats.relationship_count);

        if let Ok(meta) = std::fs::metadata(&store_path) {
            let size_mb = meta.len() as f64 / (1024.0 * 1024.0);
            println!("Store size:    {size_mb:.1} MB");
        }
    } else {
        println!("Store: not created yet");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_clean
// ---------------------------------------------------------------------------

async fn cmd_clean() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;

    // Uninstall git hooks
    git::uninstall_hooks(&repo_root)?;
    println!("Removed git hooks");

    // Delete .canopy/ directory
    let canopy_dir = Config::canopy_dir(&repo_root);
    if canopy_dir.exists() {
        std::fs::remove_dir_all(&canopy_dir)?;
        println!("Removed .canopy/");
    }

    // Delete legacy .canopy.toml if it exists
    let legacy = repo_root.join(".canopy.toml");
    if legacy.exists() {
        std::fs::remove_file(&legacy)?;
        println!("Removed legacy .canopy.toml");
    }

    println!("Clean complete. Run `canopy init` to start fresh.");
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_view
// ---------------------------------------------------------------------------

async fn cmd_view(port: u16) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;
    let embed_client = make_embed_client(&config);

    canopy::view::serve(&repo_root, port, &config, embed_client).await
}

// ---------------------------------------------------------------------------
// cmd_search
// ---------------------------------------------------------------------------

async fn cmd_search(question: &str, max_results: Option<usize>, synthesize: bool, path_filter: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;
    let vectors_path = Config::vectors_path(&repo_root);

    if !vectors_path.exists() {
        anyhow::bail!("No vector index found. Run `canopy index` first.");
    }

    let store_inst = Store::open(&Config::store_path(&repo_root))?;
    let embed_client = make_embed_client(&config);
    let chunk_index = store::load_vector_index(&vectors_path)?;

    let top_k = max_results.unwrap_or(config.query.top_k);
    let query_vec = embed_client.embed_one(question).await?;
    let filter = build_path_filter(path_filter);

    // Stage 1: Vector retrieval (extra headroom when filtering by path)
    let retrieval_k = if filter.is_some() { top_k * 10 } else { top_k * 3 };
    let raw_hits = chunk_index.search(&query_vec, retrieval_k);

    // Stage 2: Graph rerank (fetch extra to allow for path filtering)
    let rerank_k = if filter.is_some() { top_k * 5 } else { top_k };
    let engine = query::QueryEngine::new(&store_inst);
    let ranked_all = engine.graph_rerank(&raw_hits, rerank_k, config.query.test_penalty);

    // Stage 2b: Apply path filter, then take top_k
    let ranked: Vec<_> = ranked_all
        .into_iter()
        .filter(|rc| path_matches(&rc.file_path, &filter))
        .take(top_k)
        .collect();

    // Stage 3: Neighbor pull-in (also filtered)
    let related_all = engine.neighbor_pull_in(&ranked, config.query.output_caps.related_neighbors);
    let related_ranked: Vec<_> = related_all
        .into_iter()
        .filter(|rc| path_matches(&rc.file_path, &filter))
        .collect();

    // Convert RankedChunks to SearchHits
    let symbols_cap = config.query.output_caps.symbols_per_chunk;
    let to_hit = |rc: query::RankedChunk| -> cli::search::SearchHit {
        let mut symbols: Vec<_> = rc
            .entity_keys
            .iter()
            .filter_map(|k| engine.entity_key_to_candidate(k))
            .collect();
        symbols.truncate(symbols_cap);
        cli::search::SearchHit {
            file_path: rc.file_path,
            line_range: rc.line_range,
            symbols,
        }
    };

    let hits: Vec<_> = ranked.into_iter().map(&to_hit).collect();
    let related: Vec<_> = related_ranked.into_iter().map(to_hit).collect();

    let total_available = raw_hits.len();
    let truncated = if total_available > hits.len() {
        Some(total_available - hits.len())
    } else {
        None
    };

    let result = cli::search::SearchResult {
        hits,
        related,
        truncated,
    };

    if synthesize {
        let synth = SynthesisClient::new(&config.query, &config.embedding.url)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "synthesis not configured — set synthesis_provider in canopy.toml"
                )
            })?;
        // Build context from chunk content for synthesis
        let mut context = String::new();
        for hit in &result.hits {
            if let Ok(Some(fi)) = store_inst.get_file_index(&hit.file_path) {
                for cid in &fi.chunk_ids {
                    if let Ok(Some(chunk)) = store_inst.get_chunk(*cid) {
                        if chunk.line_range == hit.line_range {
                            context.push_str(&chunk.content);
                            context.push('\n');
                        }
                    }
                }
            }
        }
        let answer = synth.synthesize(question, &context).await?;
        println!("{answer}");
    } else {
        print!("{}", cli::search::format_search_result(&result));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_map
// ---------------------------------------------------------------------------

fn cmd_map(symbol: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;
    let store_inst = Store::open(&Config::store_path(&repo_root))?;

    let resolver = resolve::FuzzyResolver::new(&store_inst, config.query.max_suggestions);
    let resolution = resolver.resolve(symbol);

    match &resolution {
        resolve::Resolution::Exact(key) => {
            let engine = query::QueryEngine::new(&store_inst);
            let max_per_cat = config.query.output_caps.map_relationships_per_category;
            match engine.build_map(key, max_per_cat) {
                Some(detail) => {
                    let result = cli::map::MapResult {
                        entity: detail,
                        resolution,
                    };
                    print!("{}", cli::map::format_map_result(&result));
                }
                None => {
                    eprintln!("Entity not found: {}", key);
                    std::process::exit(2);
                }
            }
        }
        _ => {
            print!("{}", resolve::format_resolution(&resolution));
            std::process::exit(1);
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_trace
// ---------------------------------------------------------------------------

fn cmd_trace(from: &str, to: &str) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;
    let store_inst = Store::open(&Config::store_path(&repo_root))?;

    let resolver = resolve::FuzzyResolver::new(&store_inst, config.query.max_suggestions);

    let resolution_from = resolver.resolve(from);
    let resolution_to = resolver.resolve(to);

    let from_key = match &resolution_from {
        resolve::Resolution::Exact(k) => k.clone(),
        _ => {
            print!("from: {}", resolve::format_resolution(&resolution_from));
            std::process::exit(1);
        }
    };

    let to_key = match &resolution_to {
        resolve::Resolution::Exact(k) => k.clone(),
        _ => {
            print!("to: {}", resolve::format_resolution(&resolution_to));
            std::process::exit(1);
        }
    };

    let engine = query::QueryEngine::new(&store_inst);
    let weighted_hops = engine.find_weighted_path(&from_key, &to_key, config.query.path_max_hops);

    let hops: Vec<cli::trace::TraceHop> = weighted_hops
        .into_iter()
        .map(|wh| {
            let from_c = engine
                .entity_key_to_candidate(&wh.from_key)
                .unwrap_or_else(|| resolve::Candidate {
                    name: wh.from_key.clone(),
                    kind: "unknown".to_string(),
                    file_path: String::new(),
                    line: 0,
                    ambiguous: false,
                });
            let to_c = engine
                .entity_key_to_candidate(&wh.to_key)
                .unwrap_or_else(|| resolve::Candidate {
                    name: wh.to_key.clone(),
                    kind: "unknown".to_string(),
                    file_path: String::new(),
                    line: 0,
                    ambiguous: false,
                });
            cli::trace::TraceHop {
                from: from_c,
                to: to_c,
                edge_type: wh.edge_type,
                edge_cost: wh.edge_cost,
                forward: wh.forward,
            }
        })
        .collect();

    let result = cli::trace::TraceResult {
        hops,
        resolution_from,
        resolution_to,
    };
    print!("{}", cli::trace::format_trace_result(&result));
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_cluster
// ---------------------------------------------------------------------------

fn cmd_cluster(identifier: &str, path_filter: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let config_path = Config::config_path(&repo_root);

    if !config_path.exists() {
        anyhow::bail!("Not a canopy project. Run `canopy init` first.");
    }

    let config = Config::load(&config_path)?;
    let store_inst = Store::open(&Config::store_path(&repo_root))?;

    let (cluster_id, label) = match store_inst.find_cluster_by_label(identifier)? {
        Some((id, label)) => (id, label),
        None => {
            let resolver =
                resolve::FuzzyResolver::new(&store_inst, config.query.max_suggestions);
            let resolution = resolver.resolve(identifier);
            match &resolution {
                resolve::Resolution::Exact(key) => {
                    match store_inst.get_entity_cluster(key)? {
                        Some(cid) => {
                            let label = store_inst
                                .get_cluster_label(cid)?
                                .unwrap_or_else(|| format!("cluster-{}", cid));
                            (cid, label)
                        }
                        None => {
                            eprintln!("Symbol '{}' is not in any cluster.", identifier);
                            std::process::exit(1);
                        }
                    }
                }
                _ => {
                    print!("{}", resolve::format_resolution(&resolution));
                    std::process::exit(1);
                }
            }
        }
    };

    let engine = query::QueryEngine::new(&store_inst);
    let mut result = engine.get_cluster_detail(
        cluster_id,
        &label,
        config.query.output_caps.cluster_members,
        config.query.output_caps.cluster_relationships,
    )?;

    let filter = build_path_filter(path_filter);
    if filter.is_some() {
        result.members.retain(|m| path_matches(&m.file_path, &filter));
    }

    print!("{}", cli::cluster::format_cluster_result(&result));
    Ok(())
}

// ---------------------------------------------------------------------------
// cmd_clusters
// ---------------------------------------------------------------------------

fn cmd_clusters(page: usize, path_filter: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let repo_root = git::find_root(&cwd)?;
    let store = Store::open(&Config::store_path(&repo_root))?;

    let filter = build_path_filter(path_filter);

    let all = if filter.is_some() {
        // Recount members per cluster considering only those matching the path filter
        let mut clusters = store.list_clusters()?;
        for entry in &mut clusters {
            let (cluster_id, _, ref mut count) = entry;
            let members = store.get_cluster_members(*cluster_id)?;
            let filtered_count = members.iter().filter(|name| {
                store.get_entity(name).ok().flatten()
                    .map(|e| path_matches(&e.file_path, &filter))
                    .unwrap_or(false)
            }).count();
            *count = filtered_count;
        }
        // Remove clusters with zero matching members and re-sort
        clusters.retain(|&(_, _, count)| count > 0);
        clusters.sort_by(|a, b| b.2.cmp(&a.2).then(a.1.cmp(&b.1)));
        clusters
    } else {
        store.list_clusters()?
    };

    if all.is_empty() {
        println!("No clusters found. Run `canopy reindex` first.");
        return Ok(());
    }

    let page_size = 50;
    let total_pages = (all.len() + page_size - 1) / page_size;
    let page = page.clamp(1, total_pages);
    let start = (page - 1) * page_size;
    let end = (start + page_size).min(all.len());

    for (_, label, count) in &all[start..end] {
        println!("{} ({} members)", label, count);
    }

    println!("\nPage {}/{}", page, total_pages);

    Ok(())
}

// ---------------------------------------------------------------------------
// File discovery
// ---------------------------------------------------------------------------

/// Walk the repo respecting .gitignore and config.indexing.ignore, return indexable file paths.
fn collect_indexable_files(repo_root: &Path, config: &Config) -> Vec<String> {
    let mut builder = WalkBuilder::new(repo_root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);

    // Add custom ignore patterns from config
    for pattern in &config.indexing.ignore {
        let mut override_builder = ignore::overrides::OverrideBuilder::new(repo_root);
        // Negate pattern to make it an ignore
        let _ = override_builder.add(&format!("!{pattern}"));
        if let Ok(overrides) = override_builder.build() {
            builder.overrides(overrides);
        }
    }

    let mut files = Vec::new();
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let path = entry.path().strip_prefix(repo_root).unwrap_or(entry.path());
        let path_str = path.to_string_lossy().to_string();
        if chunker::detect_language(&path_str).is_some() {
            files.push(path_str);
        }
    }
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// Chunking (Phase 1 -- CPU only, no network)
// ---------------------------------------------------------------------------

fn chunk_file(repo_root: &Path, file_path: &str, config: &Config) -> Result<Vec<Chunk>> {
    let full_path = repo_root.join(file_path);
    let source = std::fs::read_to_string(&full_path)
        .with_context(|| format!("Failed to read {file_path}"))?;

    let language = chunker::detect_language(file_path).unwrap_or("unknown");
    Ok(chunker::chunk_file(
        &source,
        file_path,
        language,
        config.indexing.merge_threshold,
        config.indexing.split_threshold,
        config.indexing.method_blocklist.as_deref(),
    ))
}

fn chunk_all_files(repo_root: &Path, config: &Config) -> Vec<(String, Vec<Chunk>)> {
    let files = collect_indexable_files(repo_root, config);
    println!("Found {} files", files.len());

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "  Parsing  {bar:30.cyan/dim} {pos}/{len} files  [{elapsed_precise}] {msg}",
        )
        .unwrap(),
    );

    let mut all_chunks = Vec::new();
    let mut total_chunks = 0usize;
    let mut total_symbols = 0usize;
    for file_path in &files {
        pb.set_message(truncate_path(file_path, 40));
        match chunk_file(repo_root, file_path, config) {
            Ok(chunks) => {
                total_symbols += chunks.iter().map(|c| c.defines.len()).sum::<usize>();
                total_chunks += chunks.len();
                all_chunks.push((file_path.clone(), chunks));
            }
            Err(e) => {
                pb.suspend(|| eprintln!("  ERROR {file_path}: {e}"));
            }
        }
        pb.inc(1);
    }

    pb.finish_with_message(format!("{total_chunks} chunks, {total_symbols} symbols"));

    all_chunks
}

fn truncate_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        path.to_string()
    } else {
        format!("...{}", &path[path.len() - max + 3..])
    }
}

// ---------------------------------------------------------------------------
// Progress bar helpers
// ---------------------------------------------------------------------------

fn make_progress_bar(total: usize, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total as u64);
    pb.set_style(
        ProgressStyle::with_template(&format!(
            "  {{spinner:.green}} {label:<7} {{bar:30.green/dim}} {{pos}}/{{len}}  [{{elapsed_precise}}  ETA {{eta_precise}}  {{per_sec}}]"
        ))
        .unwrap()
        .progress_chars("##-"),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(120));
    pb
}

fn finish_progress_bar(pb: &ProgressBar, count: usize, errors: usize) {
    let elapsed = pb.elapsed();
    let rate = if elapsed.as_secs_f64() > 0.0 {
        count as f64 / elapsed.as_secs_f64()
    } else {
        0.0
    };
    let err_msg = if errors > 0 {
        format!(", {errors} errors")
    } else {
        String::new()
    };
    pb.finish_with_message(format!("{count} done ({rate:.1}/s){err_msg}"));
}

// ---------------------------------------------------------------------------
// Phase 2: Embed and store chunks
// ---------------------------------------------------------------------------

/// Batch size for embedding API calls
const EMBED_BATCH_SIZE: usize = 64;

async fn embed_and_store_chunks(
    file_chunks: &[(String, Vec<Chunk>)],
    store: &Store,
    embed: &EmbedClient,
) -> Result<usize> {
    // Collect all chunk items
    struct ChunkItem {
        file_path: String,
        language: String,
        node_kinds: Vec<String>,
        line_range: (usize, usize),
        parent_scope: String,
        content: String,
    }

    let mut chunk_items: Vec<ChunkItem> = Vec::new();
    for (file_path, chunks) in file_chunks {
        for chunk in chunks {
            chunk_items.push(ChunkItem {
                file_path: file_path.clone(),
                language: chunk.language.clone(),
                node_kinds: chunk.node_kinds.clone(),
                line_range: (chunk.line_start, chunk.line_end),
                parent_scope: chunk.parent_scope.clone(),
                content: chunk.content.clone(),
            });
        }
    }

    if chunk_items.is_empty() {
        return Ok(0);
    }

    let total = chunk_items.len();
    let pb = make_progress_bar(total, "Chunks");

    let mut count = 0usize;
    let mut errors = 0usize;

    for batch_start in (0..chunk_items.len()).step_by(EMBED_BATCH_SIZE) {
        let batch_end = (batch_start + EMBED_BATCH_SIZE).min(chunk_items.len());
        let batch = &chunk_items[batch_start..batch_end];

        let texts: Vec<&str> = batch.iter().map(|item| item.content.as_str()).collect();
        let embeddings = match embed.embed(&texts).await {
            Ok(e) => e,
            Err(e) => {
                errors += batch.len();
                pb.suspend(|| eprintln!("  Warning: chunk embedding batch failed: {e}"));
                pb.inc(batch.len() as u64);
                continue;
            }
        };

        for (i, item) in batch.iter().enumerate() {
            if i >= embeddings.len() {
                errors += 1;
                continue;
            }
            let record = ChunkRecord {
                file_path: item.file_path.clone(),
                language: item.language.clone(),
                node_kinds: item.node_kinds.clone(),
                line_range: item.line_range,
                parent_scope: item.parent_scope.clone(),
                content: item.content.clone(),
            };
            match store.insert_chunk(record, &embeddings[i]) {
                Ok(_id) => count += 1,
                Err(e) => {
                    errors += 1;
                    pb.suspend(|| eprintln!("  Warning: chunk store failed for {}: {e}", item.file_path));
                }
            }
            pb.inc(1);
        }
    }

    finish_progress_bar(&pb, count, errors);
    Ok(count)
}

// ---------------------------------------------------------------------------
// Phase 4: Store entities (deterministic, no embedding)
// ---------------------------------------------------------------------------

fn store_entities(
    file_chunks: &[(String, Vec<Chunk>)],
    store: &Store,
) -> Result<usize> {
    let mut count = 0usize;
    let mut errors = 0usize;

    let mut entity_items: Vec<(String, graph::EntityDef)> = Vec::new();

    for (file_path, chunks) in file_chunks {
        // MODULE entity for the file itself
        let module_entity = graph::generate_module_entity(file_path);
        entity_items.push((file_path.clone(), module_entity));

        // Entities from each chunk's defined symbols
        for chunk in chunks {
            let entities = graph::generate_entities(chunk, "canopy");
            for entity in entities {
                entity_items.push((file_path.clone(), entity));
            }
        }
    }

    if entity_items.is_empty() {
        return Ok(0);
    }

    let total = entity_items.len();
    let pb = make_progress_bar(total, "Entities");

    for (file_path, entity) in &entity_items {
        match store.insert_entity(
            &entity.entity_name,
            &entity.entity_type,
            &entity.description,
            &entity.source_id,
            file_path,
            entity.metadata.clone(),
            entity.parent.clone(),
            entity.visibility.clone(),
        ) {
            Ok(()) => count += 1,
            Err(e) => {
                errors += 1;
                pb.suspend(|| {
                    eprintln!("  Warning: entity store failed for {file_path}: {e}");
                });
            }
        }
        pb.inc(1);
    }

    finish_progress_bar(&pb, count, errors);
    Ok(count)
}

// ---------------------------------------------------------------------------
// Phase 5: Store relationships (deterministic, no embedding)
// ---------------------------------------------------------------------------

fn store_relationships(
    file_chunks: &[(String, Vec<Chunk>)],
    store: &Store,
) -> Result<usize> {
    // Build global symbol map for cross-file reference resolution
    let symbol_map = graph::build_symbol_map(file_chunks, &HashMap::new());

    let mut rel_items: Vec<graph::RelationshipDef> = Vec::new();

    for (file_path, chunks) in file_chunks {
        // Collect all entities for this file
        let mut file_entities: Vec<graph::EntityDef> = Vec::new();
        for chunk in chunks {
            let entities = graph::generate_entities(chunk, "canopy");
            file_entities.extend(entities);
        }

        // DEFINES: module -> symbol
        let defines = graph::generate_defines_relationships(file_path, &file_entities);
        rel_items.extend(defines);

        // IMPORTS: once per file, from the first chunk's imports
        if let Some(first_chunk) = chunks.first() {
            let imports = graph::generate_imports_relationships(
                &first_chunk.imports,
                file_path,
                &symbol_map,
            );
            rel_items.extend(imports);
        }

        // Per-chunk typed relationship generators
        for chunk in chunks {
            let contains = graph::generate_contains_relationships(chunk, &symbol_map);
            rel_items.extend(contains);

            let refs = graph::generate_classified_relationships(chunk, &symbol_map);
            rel_items.extend(refs);

            let type_refs = graph::generate_type_ref_relationships(chunk, &symbol_map);
            rel_items.extend(type_refs);

            let fields = graph::generate_field_of_relationships(chunk, &symbol_map);
            rel_items.extend(fields);

            let impls = graph::generate_implements_relationships(chunk, &symbol_map);
            rel_items.extend(impls);
        }
    }

    // Dedup: one edge per (src, tgt), strongest type wins
    let rel_items = graph::dedup_relationships(rel_items);

    if rel_items.is_empty() {
        return Ok(0);
    }

    let total = rel_items.len();
    let pb = make_progress_bar(total, "Rels");

    let mut count = 0usize;
    let mut errors = 0usize;

    for rel in &rel_items {
        match store.insert_relationship(
            &rel.src_id,
            &rel.tgt_id,
            &rel.relationship_type,
            &rel.keywords,
            rel.weight,
            &rel.description,
            &rel.source_id,
            rel.ambiguous,
        ) {
            Ok(()) => count += 1,
            Err(e) => {
                errors += 1;
                pb.suspend(|| {
                    eprintln!(
                        "  Warning: relationship store failed ({} -> {}): {e}",
                        rel.src_id, rel.tgt_id
                    );
                });
            }
        }
        pb.inc(1);
    }

    finish_progress_bar(&pb, count, errors);
    Ok(count)
}

// ---------------------------------------------------------------------------
// Phase 6: Build and save HNSW indexes
// ---------------------------------------------------------------------------

fn rebuild_vector_indexes(store: &Store, repo_root: &Path) -> Result<()> {
    let chunk_embeddings = store.all_chunk_embeddings()?;

    let chunk_index = VectorIndex::build(&chunk_embeddings);

    store::save_vector_index(&chunk_index, &Config::vectors_path(repo_root))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Full index pipeline
// ---------------------------------------------------------------------------

async fn do_full_index(
    repo_root: &Path,
    store: &Store,
    embed: &EmbedClient,
    config: &mut Config,
) -> Result<()> {
    let started = Instant::now();

    // Phase 1: Chunk all files (CPU only, no network)
    let chunked = chunk_all_files(repo_root, config);

    // Phase 2: Embed and store chunks
    let chunk_count = embed_and_store_chunks(&chunked, store, embed).await?;

    // Phase 4: Store entities (deterministic from tree-sitter)
    let entity_count = store_entities(&chunked, store)?;

    // Phase 5: Store relationships (deterministic)
    let rel_count = store_relationships(&chunked, store)?;

    // Phase 5b: Detect hubs
    println!("  Detecting hubs...");
    store.detect_hubs()?;

    // Phase 5c: Compute clusters
    println!("  Computing clusters...");
    let hub_names = store.get_hub_entity_names()?;
    let (cluster_entities, affinities) = canopy::cluster::build_affinity_graph(store, &hub_names)?;
    let mut clusters = canopy::cluster::louvain(&cluster_entities, &affinities, 1.5);
    let hub_assignments = canopy::cluster::attach_hubs_to_clusters(store, &hub_names, &clusters)?;
    clusters.extend(hub_assignments);
    canopy::cluster::absorb_small_clusters(store, &mut clusters, 3)?;
    let cluster_labels = canopy::cluster::compute_cluster_labels(&clusters, store)?;
    store.store_clusters(&clusters)?;
    store.store_cluster_meta(&cluster_labels)?;
    store.set_meta("files_changed_since_cluster", "0")?;
    store.set_meta("total_indexed_files", &chunked.len().to_string())?;

    // Phase 6: Build and save HNSW indexes
    println!("  Building vector indexes...");
    rebuild_vector_indexes(store, repo_root)?;

    // Update last_sha
    config.indexing.last_sha = git::head_sha(repo_root)?;

    let elapsed = started.elapsed();
    println!();
    println!("  Indexing complete in {}", HumanDuration(elapsed));
    println!("    Files:         {}", chunked.len());
    println!("    Chunks:        {chunk_count}");
    println!("    Entities:      {entity_count}");
    println!("    Relationships: {rel_count}");
    if elapsed.as_secs() > 0 {
        let total = chunk_count + entity_count + rel_count;
        let rate = total as f64 / elapsed.as_secs_f64();
        println!("    Rate:          {rate:.1} items/s overall");
    }
    Ok(())
}
