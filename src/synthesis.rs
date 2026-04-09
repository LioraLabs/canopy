// Synthesis client: optional LLM chat for answer generation

use anyhow::{anyhow, Result};
use futures::channel::mpsc;
use futures::SinkExt;
use serde_json::Value;

use crate::config::QueryConfig;

// ── Prompt ──────────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = r#"You are a codebase expert. Answer questions by synthesizing information from the provided context. Use ONLY the context below — do not invent or infer information beyond what is explicitly stated.

Instructions:
1. Lead with 3-4 sentences directly answering the question.
2. Then list relevant files as bullet points: `path/to/file.rs:line` — one sentence explaining relevance.
3. If the context doesn't fully answer the query, state what IS available and note what's missing.
4. When the query asks where to add or implement something, suggest placement based on existing structure and note extension points.
5. No preamble, no "Summary" sections, no numbered walkthroughs.
6. Target under 200 words total."#;

// ── Client ──────────────────────────────────────────────────────────────────

pub struct SynthesisClient {
    http: reqwest::Client,
    provider: String,
    model: String,
    url: String,
    api_key: Option<String>,
}

impl SynthesisClient {
    /// Create a SynthesisClient if synthesis is configured (both provider and model set).
    /// Returns None if synthesis is not configured.
    /// `fallback_url` is used when `synthesis_url` is not set (typically the embedding URL).
    pub fn new(config: &QueryConfig, fallback_url: &str) -> Option<Self> {
        let provider = config.synthesis_provider.as_ref()?;
        let model = config.synthesis_model.as_ref()?;

        let url = config
            .synthesis_url
            .as_deref()
            .unwrap_or(fallback_url)
            .to_string();

        let api_key = config
            .synthesis_api_key_env
            .as_deref()
            .and_then(|env_var| std::env::var(env_var).ok());

        if provider == "openai" && api_key.is_none() && config.synthesis_api_key_env.is_some() {
            eprintln!(
                "[canopy] warning: synthesis_api_key_env is set but the environment variable is not defined"
            );
        }

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Some(Self {
            http,
            provider: provider.clone(),
            model: model.clone(),
            url,
            api_key,
        })
    }

    /// Build the (url, json_body) pair for a synthesis request.
    /// Exposed for unit testing.
    pub fn build_request(&self, question: &str, toon_context: &str) -> (String, String) {
        let user_message = format!("---Context---\n\n{toon_context}\n\n---Query---\n\n{question}");

        let messages = serde_json::json!([
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_message },
        ]);

        match self.provider.as_str() {
            "openai" => {
                let endpoint = format!("{}/v1/chat/completions", self.url);
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": false,
                });
                (endpoint, body.to_string())
            }
            _ => {
                // Ollama (default)
                let endpoint = format!("{}/api/chat", self.url);
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": false,
                });
                (endpoint, body.to_string())
            }
        }
    }

    /// Call the LLM with the question and TOON context, return the response text.
    pub async fn synthesize(&self, question: &str, toon_context: &str) -> Result<String> {
        let (url, body) = self.build_request(question, toon_context);

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body);

        if let Some(api_key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!("synthesis API returned {}: {}", status, text));
        }

        let content = match self.provider.as_str() {
            "openai" => Self::parse_openai_response(&text),
            _ => Self::parse_ollama_response(&text),
        }?;

        Ok(Self::strip_think_tags(&content))
    }

    /// Build the (url, json_body) pair for a streaming synthesis request.
    /// Same as `build_request` but with `"stream": true`.
    pub fn build_stream_request(&self, question: &str, toon_context: &str) -> (String, String) {
        let user_message = format!("---Context---\n\n{toon_context}\n\n---Query---\n\n{question}");

        let messages = serde_json::json!([
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_message },
        ]);

        match self.provider.as_str() {
            "openai" => {
                let endpoint = format!("{}/v1/chat/completions", self.url);
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": true,
                });
                (endpoint, body.to_string())
            }
            _ => {
                let endpoint = format!("{}/api/chat", self.url);
                let body = serde_json::json!({
                    "model": self.model,
                    "messages": messages,
                    "stream": true,
                });
                (endpoint, body.to_string())
            }
        }
    }

    /// Stream synthesis token-by-token. Sends the request, then spawns a task
    /// that parses the streaming response and yields text deltas through a channel.
    /// Think tags are buffered and stripped before any tokens are emitted.
    pub async fn synthesize_stream(
        &self,
        question: &str,
        toon_context: &str,
    ) -> Result<mpsc::Receiver<String>> {
        let (url, body) = self.build_stream_request(question, toon_context);

        let mut req = self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body);

        if let Some(api_key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            return Err(anyhow!("synthesis API returned {}: {}", status, text));
        }

        let (mut tx, rx) = mpsc::channel(32);
        let provider = self.provider.clone();

        tokio::spawn(async move {
            let mut resp = resp;
            let mut line_buf = String::new();
            let mut think_buf = String::new();
            let mut think_handled = false;

            while let Ok(Some(bytes)) = resp.chunk().await {
                line_buf.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(newline_pos) = line_buf.find('\n') {
                    let line = line_buf[..newline_pos].trim().to_string();
                    line_buf = line_buf[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let delta = match provider.as_str() {
                        "openai" => Self::parse_openai_stream_chunk(&line),
                        _ => Self::parse_ollama_stream_chunk(&line),
                    };

                    if let Some(text) = delta {
                        if !think_handled {
                            think_buf.push_str(&text);
                            if think_buf.starts_with("<think>") {
                                // Inside think block — check for closing tag
                                if let Some(end) = think_buf.find("</think>") {
                                    think_handled = true;
                                    let remainder = think_buf[end + 8..].trim_start().to_string();
                                    if !remainder.is_empty()
                                        && tx.send(remainder).await.is_err()
                                    {
                                        return;
                                    }
                                }
                            } else if "<think>".starts_with(think_buf.trim_start())
                                && think_buf.len() < 8
                            {
                                // Could still be the start of <think>, keep buffering
                            } else {
                                // Not a think block — flush accumulated
                                think_handled = true;
                                if tx.send(think_buf.clone()).await.is_err() {
                                    return;
                                }
                            }
                        } else if tx.send(text).await.is_err() {
                            return; // receiver dropped
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    /// Strip `<think>...</think>` blocks that reasoning models emit.
    pub fn strip_think_tags(s: &str) -> String {
        let mut result = s.to_string();
        while let Some(start) = result.find("<think>") {
            if let Some(end) = result.find("</think>") {
                result = format!("{}{}", &result[..start], &result[end + 8..]);
            } else {
                // Unclosed <think> — strip from tag to end
                result = result[..start].to_string();
            }
        }
        result.trim().to_string()
    }

    // ── Response parsers (public for testing) ────────────────────────────

    pub fn parse_ollama_response(body: &str) -> Result<String> {
        let v: Value = serde_json::from_str(body)?;
        v["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing message.content in Ollama chat response"))
    }

    pub fn parse_openai_response(body: &str) -> Result<String> {
        let v: Value = serde_json::from_str(body)?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("missing choices[0].message.content in OpenAI response"))
    }

    // ── Streaming parsers ───────────────────────────────────────────────

    /// Parse one NDJSON line from an Ollama streaming response.
    /// Returns the text delta, or None if the chunk is empty or signals done.
    pub fn parse_ollama_stream_chunk(line: &str) -> Option<String> {
        let v: Value = serde_json::from_str(line).ok()?;
        if v["done"].as_bool() == Some(true) {
            return None;
        }
        let content = v["message"]["content"].as_str()?;
        if content.is_empty() {
            return None;
        }
        Some(content.to_string())
    }

    /// Parse one SSE line from an OpenAI streaming response.
    /// Expects `data: {...}` format. Returns the text delta, or None for [DONE] / role-only.
    pub fn parse_openai_stream_chunk(line: &str) -> Option<String> {
        let data = line.strip_prefix("data: ")?;
        if data == "[DONE]" {
            return None;
        }
        let v: Value = serde_json::from_str(data).ok()?;
        let content = v["choices"][0]["delta"]["content"].as_str()?;
        if content.is_empty() {
            return None;
        }
        Some(content.to_string())
    }
}
