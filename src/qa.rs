//! Repository question‑answering (the AA "RepoQnA" family).
//!
//! Damascus is otherwise an *implementation* harness; this read‑only mode adds
//! the third coding‑agent capability: answer questions about a codebase. It also
//! builds the **retrieval / localization** layer (over the tree‑sitter
//! `RepoIndex`) that repo‑scale implementation work will reuse.
//!
//! Test‑time scaling still applies: we sample N answers and let a judge select
//! the best (self‑consistency), rather than trusting one shot.

use anyhow::{anyhow, Result};
use futures::future::join_all;

use crate::config::Config;
use crate::provider::{ChatProvider, ChatRequest, Message};
use crate::slice::RepoIndex;

const MAX_FILES: usize = 6;
const MAX_CHARS: usize = 12_000;
const FULL_FILE_LINES: usize = 120;

pub struct QaResult {
    pub answer: String,
    pub sources: Vec<String>,
}

/// Tokenize text into lowercased identifier/word tokens (>=3 chars).
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c.to_ascii_lowercase());
        } else {
            if cur.len() >= 3 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.len() >= 3 {
        out.push(cur);
    }
    out
}

/// Rank files by relevance to the question (symbol-name and path-token overlap),
/// then build a bounded context plus the cited source list. This is the
/// localization step — the same machinery repo-scale editing needs.
pub fn retrieve(index: &RepoIndex, question: &str) -> (String, Vec<String>) {
    let q: std::collections::BTreeSet<String> = tokens(question).into_iter().collect();
    let mut scored: Vec<(i32, String)> = Vec::new();
    for path in index.file_paths() {
        let mut score = 0i32;
        for t in tokens(path) {
            if q.contains(&t) {
                score += 2;
            }
        }
        for sym in index.symbols_in(path) {
            if q.contains(&sym.name.to_ascii_lowercase()) {
                score += 4;
            }
        }
        if score > 0 {
            scored.push((score, path.to_string()));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.truncate(MAX_FILES);

    let mut ctx = String::new();
    let mut sources = Vec::new();
    for (rank, (_score, path)) in scored.iter().enumerate() {
        sources.push(path.clone());
        ctx.push_str(&format!("\n===== {path} =====\n"));
        if rank < 2 {
            // Top hits: include the file (truncated).
            if let Some(src) = index.file_source(path) {
                ctx.push_str(&truncate_lines(src, FULL_FILE_LINES));
            }
        } else {
            // Others: just the symbol signatures (cheap structural summary).
            for sym in index.symbols_in(path) {
                ctx.push_str(&sym.signature);
                ctx.push('\n');
            }
        }
        if ctx.len() > MAX_CHARS {
            ctx.truncate(nearest_boundary(&ctx, MAX_CHARS));
            ctx.push_str("\n… (context truncated)\n");
            break;
        }
    }
    if sources.is_empty() {
        ctx = "(no obviously relevant files found by retrieval)".to_string();
    }
    (ctx, sources)
}

fn truncate_lines(s: &str, max: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\n… ({} more lines)",
            lines[..max].join("\n"),
            lines.len() - max
        )
    }
}

fn nearest_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn qa_system() -> String {
    "You are a precise software engineer answering a question about a specific codebase. \
     Use ONLY the provided code context. Be concrete and cite file paths. If the context \
     is insufficient, say what is missing rather than guessing."
        .to_string()
}

fn qa_user(question: &str, context: &str) -> String {
    format!("Question: {question}\n\nCode context:\n{context}\n\nAnswer the question using the context above.")
}

/// Answer `question` about the repo: retrieve, sample N answers (self-consistency),
/// and judge-select the best. Uses the judge role model for both.
pub async fn answer(
    provider: &dyn ChatProvider,
    cfg: &Config,
    index: &RepoIndex,
    question: &str,
    n: usize,
) -> Result<QaResult> {
    let model = cfg.models.judge_ref()?;
    let (context, sources) = retrieve(index, question);
    let system = qa_system();
    let user = qa_user(question, &context);

    let n = n.max(1);
    let futures = (0..n).map(|i| {
        let req = ChatRequest {
            model: model.clone(),
            messages: vec![Message::system(system.clone()), Message::user(user.clone())],
            temperature: if n == 1 { 0.1 } else { 0.2 + 0.15 * i as f32 },
            max_tokens: cfg.scaling.max_tokens,
        };
        async move { provider.complete(req).await.ok() }
    });
    let answers: Vec<String> = join_all(futures)
        .await
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if answers.is_empty() {
        return Err(anyhow!("model returned no answer"));
    }
    if answers.len() == 1 {
        return Ok(QaResult {
            answer: answers.into_iter().next().unwrap(),
            sources,
        });
    }

    // Self-consistency: judge selects the best answer.
    let best = judge_select(provider, &model, question, &answers)
        .await
        .unwrap_or(0);
    Ok(QaResult {
        answer: answers.into_iter().nth(best).unwrap(),
        sources,
    })
}

async fn judge_select(
    provider: &dyn ChatProvider,
    model: &crate::provider::ModelRef,
    question: &str,
    answers: &[String],
) -> Option<usize> {
    let mut prompt = format!(
        "Question: {question}\n\nSeveral candidate answers follow. Pick the single most correct, \
         complete, and well-grounded one. Reply with ONLY its integer index.\n\n"
    );
    for (i, a) in answers.iter().enumerate() {
        prompt.push_str(&format!(
            "=== Answer {i} ===\n{}\n\n",
            truncate_lines(a, 60)
        ));
    }
    let req = ChatRequest {
        model: model.clone(),
        messages: vec![Message::user(prompt)],
        temperature: 0.0,
        max_tokens: Some(8),
    };
    let resp = provider.complete(req).await.ok()?;
    let digits: String = resp
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let idx: usize = digits.parse().ok()?;
    if idx < answers.len() {
        Some(idx)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(root: &std::path::Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn tokens_filters_short() {
        let t = tokens("How does parse_blocks work?");
        assert!(t.contains(&"parse_blocks".to_string()));
        assert!(t.contains(&"does".to_string()));
        assert!(!t.iter().any(|w| w.len() < 3));
    }

    #[test]
    fn retrieve_ranks_by_symbol_match() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            "src/parser.rs",
            "pub fn parse_blocks() {}\npub fn helper() {}\n",
        );
        write(dir.path(), "src/other.rs", "pub fn unrelated() {}\n");
        let idx = RepoIndex::build(dir.path());
        let (ctx, sources) = retrieve(&idx, "where is parse_blocks implemented?");
        assert_eq!(sources.first().map(|s| s.as_str()), Some("src/parser.rs"));
        assert!(ctx.contains("parse_blocks"));
    }

    #[test]
    fn retrieve_empty_on_no_match() {
        let dir = tempdir().unwrap();
        write(dir.path(), "a.rs", "fn x() {}\n");
        let (_ctx, sources) = retrieve(&RepoIndex::build(dir.path()), "quantum chromodynamics");
        assert!(sources.is_empty());
    }

    struct MockQa;
    impl ChatProvider for MockQa {
        fn complete<'a>(&'a self, req: ChatRequest) -> crate::provider::ChatFuture<'a> {
            let last = req
                .messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            Box::pin(async move {
                if last.contains("integer index") {
                    Ok("1".to_string())
                } else {
                    Ok("parse_blocks is in src/parser.rs".to_string())
                }
            })
        }
    }

    #[tokio::test]
    async fn answer_uses_retrieval_and_judge() {
        let dir = tempdir().unwrap();
        write(dir.path(), "src/parser.rs", "pub fn parse_blocks() {}\n");
        let cfg: Config = toml::from_str(
            "[providers.local]\nbase_url=\"x\"\n[models]\nplanner=\"local/m\"\ndrafter=\"local/m\"\njudge=\"local/m\"\nrepairer=\"local/m\"",
        )
        .unwrap();
        let idx = RepoIndex::build(dir.path());
        let r = answer(&MockQa, &cfg, &idx, "where is parse_blocks?", 3)
            .await
            .unwrap();
        assert!(r.answer.contains("parse_blocks"));
        assert_eq!(r.sources, vec!["src/parser.rs".to_string()]);
    }
}
