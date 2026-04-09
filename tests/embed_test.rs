use canopy::config::EmbeddingConfig;
use canopy::embed::{EmbedClient, EmbedProvider};
use serde_json::Value;

// ── Helper ────────────────────────────────────────────────────────────────────

fn ollama_client(url: &str, model: &str) -> EmbedClient {
    EmbedClient::new(EmbedProvider::Ollama {
        url: url.to_string(),
        model: model.to_string(),
    })
}

fn openai_client(url: &str, model: &str, api_key: &str) -> EmbedClient {
    EmbedClient::new(EmbedProvider::OpenAI {
        url: url.to_string(),
        model: model.to_string(),
        api_key: api_key.to_string(),
    })
}

// ── Request-format tests ──────────────────────────────────────────────────────

#[test]
fn test_ollama_request_format() {
    let client = ollama_client("http://localhost:11434", "nomic-embed-text");
    let (url, body) = client.build_request(&["hello world", "foo bar"]);

    assert_eq!(url, "http://localhost:11434/api/embed");

    let parsed: Value = serde_json::from_str(&body).expect("body must be valid JSON");
    assert_eq!(parsed["model"], "nomic-embed-text");

    let input = parsed["input"].as_array().expect("input must be an array");
    assert_eq!(input.len(), 2);
    assert_eq!(input[0], "hello world");
    assert_eq!(input[1], "foo bar");
}

#[test]
fn test_openai_request_format() {
    let client = openai_client(
        "https://api.openai.com",
        "text-embedding-3-small",
        "sk-secret",
    );
    let (url, body) = client.build_request(&["hello world"]);

    assert_eq!(url, "https://api.openai.com/v1/embeddings");

    let parsed: Value = serde_json::from_str(&body).expect("body must be valid JSON");
    assert_eq!(parsed["model"], "text-embedding-3-small");

    let input = parsed["input"].as_array().expect("input must be an array");
    assert_eq!(input.len(), 1);
    assert_eq!(input[0], "hello world");
}

// ── Response-parser tests ─────────────────────────────────────────────────────

#[test]
fn test_parse_ollama_response() {
    let body = r#"{"embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]}"#;
    let vecs = EmbedClient::parse_ollama_response(body).expect("parse must succeed");

    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 3);
    assert!((vecs[0][0] - 0.1f32).abs() < 1e-6);
    assert!((vecs[0][1] - 0.2f32).abs() < 1e-6);
    assert!((vecs[0][2] - 0.3f32).abs() < 1e-6);
    assert!((vecs[1][0] - 0.4f32).abs() < 1e-6);
    assert!((vecs[1][1] - 0.5f32).abs() < 1e-6);
    assert!((vecs[1][2] - 0.6f32).abs() < 1e-6);
}

#[test]
fn test_parse_openai_response() {
    let body = r#"{
        "data": [
            {"embedding": [0.1, 0.2, 0.3]},
            {"embedding": [0.4, 0.5, 0.6]}
        ]
    }"#;
    let vecs = EmbedClient::parse_openai_response(body).expect("parse must succeed");

    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), 3);
    assert!((vecs[0][0] - 0.1f32).abs() < 1e-6);
    assert!((vecs[0][1] - 0.2f32).abs() < 1e-6);
    assert!((vecs[0][2] - 0.3f32).abs() < 1e-6);
    assert!((vecs[1][0] - 0.4f32).abs() < 1e-6);
    assert!((vecs[1][1] - 0.5f32).abs() < 1e-6);
    assert!((vecs[1][2] - 0.6f32).abs() < 1e-6);
}

// ── Provider-from-config test ─────────────────────────────────────────────────

#[test]
fn test_provider_from_config_ollama() {
    let config = EmbeddingConfig {
        provider: "ollama".to_string(),
        model: "nomic-embed-text".to_string(),
        url: "http://localhost:11434".to_string(),
        api_key_env: None,
        dimensions: None,
    };

    let provider = EmbedProvider::from_config(&config, None);
    // Verify by building a request through a client wrapping this provider.
    let client = EmbedClient::new(provider);
    let (url, body) = client.build_request(&["test"]);

    assert_eq!(url, "http://localhost:11434/api/embed");
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "nomic-embed-text");
}

#[test]
fn test_provider_from_config_openai() {
    let config = EmbeddingConfig {
        provider: "openai".to_string(),
        model: "text-embedding-3-small".to_string(),
        url: "https://api.openai.com".to_string(),
        api_key_env: Some("OPENAI_API_KEY".to_string()),
        dimensions: Some(1536),
    };

    let provider = EmbedProvider::from_config(&config, Some("sk-test-key".to_string()));
    let client = EmbedClient::new(provider);
    let (url, body) = client.build_request(&["test"]);

    assert_eq!(url, "https://api.openai.com/v1/embeddings");
    let parsed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "text-embedding-3-small");
}

#[test]
fn test_provider_from_config_unknown_defaults_to_ollama() {
    let config = EmbeddingConfig {
        provider: "some-unknown-provider".to_string(),
        model: "my-model".to_string(),
        url: "http://my-host:11434".to_string(),
        api_key_env: None,
        dimensions: None,
    };

    let provider = EmbedProvider::from_config(&config, None);
    let client = EmbedClient::new(provider);
    let (url, _body) = client.build_request(&["test"]);

    // Unknown provider falls through to Ollama path.
    assert_eq!(url, "http://my-host:11434/api/embed");
}
