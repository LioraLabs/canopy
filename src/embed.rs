// Embedding client: Ollama + OpenAI-compatible APIs

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::config::EmbeddingConfig;

// ── Provider ─────────────────────────────────────────────────────────────────

pub enum EmbedProvider {
    Ollama { url: String, model: String },
    OpenAI { url: String, model: String, api_key: String },
}

impl EmbedProvider {
    pub fn from_config(config: &EmbeddingConfig, api_key: Option<String>) -> Self {
        match config.provider.as_str() {
            "openai" => EmbedProvider::OpenAI {
                url: config.url.clone(),
                model: config.model.clone(),
                api_key: api_key.unwrap_or_default(),
            },
            _ => EmbedProvider::Ollama {
                url: config.url.clone(),
                model: config.model.clone(),
            },
        }
    }
}

// ── Client ───────────────────────────────────────────────────────────────────

/// Conservative character limit for embedding inputs.
/// snowflake-arctic-embed2 has an 8192-token context window. Code tokenizes
/// at roughly 3 chars/token, so 8000 chars ≈ 2700 tokens — well within limits
/// even for token-dense code. Inputs longer than this are truncated before
/// embedding. The full content is still stored in the DB and returned in
/// query results.
const MAX_EMBED_CHARS: usize = 8_000;

pub struct EmbedClient {
    provider: EmbedProvider,
    http: reqwest::Client,
}

impl EmbedClient {
    pub fn new(provider: EmbedProvider) -> Self {
        Self {
            provider,
            http: reqwest::Client::new(),
        }
    }

    /// Build the (url, json_body) pair for the given texts without sending.
    /// Exposed for unit testing.
    pub fn build_request(&self, texts: &[&str]) -> (String, String) {
        match &self.provider {
            EmbedProvider::Ollama { url, model } => {
                let endpoint = format!("{}/api/embed", url);
                let body = serde_json::json!({
                    "model": model,
                    "input": texts,
                });
                (endpoint, body.to_string())
            }
            EmbedProvider::OpenAI { url, model, .. } => {
                let endpoint = format!("{}/v1/embeddings", url);
                let body = serde_json::json!({
                    "model": model,
                    "input": texts,
                });
                (endpoint, body.to_string())
            }
        }
    }

    /// Batch-embed a slice of texts, returning one vector per text.
    /// Inputs exceeding MAX_EMBED_CHARS are truncated at a line boundary.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let truncated: Vec<String> = texts
            .iter()
            .map(|t| truncate_to_limit(t, MAX_EMBED_CHARS))
            .collect();
        let refs: Vec<&str> = truncated.iter().map(|s| s.as_str()).collect();
        let (url, body) = self.build_request(&refs);

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body);

        if let EmbedProvider::OpenAI { api_key, .. } = &self.provider {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!("embedding API returned {}: {}", status, text));
        }

        match &self.provider {
            EmbedProvider::Ollama { .. } => Self::parse_ollama_response(&text),
            EmbedProvider::OpenAI { .. } => Self::parse_openai_response(&text),
        }
    }

    /// Convenience: embed a single text.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow!("embedding API returned empty results"))
    }

    /// Embed a probe string and return the vector length as the model's
    /// embedding dimension.
    pub async fn probe_dimensions(&self) -> Result<usize> {
        let vec = self.embed_one("dimension probe").await
            .map_err(|e| {
                let server_info = match &self.provider {
                    EmbedProvider::Ollama { url, model } => {
                        format!("{url} (model: {model})")
                    }
                    EmbedProvider::OpenAI { url, model, .. } => {
                        format!("{url} (model: {model})")
                    }
                };
                anyhow!("Cannot connect to embedding server at {server_info}.\n\
                         Is the server running? Original error: {e}")
            })?;
        Ok(vec.len())
    }

    // ── Response parsers (public for testing) ─────────────────────────────

    pub fn parse_ollama_response(body: &str) -> Result<Vec<Vec<f32>>> {
        let v: Value = serde_json::from_str(body)?;
        let embeddings = v["embeddings"]
            .as_array()
            .ok_or_else(|| anyhow!("missing 'embeddings' array in Ollama response"))?;

        embeddings
            .iter()
            .map(|row| {
                row.as_array()
                    .ok_or_else(|| anyhow!("embedding row is not an array"))?
                    .iter()
                    .map(|x| {
                        x.as_f64()
                            .map(|f| f as f32)
                            .ok_or_else(|| anyhow!("non-numeric value in embedding"))
                    })
                    .collect::<Result<Vec<f32>>>()
            })
            .collect()
    }

    pub fn parse_openai_response(body: &str) -> Result<Vec<Vec<f32>>> {
        let v: Value = serde_json::from_str(body)?;
        let data = v["data"]
            .as_array()
            .ok_or_else(|| anyhow!("missing 'data' array in OpenAI response"))?;

        data.iter()
            .map(|item| {
                item["embedding"]
                    .as_array()
                    .ok_or_else(|| anyhow!("missing 'embedding' key in data item"))?
                    .iter()
                    .map(|x| {
                        x.as_f64()
                            .map(|f| f as f32)
                            .ok_or_else(|| anyhow!("non-numeric value in embedding"))
                    })
                    .collect::<Result<Vec<f32>>>()
            })
            .collect()
    }
}

/// Truncate text to at most `limit` characters, cutting at the last newline before the limit.
fn truncate_to_limit(text: &str, limit: usize) -> String {
    // Find the byte offset of the `limit`-th character (or end of string)
    let byte_limit = text
        .char_indices()
        .nth(limit)
        .map(|(i, _)| i)
        .unwrap_or(text.len());

    if byte_limit >= text.len() {
        return text.to_string();
    }

    match text[..byte_limit].rfind('\n') {
        Some(pos) => text[..pos].to_string(),
        None => text[..byte_limit].to_string(),
    }
}
