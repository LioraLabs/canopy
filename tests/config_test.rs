use canopy::config::Config;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn test_config_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");

    let config = Config::default_for("test-project");
    config.save(&config_path).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.project.name, "test-project");
    assert_eq!(loaded.embedding.provider, "ollama");
    assert_eq!(loaded.embedding.model, "qwen3-embedding:4b");
    assert_eq!(loaded.embedding.url, "http://localhost:11434");
    assert_eq!(loaded.embedding.dimensions, None);
    assert_eq!(loaded.indexing.merge_threshold, 20);
    assert_eq!(loaded.indexing.split_threshold, 200);
    assert_eq!(loaded.query.top_k, 10);
    assert_eq!(loaded.query.max_suggestions, 10);
}

#[test]
fn test_config_with_dimensions() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");

    let mut config = Config::default_for("test-project");
    config.embedding.dimensions = Some(768);
    config.save(&config_path).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.embedding.dimensions, Some(768));
}

#[test]
fn test_config_with_synthesis() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");

    let mut config = Config::default_for("test-project");
    config.query.synthesis_provider = Some("ollama".to_string());
    config.query.synthesis_model = Some("qwen2.5-coder:32b".to_string());
    config.save(&config_path).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.query.synthesis_provider.as_deref(), Some("ollama"));
    assert_eq!(loaded.query.synthesis_model.as_deref(), Some("qwen2.5-coder:32b"));
}

#[test]
fn test_config_with_openai_provider() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");

    let mut config = Config::default_for("test-project");
    config.embedding.provider = "openai".to_string();
    config.embedding.model = "text-embedding-3-small".to_string();
    config.embedding.url = "https://api.openai.com".to_string();
    config.embedding.api_key_env = Some("OPENAI_API_KEY".to_string());
    config.save(&config_path).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.embedding.provider, "openai");
    assert_eq!(loaded.embedding.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
}

#[test]
fn test_canopy_dir_helper() {
    let repo_root = Path::new("/home/user/myproject");
    let canopy_dir = Config::canopy_dir(repo_root);
    assert_eq!(canopy_dir, repo_root.join(".canopy"));
}

#[test]
fn test_config_path_helper() {
    let repo_root = Path::new("/home/user/myproject");
    let config_path = Config::config_path(repo_root);
    assert_eq!(config_path, repo_root.join(".canopy").join("canopy.toml"));
}

#[test]
fn test_store_path_helper() {
    let repo_root = Path::new("/home/user/myproject");
    let store_path = Config::store_path(repo_root);
    assert_eq!(store_path, repo_root.join(".canopy").join("store.redb"));
}

#[test]
fn test_vectors_path_helper() {
    let repo_root = Path::new("/home/user/myproject");
    let vectors_path = Config::vectors_path(repo_root);
    assert_eq!(vectors_path, repo_root.join(".canopy").join("vectors.idx"));
}

#[test]
fn test_query_config_defaults() {
    let config = canopy::config::Config::default_for("test");
    assert_eq!(config.query.top_k, 10);
    assert_eq!(config.query.max_suggestions, 10);
    assert_eq!(config.query.path_max_hops, 15);
    assert_eq!(config.query.output_caps.symbols_per_chunk, 5);
    assert_eq!(config.query.output_caps.related_neighbors, 5);
    assert_eq!(config.query.output_caps.map_relationships_per_category, 20);
    assert_eq!(config.query.output_caps.cluster_members, 30);
    assert_eq!(config.query.output_caps.cluster_relationships, 30);
}

#[test]
fn test_default_embedding_model_is_nomic() {
    let config = canopy::config::Config::default_for("test");
    assert_eq!(config.embedding.model, "qwen3-embedding:4b");
}

#[test]
fn test_config_with_synthesis_url_and_api_key_env() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");

    let mut config = Config::default_for("test-project");
    config.query.synthesis_provider = Some("openai".to_string());
    config.query.synthesis_model = Some("gpt-4o-mini".to_string());
    config.query.synthesis_url = Some("https://api.openai.com".to_string());
    config.query.synthesis_api_key_env = Some("OPENAI_API_KEY".to_string());
    config.save(&config_path).unwrap();

    let loaded = Config::load(&config_path).unwrap();
    assert_eq!(loaded.query.synthesis_url.as_deref(), Some("https://api.openai.com"));
    assert_eq!(loaded.query.synthesis_api_key_env.as_deref(), Some("OPENAI_API_KEY"));
}

#[test]
fn test_method_blocklist_defaults_to_none() {
    let toml = r#"
[project]
name = "test"

[indexing]
"#;
    let config: Config = toml::from_str(toml).unwrap();
    assert!(config.indexing.method_blocklist.is_none());
}

#[test]
fn test_method_blocklist_custom() {
    let toml = r#"
[project]
name = "test"

[indexing]
method_blocklist = ["clone", "iter", "map"]
"#;
    let config: Config = toml::from_str(toml).unwrap();
    let bl = config.indexing.method_blocklist.unwrap();
    assert_eq!(bl.len(), 3);
    assert!(bl.contains(&"clone".to_string()));
}

#[test]
fn test_test_penalty_default() {
    let config = Config::default_for("test-project");
    assert!((config.query.test_penalty - 0.3).abs() < f64::EPSILON);
}

#[test]
fn test_test_penalty_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config_path = dir.path().join("canopy.toml");
    let mut config = Config::default_for("test-project");
    config.query.test_penalty = 0.5;
    config.save(&config_path).unwrap();
    let loaded = Config::load(&config_path).unwrap();
    assert!((loaded.query.test_penalty - 0.5).abs() < f64::EPSILON);
}
