//! Task decomposition (the "atomize" stage). Smaller atoms => higher per-step
//! success rate for modest models.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::prompts;
use crate::provider::{ChatProvider, ChatRequest, Message};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    /// Optional per-step acceptance command (exit 0 == pass). Overrides the
    /// global test gate for this step when present.
    #[serde(default)]
    pub check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Plan {
    pub steps: Vec<Step>,
}

/// Ask the planner model to decompose `task`. Falls back to a single step if the
/// model output cannot be parsed — the loop still works, just with bigger atoms.
pub async fn make_plan(
    provider: &dyn ChatProvider,
    cfg: &Config,
    task: &str,
    repo_summary: &str,
) -> Result<Plan> {
    let model = cfg.models.planner_ref()?;
    let req = ChatRequest {
        model,
        messages: vec![
            Message::system(prompts::planner_system()),
            Message::user(prompts::planner_user(
                task,
                repo_summary,
                cfg.verify.test.is_some(),
            )),
        ],
        temperature: 0.2,
        max_tokens: cfg.scaling.max_tokens,
    };
    let raw = provider.complete(req).await?;
    Ok(parse_plan(&raw).unwrap_or_else(|| Plan {
        steps: vec![Step {
            title: task.chars().take(80).collect(),
            detail: task.to_string(),
            check: None,
        }],
    }))
}

/// Extract a JSON array of steps from arbitrary model text.
pub fn parse_plan(text: &str) -> Option<Plan> {
    let json = extract_json_array(text)?;
    let steps: Vec<Step> = serde_json::from_str(&json).ok()?;
    if steps.is_empty() {
        return None;
    }
    // Normalize blank checks to None.
    let steps = steps
        .into_iter()
        .map(|mut s| {
            if let Some(c) = &s.check {
                if c.trim().is_empty() || c.trim().eq_ignore_ascii_case("null") {
                    s.check = None;
                }
            }
            s
        })
        .collect();
    Some(Plan { steps })
}

/// Grab the first balanced `[ ... ]` span (handles strings/escapes).
fn extract_json_array(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = text.find('[')?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clean_json() {
        let t = r#"[{"title":"a","detail":"d","check":"cargo test"}]"#;
        let p = parse_plan(t).unwrap();
        assert_eq!(p.steps.len(), 1);
        assert_eq!(p.steps[0].check.as_deref(), Some("cargo test"));
    }

    #[test]
    fn parses_json_wrapped_in_prose() {
        let t = "Sure! Here is the plan:\n```json\n[{\"title\":\"x\"}]\n```\nGood luck.";
        let p = parse_plan(t).unwrap();
        assert_eq!(p.steps[0].title, "x");
        assert_eq!(p.steps[0].check, None);
    }

    #[test]
    fn normalizes_blank_check() {
        let t = r#"[{"title":"x","check":"  "}]"#;
        let p = parse_plan(t).unwrap();
        assert_eq!(p.steps[0].check, None);
    }

    #[test]
    fn returns_none_for_garbage() {
        assert!(parse_plan("no json here").is_none());
    }

    #[test]
    fn handles_brackets_inside_strings() {
        let t = r#"[{"title":"fix [edge] case","detail":"a]b"}]"#;
        let p = parse_plan(t).unwrap();
        assert_eq!(p.steps[0].title, "fix [edge] case");
    }
}
