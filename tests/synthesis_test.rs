#![allow(clippy::field_reassign_with_default)]

use canopy::synthesis::SynthesisClient;
use canopy::config::QueryConfig;

#[test]
fn test_synthesis_client_none_when_no_provider() {
    let config = QueryConfig::default();
    let client = SynthesisClient::new(&config, "http://localhost:11434");
    assert!(client.is_none());
}

#[test]
fn test_synthesis_client_none_when_no_model() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434");
    assert!(client.is_none());
}

#[test]
fn test_synthesis_client_some_when_configured() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    config.synthesis_model = Some("qwen3:8b".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434");
    assert!(client.is_some());
}

#[test]
fn test_ollama_request_format() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    config.synthesis_model = Some("qwen3:8b".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434").unwrap();

    let (url, body) = client.build_request("How does the query engine work?", "TOON context here");

    assert_eq!(url, "http://localhost:11434/api/chat");

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "qwen3:8b");
    assert_eq!(parsed["stream"], false);

    let messages = parsed["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "user");

    let system_content = messages[0]["content"].as_str().unwrap();
    assert!(system_content.contains("codebase expert"), "system prompt should contain instructions");

    let user_content = messages[1]["content"].as_str().unwrap();
    assert!(user_content.contains("TOON context here"), "user message should contain context");
    assert!(user_content.contains("How does the query engine work?"), "user message should contain question");
}

#[test]
fn test_openai_request_format() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("openai".to_string());
    config.synthesis_model = Some("gpt-4o-mini".to_string());
    config.synthesis_url = Some("https://api.openai.com".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434").unwrap();

    let (url, body) = client.build_request("What is Store?", "context");

    assert_eq!(url, "https://api.openai.com/v1/chat/completions");

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "gpt-4o-mini");
    assert_eq!(parsed["stream"], false);
}

#[test]
fn test_synthesis_url_defaults_to_fallback() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    config.synthesis_model = Some("qwen3:8b".to_string());
    let client = SynthesisClient::new(&config, "http://myhost:11434").unwrap();

    let (url, _body) = client.build_request("test", "context");
    assert_eq!(url, "http://myhost:11434/api/chat");
}

#[test]
fn test_synthesis_url_overrides_fallback() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    config.synthesis_model = Some("qwen3:8b".to_string());
    config.synthesis_url = Some("http://custom:11434".to_string());
    let client = SynthesisClient::new(&config, "http://default:11434").unwrap();

    let (url, _body) = client.build_request("test", "context");
    assert_eq!(url, "http://custom:11434/api/chat");
}

#[test]
fn test_parse_ollama_chat_response() {
    let body = r#"{
        "model": "qwen3:8b",
        "message": {
            "role": "assistant",
            "content": "The query engine uses vector search.\n\n- `src/query.rs:60` — Main QueryEngine struct"
        },
        "done": true
    }"#;
    let result = SynthesisClient::parse_ollama_response(body).unwrap();
    assert!(result.contains("query engine uses vector search"));
    assert!(result.contains("src/query.rs:60"));
}

#[test]
fn test_parse_openai_chat_response() {
    let body = r#"{
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "The store module handles persistence.\n\n- `src/store.rs:1` — Store struct"
            },
            "finish_reason": "stop"
        }]
    }"#;
    let result = SynthesisClient::parse_openai_response(body).unwrap();
    assert!(result.contains("store module handles persistence"));
    assert!(result.contains("src/store.rs:1"));
}

#[test]
fn test_parse_ollama_response_missing_content() {
    let body = r#"{"model": "qwen3:8b", "done": true}"#;
    let result = SynthesisClient::parse_ollama_response(body);
    assert!(result.is_err());
}

#[test]
fn test_parse_openai_response_empty_choices() {
    let body = r#"{"choices": []}"#;
    let result = SynthesisClient::parse_openai_response(body);
    assert!(result.is_err());
}

#[test]
fn test_strip_think_tags() {
    // Basic think block
    assert_eq!(
        SynthesisClient::strip_think_tags("<think>reasoning here</think>The answer."),
        "The answer."
    );
    // No think tags — pass through
    assert_eq!(
        SynthesisClient::strip_think_tags("Just an answer."),
        "Just an answer."
    );
    // Think block with newlines
    assert_eq!(
        SynthesisClient::strip_think_tags("<think>\nlong\nreasoning\n</think>\nThe answer."),
        "The answer."
    );
    // Unclosed think tag — strip from tag to end
    assert_eq!(
        SynthesisClient::strip_think_tags("<think>unfinished"),
        ""
    );
}

#[test]
fn test_parse_ollama_stream_chunk_content() {
    let line = r#"{"model":"qwen3:8b","message":{"role":"assistant","content":"The query"},"done":false}"#;
    assert_eq!(
        SynthesisClient::parse_ollama_stream_chunk(line),
        Some("The query".to_string())
    );
}

#[test]
fn test_parse_ollama_stream_chunk_done() {
    let line = r#"{"model":"qwen3:8b","message":{"role":"assistant","content":""},"done":true}"#;
    assert_eq!(SynthesisClient::parse_ollama_stream_chunk(line), None);
}

#[test]
fn test_parse_ollama_stream_chunk_empty_content() {
    let line = r#"{"model":"qwen3:8b","message":{"role":"assistant","content":""},"done":false}"#;
    assert_eq!(SynthesisClient::parse_ollama_stream_chunk(line), None);
}

#[test]
fn test_parse_openai_stream_chunk_content() {
    let line = r#"data: {"choices":[{"delta":{"content":"The query"},"index":0}]}"#;
    assert_eq!(
        SynthesisClient::parse_openai_stream_chunk(line),
        Some("The query".to_string())
    );
}

#[test]
fn test_parse_openai_stream_chunk_done() {
    let line = "data: [DONE]";
    assert_eq!(SynthesisClient::parse_openai_stream_chunk(line), None);
}

#[test]
fn test_parse_openai_stream_chunk_role_only() {
    let line = r#"data: {"choices":[{"delta":{"role":"assistant"},"index":0}]}"#;
    assert_eq!(SynthesisClient::parse_openai_stream_chunk(line), None);
}

#[test]
fn test_ollama_stream_request_format() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("ollama".to_string());
    config.synthesis_model = Some("qwen3:8b".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434").unwrap();

    let (url, body) = client.build_stream_request("How does query work?", "TOON context");

    assert_eq!(url, "http://localhost:11434/api/chat");

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "qwen3:8b");
    assert_eq!(parsed["stream"], true);

    let messages = parsed["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "user");
}

#[test]
fn test_openai_stream_request_format() {
    let mut config = QueryConfig::default();
    config.synthesis_provider = Some("openai".to_string());
    config.synthesis_model = Some("gpt-4o-mini".to_string());
    config.synthesis_url = Some("https://api.openai.com".to_string());
    let client = SynthesisClient::new(&config, "http://localhost:11434").unwrap();

    let (url, body) = client.build_stream_request("What is Store?", "context");

    assert_eq!(url, "https://api.openai.com/v1/chat/completions");

    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["model"], "gpt-4o-mini");
    assert_eq!(parsed["stream"], true);
}
