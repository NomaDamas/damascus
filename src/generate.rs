//! Best-of-N candidate generation — the test-time-scaling lever. We spend extra
//! inference (multiple samples at rising temperatures, optionally across an
//! ensemble of models) instead of relying on one strong model.

use anyhow::Result;
use futures::stream::{self, StreamExt};

use crate::config::Config;
use crate::edits::{parse_blocks, EditBlock};
use crate::provider::{ChatProvider, ChatRequest, Message, ModelRef};

/// A single generated candidate for a step.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub index: usize,
    pub raw: String,
    pub blocks: Vec<EditBlock>,
    pub temperature: f32,
}

/// Generate `n` candidates concurrently for the given drafter messages. Each
/// sample uses a slightly higher temperature for diversity. Samples whose output
/// contains no parseable edit blocks are dropped (they cannot be verified).
pub async fn sample_candidates(
    provider: &dyn ChatProvider,
    model: &ModelRef,
    cfg: &Config,
    system: String,
    user: String,
    n: usize,
) -> Vec<Candidate> {
    let temps = cfg.scaling.track_temperatures(n);
    let concurrency = cfg.scaling.concurrency.max(1);
    let max_tokens = cfg.scaling.max_tokens;

    let results: Vec<Option<(String, Vec<EditBlock>, f32)>> = stream::iter(temps)
        .map(|temp| {
            let req = ChatRequest {
                model: model.clone(),
                messages: vec![Message::system(system.clone()), Message::user(user.clone())],
                temperature: temp,
                max_tokens,
            };
            async move {
                match provider.complete(req).await {
                    Ok(raw) => match parse_blocks(&raw) {
                        Ok(blocks) if !blocks.is_empty() => Some((raw, blocks, temp)),
                        _ => None,
                    },
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
        .map(|(index, (raw, blocks, temperature))| Candidate {
            index,
            raw,
            blocks,
            temperature,
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
) -> Result<Option<Candidate>> {
    let req = ChatRequest {
        model: model.clone(),
        messages: vec![Message::system(system), Message::user(user)],
        temperature,
        max_tokens: cfg.scaling.max_tokens,
    };
    let raw = provider.complete(req).await?;
    let blocks = parse_blocks(&raw).unwrap_or_default();
    if blocks.is_empty() {
        return Ok(None);
    }
    Ok(Some(Candidate {
        index: 0,
        raw,
        blocks,
        temperature,
    }))
}
