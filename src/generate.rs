//! Best-of-N candidate generation — the test-time-scaling lever. We spend extra
//! inference (multiple samples at rising temperatures, optionally across an
//! ensemble of models) instead of relying on one strong model.

use anyhow::Result;
use futures::stream::{self, StreamExt};

use crate::config::Config;
use crate::edits::{parse_blocks_fallback, EditBlock};
use crate::provider::{ChatProvider, ChatRequest, Message, ModelRef};

/// A single generated candidate for a step.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub index: usize,
    pub raw: String,
    pub blocks: Vec<EditBlock>,
    pub temperature: f32,
    /// Which model in the ensemble produced this candidate.
    pub model: String,
}

/// Generate `n` candidates concurrently for the given drafter messages. Each
/// sample uses a slightly higher temperature for diversity. Samples whose output
/// contains no parseable edit blocks are dropped (they cannot be verified).
pub async fn sample_candidates(
    provider: &dyn ChatProvider,
    models: &[ModelRef],
    cfg: &Config,
    system: String,
    user: String,
    n: usize,
    default_path: Option<String>,
) -> Vec<Candidate> {
    let temps = cfg.scaling.track_temperatures(n);
    let concurrency = cfg.scaling.concurrency.max(1);
    let max_tokens = cfg.scaling.max_tokens;

    let pool: Vec<ModelRef> = if models.is_empty() {
        return Vec::new();
    } else {
        models.to_vec()
    };
    type SampleOut = Option<(String, Vec<EditBlock>, f32, String)>;
    let results: Vec<SampleOut> = stream::iter(temps.into_iter().enumerate())
        .map(|(i, temp)| {
            let dp = default_path.clone();
            // Spread samples across the ensemble round-robin.
            let model = pool[i % pool.len()].clone();
            let model_name = model.to_string();
            let req = ChatRequest {
                model,
                messages: vec![Message::system(system.clone()), Message::user(user.clone())],
                temperature: temp,
                max_tokens,
            };
            async move {
                match provider.complete(req).await {
                    Ok(raw) => {
                        let blocks = parse_blocks_fallback(&raw, dp.as_deref());
                        if blocks.is_empty() {
                            None
                        } else {
                            Some((raw, blocks, temp, model_name))
                        }
                    }
                    Err(_) => None,
                }
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    results
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, (raw, blocks, temperature, model))| Candidate {
            index,
            raw,
            blocks,
            temperature,
            model,
        })
        .collect()
}

/// One reflexion repair attempt: a single sample conditioned on the failure log.
pub async fn repair_once(
    provider: &dyn ChatProvider,
    model: &ModelRef,
    cfg: &Config,
    system: String,
    user: String,
    temperature: f32,
    default_path: Option<String>,
) -> Result<Option<Candidate>> {
    let req = ChatRequest {
        model: model.clone(),
        messages: vec![Message::system(system), Message::user(user)],
        temperature,
        max_tokens: cfg.scaling.max_tokens,
    };
    let raw = provider.complete(req).await?;
    let blocks = parse_blocks_fallback(&raw, default_path.as_deref());
    if blocks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Candidate {
        index: 0,
        raw,
        blocks,
        temperature,
        model: model.to_string(),
    }))
}
