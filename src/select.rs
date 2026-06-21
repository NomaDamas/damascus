//! Selection among candidates that already passed verification.
//!
//! Ordering: fewest diagnostics → smallest diff → LLM judge tie-break. The judge
//! is only consulted when objective signals tie, keeping cost low and decisions
//! mostly deterministic.

use crate::config::Config;
use crate::generate::Candidate;
use crate::plan::Step;
use crate::prompts;
use crate::provider::{ChatProvider, ChatRequest, Message};
use crate::verify::Verdict;

/// A candidate that passed verification, with the data needed to rank it.
pub struct Scored {
    pub candidate: Candidate,
    pub verdict: Verdict,
    pub touched_lines: usize,
}

/// Pick the best passing candidate. Returns the index into `scored`.
pub async fn select_best(
    provider: &dyn ChatProvider,
    cfg: &Config,
    task: &str,
    step: &Step,
    scored: &[Scored],
) -> usize {
    assert!(!scored.is_empty(), "select_best requires >= 1 candidate");
    if scored.len() == 1 {
        return 0;
    }

    // Objective ranking first.
    let min_diag = scored.iter().map(|s| s.verdict.diagnostics).min().unwrap();
    let mut front: Vec<usize> = (0..scored.len())
        .filter(|&i| scored[i].verdict.diagnostics == min_diag)
        .collect();

    if front.len() > 1 {
        let min_lines = front
            .iter()
            .map(|&i| scored[i].touched_lines)
            .min()
            .unwrap();
        front.retain(|&i| scored[i].touched_lines == min_lines);
    }

    if front.len() == 1 {
        return front[0];
    }

    // Tie-break with the judge model; fall back to the first on any failure.
    let texts: Vec<String> = front
        .iter()
        .map(|&i| scored[i].candidate.raw.clone())
        .collect();
    if let Ok(model) = cfg.models.judge_ref() {
        let req = ChatRequest {
            model,
            messages: vec![
                Message::system(prompts::judge_system()),
                Message::user(prompts::judge_user(task, step, &texts)),
            ],
            temperature: 0.0,
            max_tokens: Some(16),
        };
        if let Ok(ans) = provider.complete(req).await {
            if let Some(local) = parse_index(&ans, front.len()) {
                return front[local];
            }
        }
    }
    front[0]
}

/// Parse a leading integer index from the judge reply, bounded by `len`.
fn parse_index(s: &str, len: usize) -> Option<usize> {
    let digits: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let n: usize = digits.parse().ok()?;
    if n < len {
        Some(n)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::EditBlock;
    use crate::verify::Verdict;

    fn cand(index: usize) -> Candidate {
        Candidate {
            index,
            raw: format!("cand{index}"),
            blocks: vec![EditBlock {
                path: "a".into(),
                search: "".into(),
                replace: "x".into(),
            }],
            temperature: 0.4,
        }
    }
    fn verdict(diag: usize) -> Verdict {
        Verdict {
            passed: true,
            gates: vec![],
            diagnostics: diag,
        }
    }

    struct NoProvider;
    impl ChatProvider for NoProvider {
        fn complete<'a>(&'a self, _req: ChatRequest) -> crate::provider::ChatFuture<'a> {
            Box::pin(async { Err(anyhow::anyhow!("judge unavailable")) })
        }
    }

    #[tokio::test]
    async fn prefers_fewest_diagnostics() {
        let cfg: Config = toml::from_str(
            "[providers.local]\nbase_url=\"x\"\n[models]\nplanner=\"local/m\"\ndrafter=\"local/m\"\njudge=\"local/m\"\nrepairer=\"local/m\"",
        )
        .unwrap();
        let scored = vec![
            Scored {
                candidate: cand(0),
                verdict: verdict(5),
                touched_lines: 2,
            },
            Scored {
                candidate: cand(1),
                verdict: verdict(1),
                touched_lines: 9,
            },
        ];
        let step = Step {
            title: "t".into(),
            detail: "".into(),
            check: None,
            ..Default::default()
        };
        let best = select_best(&NoProvider, &cfg, "task", &step, &scored).await;
        assert_eq!(best, 1);
    }

    #[tokio::test]
    async fn breaks_diag_tie_by_diff_size() {
        let cfg: Config = toml::from_str(
            "[providers.local]\nbase_url=\"x\"\n[models]\nplanner=\"local/m\"\ndrafter=\"local/m\"\njudge=\"local/m\"\nrepairer=\"local/m\"",
        )
        .unwrap();
        let scored = vec![
            Scored {
                candidate: cand(0),
                verdict: verdict(0),
                touched_lines: 20,
            },
            Scored {
                candidate: cand(1),
                verdict: verdict(0),
                touched_lines: 3,
            },
        ];
        let step = Step {
            title: "t".into(),
            detail: "".into(),
            check: None,
            ..Default::default()
        };
        let best = select_best(&NoProvider, &cfg, "task", &step, &scored).await;
        assert_eq!(best, 1);
    }

    #[test]
    fn parse_index_bounds() {
        assert_eq!(parse_index("0", 2), Some(0));
        assert_eq!(parse_index("1 is best", 2), Some(1));
        assert_eq!(parse_index("5", 2), None);
        assert_eq!(parse_index("none", 2), None);
    }
}
