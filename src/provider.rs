//! Provider abstraction over OpenAI-compatible chat completion endpoints.
//!
//! A single trait [`ChatProvider`] covers every backend Damascus talks to:
//! OpenAI, OpenRouter, Google AI Studio (OpenAI-compatible path), Ollama,
//! vLLM and llama.cpp all speak the same `/chat/completions` shape. Tests use
//! an in-process mock that implements the same trait, so the whole Fold Loop
//! runs without a network.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::ProviderConfig;

/// A resolved reference to a concrete model on a concrete provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    /// Parse a `provider/model` string. Only the first `/` splits the provider
    /// from the model, so model ids that themselves contain slashes
    /// (e.g. `openrouter/deepseek/deepseek-chat`) are preserved.
    pub fn parse(s: &str) -> Result<Self> {
        let (provider, model) = s
            .split_once('/')
            .ok_or_else(|| anyhow!("model ref `{s}` must be `provider/model`"))?;
        if provider.is_empty() || model.is_empty() {
            return Err(anyhow!("model ref `{s}` has an empty provider or model"));
        }
        Ok(ModelRef {
            provider: provider.to_string(),
            model: model.to_string(),
        })
    }
}

impl std::fmt::Display for ModelRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Message {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: content.into(),
        }
    }
}

/// One chat completion request.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: ModelRef,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

pub type ChatFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

/// The single abstraction the orchestrator depends on.
pub trait ChatProvider: Send + Sync {
    fn complete<'a>(&'a self, req: ChatRequest) -> ChatFuture<'a>;
}

// ----- OpenAI-compatible HTTP client -------------------------------------

/// Talks to any OpenAI-compatible `/chat/completions` endpoint. Resolves the
/// per-provider base URL and API key from the parsed config.
pub struct OpenAiClient {
    http: reqwest::Client,
    providers: HashMap<String, ResolvedProvider>,
}

struct ResolvedProvider {
    base_url: String,
    api_key: Option<String>,
    extra_headers: HashMap<String, String>,
}

impl OpenAiClient {
    pub fn new(providers: &HashMap<String, ProviderConfig>, timeout: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .context("building HTTP client")?;
        let mut resolved = HashMap::new();
        for (name, cfg) in providers {
            let api_key = cfg.resolve_api_key();
            resolved.insert(
                name.clone(),
                ResolvedProvider {
                    base_url: cfg.base_url.trim_end_matches('/').to_string(),
                    api_key,
                    extra_headers: cfg.extra_headers.clone(),
                },
            );
        }
        Ok(OpenAiClient {
            http,
            providers: resolved,
        })
    }

    async fn call(&self, req: ChatRequest) -> Result<String> {
        let provider = self.providers.get(&req.model.provider).ok_or_else(|| {
            anyhow!(
                "no provider `{}` configured for model `{}`",
                req.model.provider,
                req.model
            )
        })?;

        let url = format!("{}/chat/completions", provider.base_url);
        let body = ChatCompletionBody {
            model: req.model.model.clone(),
            messages: req
                .messages
                .iter()
                .map(|m| WireMessage {
                    role: m.role.as_str(),
                    content: &m.content,
                })
                .collect(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        };

        let mut builder = self.http.post(&url).json(&body);
        if let Some(key) = &provider.api_key {
            builder = builder.bearer_auth(key);
        }
        for (k, v) in &provider.extra_headers {
            builder = builder.header(k, v);
        }

        let resp = builder
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            return Err(anyhow!(
                "provider returned {status}: {}",
                truncate(&text, 600)
            ));
        }
        let parsed: ChatCompletionResponse = serde_json::from_str(&text)
            .with_context(|| format!("parsing response: {}", truncate(&text, 400)))?;
        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("provider returned no choices"))?;
        Ok(choice.message.content.unwrap_or_default())
    }
}

impl ChatProvider for OpenAiClient {
    fn complete<'a>(&'a self, req: ChatRequest) -> ChatFuture<'a> {
        Box::pin(self.call(req))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

#[derive(Serialize)]
struct ChatCompletionBody<'a> {
    model: String,
    messages: Vec<WireMessage<'a>>,
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: RespMessage,
}

#[derive(Deserialize)]
struct RespMessage {
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_model_ref() {
        let m = ModelRef::parse("openai/gpt-5-mini").unwrap();
        assert_eq!(m.provider, "openai");
        assert_eq!(m.model, "gpt-5-mini");
    }

    #[test]
    fn keeps_slashes_in_model_id() {
        let m = ModelRef::parse("openrouter/deepseek/deepseek-chat").unwrap();
        assert_eq!(m.provider, "openrouter");
        assert_eq!(m.model, "deepseek/deepseek-chat");
    }

    #[test]
    fn rejects_missing_slash() {
        assert!(ModelRef::parse("gpt-5-mini").is_err());
    }
}
