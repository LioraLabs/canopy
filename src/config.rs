use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub project: ProjectConfig,
    pub indexing: IndexingConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub query: QueryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexingConfig {
    #[serde(default)]
    pub last_sha: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
    #[serde(default = "default_merge_threshold")]
    pub merge_threshold: usize,
    #[serde(default = "default_split_threshold")]
    pub split_threshold: usize,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub method_blocklist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default = "default_embedding_url")]
    pub url: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub dimensions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default = "default_max_suggestions")]
    pub max_suggestions: usize,
    #[serde(default = "default_path_max_hops")]
    pub path_max_hops: usize,
    #[serde(default)]
    pub output_caps: OutputCaps,
    #[serde(default)]
    pub synthesis_provider: Option<String>,
    #[serde(default)]
    pub synthesis_model: Option<String>,
    #[serde(default)]
    pub synthesis_url: Option<String>,
    #[serde(default)]
    pub synthesis_api_key_env: Option<String>,
    #[serde(default = "default_test_penalty")]
    pub test_penalty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputCaps {
    #[serde(default = "default_symbols_per_chunk")]
    pub symbols_per_chunk: usize,
    #[serde(default = "default_related_neighbors")]
    pub related_neighbors: usize,
    #[serde(default = "default_map_relationships_per_category")]
    pub map_relationships_per_category: usize,
    #[serde(default = "default_cluster_members")]
    pub cluster_members: usize,
    #[serde(default = "default_cluster_relationships")]
    pub cluster_relationships: usize,
}

fn default_symbols_per_chunk() -> usize { 5 }
fn default_related_neighbors() -> usize { 5 }
fn default_map_relationships_per_category() -> usize { 20 }
fn default_cluster_members() -> usize { 30 }
fn default_cluster_relationships() -> usize { 30 }

impl Default for OutputCaps {
    fn default() -> Self {
        Self {
            symbols_per_chunk: default_symbols_per_chunk(),
            related_neighbors: default_related_neighbors(),
            map_relationships_per_category: default_map_relationships_per_category(),
            cluster_members: default_cluster_members(),
            cluster_relationships: default_cluster_relationships(),
        }
    }
}

fn default_merge_threshold() -> usize { 20 }
fn default_split_threshold() -> usize { 200 }
fn default_concurrency() -> usize { 8 }
fn default_provider() -> String { "ollama".to_string() }
fn default_embedding_model() -> String { "qwen3-embedding:4b".to_string() }
fn default_embedding_url() -> String { "http://localhost:11434".to_string() }
fn default_test_penalty() -> f64 { 0.3 }
fn default_top_k() -> usize { 10 }
fn default_max_suggestions() -> usize { 10 }
fn default_path_max_hops() -> usize { 15 }

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_embedding_model(),
            url: default_embedding_url(),
            api_key_env: None,
            dimensions: None,
        }
    }
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            top_k: default_top_k(),
            max_suggestions: default_max_suggestions(),
            path_max_hops: default_path_max_hops(),
            output_caps: OutputCaps::default(),
            synthesis_provider: None,
            synthesis_model: None,
            synthesis_url: None,
            synthesis_api_key_env: None,
            test_penalty: default_test_penalty(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn default_for(name: &str) -> Self {
        Self {
            project: ProjectConfig {
                name: name.to_string(),
            },
            indexing: IndexingConfig {
                last_sha: String::new(),
                languages: vec![],
                ignore: vec![],
                merge_threshold: default_merge_threshold(),
                split_threshold: default_split_threshold(),
                concurrency: default_concurrency(),
                method_blocklist: None,
            },
            embedding: EmbeddingConfig::default(),
            query: QueryConfig::default(),
        }
    }

    pub fn canopy_dir(repo_root: &Path) -> PathBuf {
        repo_root.join(".canopy")
    }

    pub fn config_path(repo_root: &Path) -> PathBuf {
        Self::canopy_dir(repo_root).join("canopy.toml")
    }

    pub fn store_path(repo_root: &Path) -> PathBuf {
        Self::canopy_dir(repo_root).join("store.redb")
    }

    pub fn vectors_path(repo_root: &Path) -> PathBuf {
        Self::canopy_dir(repo_root).join("vectors.idx")
    }
}
