//! Configuration: providers, model roles, and the test-time-scaling knobs that
//! turn modest models into reliable ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::provider::ModelRef;

/// File name Damascus looks for, in the working directory then `~/.config`.
pub const CONFIG_FILE: &str = "damascus.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Named providers; the key is referenced by `models.*` as `provider/model`.
    pub providers: HashMap<String, ProviderConfig>,
    pub models: ModelRoles,
    #[serde(default)]
    pub scaling: ScalingConfig,
    #[serde(default)]
    pub verify: VerifyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// e.g. `https://api.openai.com/v1` or `http://localhost:11434/v1`.
    pub base_url: String,
    /// Environment variable that holds the API key (preferred over inline).
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// Inline key. Discouraged; use `api_key_env`.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
}

impl ProviderConfig {
    /// Resolve the API key: inline first, then the named env var. `None` is
    /// valid (many local servers need no key).
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Some(k) = &self.api_key {
            if !k.is_empty() {
                return Some(k.clone());
            }
        }
        if let Some(var) = &self.api_key_env {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoles {
    /// Decomposes a task into atomic, individually verifiable steps.
    pub planner: String,
    /// The cheap workhorse: generates candidate edits (invoked N times).
    pub drafter: String,
    /// Selects between passing candidates and runs the final critique.
    pub judge: String,
    /// Produces fixes during the reflexion repair loop.
    pub repairer: String,
    /// Optional ensemble: a pool of models to spread best-of-N generation across.
    /// When set and non-empty, overrides `drafter` for candidate generation, so a
    /// problem one model can't solve may be solved by another in the pool.
    #[serde(default)]
    pub drafters: Option<Vec<String>>,
}

impl ModelRoles {
    pub fn planner_ref(&self) -> Result<ModelRef> {
        ModelRef::parse(&self.planner)
    }
    pub fn drafter_ref(&self) -> Result<ModelRef> {
        ModelRef::parse(&self.drafter)
    }
    pub fn judge_ref(&self) -> Result<ModelRef> {
        ModelRef::parse(&self.judge)
    }
    pub fn repairer_ref(&self) -> Result<ModelRef> {
        ModelRef::parse(&self.repairer)
    }
    /// The generation model pool: the `drafters` ensemble if set, else `[drafter]`.
    pub fn drafter_pool(&self) -> Result<Vec<ModelRef>> {
        match &self.drafters {
            Some(list) if !list.is_empty() => list.iter().map(|m| ModelRef::parse(m)).collect(),
            _ => Ok(vec![self.drafter_ref()?]),
        }
    }
}

/// The levers that trade inference for quality.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScalingConfig {
    /// Best-of-N: candidate edit-sets generated per step.
    pub candidates: usize,
    /// Reflexion repair rounds attempted when no candidate passes.
    pub repair_rounds: usize,
    /// How deep a stubborn step may be recursively re-atomized.
    pub max_recursion: usize,
    /// Global cap on executed steps (runaway guard).
    pub max_steps: usize,
    /// Base sampling temperature for the drafter.
    pub temperature: f32,
    /// Added per extra candidate to diversify samples.
    pub temperature_step: f32,
    /// Optional per-call output token cap.
    pub max_tokens: Option<u32>,
    /// Max concurrent model requests in flight (the throughput knob).
    pub concurrency: usize,
    /// Exploration-track temperature for the high-temp half of the rollout.
    pub explore_temperature: f32,
}

impl Default for ScalingConfig {
    fn default() -> Self {
        ScalingConfig {
            candidates: 6,
            repair_rounds: 2,
            max_recursion: 2,
            max_steps: 40,
            temperature: 0.3,
            temperature_step: 0.2,
            max_tokens: None,
            concurrency: default_concurrency(),
            explore_temperature: default_explore_temp(),
        }
    }
}

impl ScalingConfig {
    /// Temperature for the i-th candidate (0-based), clamped to a sane ceiling.
    pub fn temperature_for(&self, i: usize) -> f32 {
        (self.temperature + self.temperature_step * i as f32).min(1.3)
    }

    /// Two-track schedule: the lower half of the rollout exploits around
    /// `temperature` (focused), the upper half explores around
    /// `explore_temperature` (diverse). Returns one temperature per candidate.
    pub fn track_temperatures(&self, n: usize) -> Vec<f32> {
        if n == 0 {
            return Vec::new();
        }
        let focus = n.div_ceil(2);
        (0..n)
            .map(|i| {
                if i < focus {
                    (self.temperature + self.temperature_step * i as f32).min(1.1)
                } else {
                    let j = i - focus;
                    (self.explore_temperature + 0.1 * j as f32).min(1.5)
                }
            })
            .collect()
    }
}

/// Objective verification gates run against every candidate, in this order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyConfig {
    /// e.g. `cargo build` / `npm run build`. Empty = skipped.
    #[serde(default)]
    pub build: Option<String>,
    /// e.g. `cargo test` / `pytest`. Used as the default acceptance check.
    #[serde(default)]
    pub test: Option<String>,
    /// e.g. `cargo clippy -- -D warnings`. Empty = skipped.
    #[serde(default)]
    pub lint: Option<String>,
    /// Per-command timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    600
}

fn default_concurrency() -> usize {
    8
}

fn default_explore_temp() -> f32 {
    0.9
}

impl Default for VerifyConfig {
    fn default() -> Self {
        VerifyConfig {
            build: None,
            test: None,
            lint: None,
            timeout_secs: default_timeout(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Find config in CWD or `~/.config/damascus/damascus.toml`.
    pub fn discover() -> Result<(Self, PathBuf)> {
        let local = PathBuf::from(CONFIG_FILE);
        if local.exists() {
            return Ok((Config::load(&local)?, local));
        }
        if let Some(dir) = dirs::config_dir() {
            let p = dir.join("damascus").join(CONFIG_FILE);
            if p.exists() {
                return Ok((Config::load(&p)?, p));
            }
        }
        Err(anyhow!(
            "no {CONFIG_FILE} found in CWD or ~/.config/damascus/. Run `damascus init`."
        ))
    }

    pub fn validate(&self) -> Result<()> {
        for role in [
            &self.models.planner,
            &self.models.drafter,
            &self.models.judge,
            &self.models.repairer,
        ] {
            let m = ModelRef::parse(role)?;
            if !self.providers.contains_key(&m.provider) {
                return Err(anyhow!(
                    "model `{role}` references provider `{}` which is not defined in [providers]",
                    m.provider
                ));
            }
        }
        for m in self.models.drafter_pool()? {
            if !self.providers.contains_key(&m.provider) {
                return Err(anyhow!(
                    "drafters ensemble references provider `{}` which is not defined in [providers]",
                    m.provider
                ));
            }
        }
        if self.scaling.candidates == 0 {
            return Err(anyhow!("scaling.candidates must be >= 1"));
        }
        Ok(())
    }

    /// A ready-to-edit starter config aimed at local / open-source models.
    pub fn template() -> String {
        r#"# Damascus configuration.
# Point every role at a cheap, local, or open-source model. Damascus reaches
# frontier-grade results by spending inference (best-of-N + repair + decomposition)
# and gating every change behind objective verification — not by using a big model.

[providers.local]
# Ollama / llama.cpp / vLLM all expose an OpenAI-compatible endpoint.
base_url = "http://localhost:11434/v1"
api_key_env = "OLLAMA_API_KEY"   # usually unused locally

[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[providers.google]
# Google AI Studio exposes an OpenAI-compatible path.
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key_env = "GEMINI_API_KEY"

[providers.openai]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[models]
# Mix providers freely; ensemble diversity improves candidate selection.
planner  = "local/qwen2.5-coder:7b"
drafter  = "local/qwen2.5-coder:7b"
judge    = "local/qwen2.5-coder:7b"
repairer = "local/qwen2.5-coder:7b"
# Optional ensemble: spread best-of-N generation across several models. A problem
# one model can't solve may be solved by another — orchestration as a quality lever.
# drafters = ["openrouter/nvidia/nemotron-3-ultra-550b-a55b", "openrouter/openai/gpt-oss-120b", "openrouter/google/gemma-4-31b-it"]

[scaling]
candidates = 8        # high-throughput best-of-N: sample many, let the filter pick
repair_rounds = 2     # reflexion retries when nothing passes
max_recursion = 2     # how deep a hard step may be re-decomposed
max_steps = 40        # global runaway guard
temperature = 0.3     # focus-track base temperature
temperature_step = 0.2   # focus-track ramp per candidate
explore_temperature = 0.9  # explore-track temperature (the diverse half)
concurrency = 8       # max model requests in flight (throughput knob)

[verify]
# These are the forcing functions. A change is accepted only if they pass.
build = "cargo build"
test  = "cargo test"
lint  = "cargo clippy -- -D warnings"
timeout_secs = 600
"#
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> &'static str {
        r#"
[providers.local]
base_url = "http://localhost:11434/v1"

[models]
planner = "local/m"
drafter = "local/m"
judge = "local/m"
repairer = "local/m"
"#
    }

    #[test]
    fn parses_and_defaults() {
        let cfg: Config = toml::from_str(sample()).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.scaling.candidates, 6);
        assert_eq!(cfg.verify.timeout_secs, 600);
    }

    #[test]
    fn rejects_unknown_provider() {
        let bad = sample().replace("local/m", "ghost/m");
        let cfg: Config = toml::from_str(&bad).unwrap();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn temperature_ramps_and_clamps() {
        let s = ScalingConfig::default();
        assert!((s.temperature_for(0) - 0.3).abs() < 1e-6);
        assert!(s.temperature_for(100) <= 1.3);
    }

    #[test]
    fn two_track_temps_split_focus_and_explore() {
        let s = ScalingConfig::default();
        let temps = s.track_temperatures(6);
        assert_eq!(temps.len(), 6);
        // focus half starts at base temperature
        assert!((temps[0] - 0.3).abs() < 1e-6);
        // explore half is hotter than the focus base
        assert!(temps[5] >= s.explore_temperature);
        assert!(temps.iter().all(|t| *t <= 1.5));
    }

    #[test]
    fn template_is_valid() {
        let cfg: Config = toml::from_str(&Config::template()).unwrap();
        cfg.validate().unwrap();
    }
}
