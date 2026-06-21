//! The deterministic, multi-stage candidate filter.
//!
//! Dozens of candidates pour in from the high-throughput generator. We must
//! reject the garbage *fast* and *without an LLM*. Stages run cheapest-first:
//!
//!   Stage 1 — Syntax: parse each changed file with tree-sitter; reject on error.
//!   Stage 2 — Contract: enforce micro-patch scope and signature preservation.
//!   Stage 3 — Verify: only survivors pay for a sandboxed build/test (in the
//!             orchestrator). This module owns stages 1 and 2.

use crate::ast::{self, Lang};
use crate::edits::{compute_changes, Changes, EditBlock};
use std::path::Path;

/// What the leaf node permits a candidate to do. This is how the harness keeps a
/// weak model from "freely roaming" the codebase.
#[derive(Debug, Clone, Default)]
pub struct Contract {
    /// Files the candidate is allowed to touch. Empty = no restriction.
    pub allowed_files: Vec<String>,
    /// `(file, symbol)` that must still exist after the edit.
    pub require_symbol: Option<(String, String)>,
    /// A signature substring (whitespace-normalized) that must be preserved.
    pub keep_signature: Option<String>,
}

impl Contract {
    pub fn unrestricted() -> Self {
        Contract::default()
    }
}

/// Outcome of the pre-sandbox stages.
#[derive(Debug)]
pub enum Prefilter {
    /// Passed stages 1 and 2; carries the computed changes for stage 3.
    Pass(Changes),
    RejectApply(String),
    RejectSyntax(String),
    RejectScope(String),
    RejectContract(String),
}

impl Prefilter {
    pub fn passed(&self) -> bool {
        matches!(self, Prefilter::Pass(_))
    }
    /// Short human reason for logs/repair feedback.
    pub fn reason(&self) -> String {
        match self {
            Prefilter::Pass(_) => "pass".into(),
            Prefilter::RejectApply(m) => format!("apply: {m}"),
            Prefilter::RejectSyntax(f) => format!("syntax error in {f}"),
            Prefilter::RejectScope(f) => format!("edits out-of-scope file {f}"),
            Prefilter::RejectContract(m) => format!("contract: {m}"),
        }
    }
}

/// Run stages 1 and 2 against a candidate's edit blocks.
pub fn prefilter(root: &Path, blocks: &[EditBlock], contract: &Contract) -> Prefilter {
    // Apply in memory (also validates patch applicability — a cheap form of reject).
    let changes = match compute_changes(root, blocks) {
        Ok(c) => c,
        Err(e) => return Prefilter::RejectApply(e.to_string()),
    };

    // Stage 2a — scope: no edits outside the allowed set.
    if !contract.allowed_files.is_empty() {
        for path in changes.contents.keys() {
            if !contract.allowed_files.iter().any(|a| a == path) {
                return Prefilter::RejectScope(path.clone());
            }
        }
    }

    // Stage 1 — syntax: every changed file in a known language must parse.
    for (path, content) in &changes.contents {
        if let Some(lang) = Lang::from_path(Path::new(path)) {
            if ast::has_syntax_errors(lang, content) {
                return Prefilter::RejectSyntax(path.clone());
            }
        }
    }

    // Stage 2b — symbol preservation: target definition still present.
    if let Some((file, name)) = &contract.require_symbol {
        match changes.contents.get(file) {
            Some(content) => {
                let lang = Lang::from_path(Path::new(file));
                if let Some(lang) = lang {
                    let syms = ast::symbols(lang, content);
                    if ast::find_symbol(&syms, name).is_none() {
                        return Prefilter::RejectContract(format!("symbol `{name}` was removed"));
                    }
                }
            }
            None => {
                // The target file wasn't changed — acceptable only if it already
                // satisfies the contract; treat as no-op rejection to force progress.
                return Prefilter::RejectContract(format!("target file `{file}` not modified"));
            }
        }
    }

    // Stage 2c — signature preservation (normalized: benign edits like adding
    // `mut` to a parameter or whitespace changes are allowed; name/types/return
    // changes are not).
    if let Some(sig) = &contract.keep_signature {
        let want = normalize_sig(sig);
        // Prefer comparing the re-extracted target signature when we know it.
        let ok = if let Some((file, name)) = &contract.require_symbol {
            match changes.contents.get(file).and_then(|c| {
                Lang::from_path(Path::new(file)).map(|lang| (ast::symbols(lang, c), c))
            }) {
                Some((syms, _)) => match ast::find_symbol(&syms, name) {
                    Some(s) if normalize_sig(&s.signature) == want => true,
                    Some(s) => {
                        return Prefilter::RejectContract(format!(
                            "target signature changed: got `{}`",
                            s.signature.trim()
                        ))
                    }
                    None => false,
                },
                None => true, // unsupported language: don't block
            }
        } else {
            changes
                .contents
                .values()
                .any(|c| normalize_sig(c).contains(&want))
        };
        if !ok {
            return Prefilter::RejectContract("target signature was changed".into());
        }
    }

    Prefilter::Pass(changes)
}

/// Normalize a signature for comparison: collapse whitespace and drop benign
/// modifiers (`mut`, `pub`) that don't change the call contract.
fn normalize_sig(s: &str) -> String {
    // Canonical form: drop the identifiers `mut`/`pub` (whole words, even when
    // glued to punctuation like `merge(mut`), drop all whitespace, keep
    // everything else. So `pub fn f(mut x: Vec<(i32,i32)>)` and
    // `fn f(x: Vec<(i32, i32)>)` compare equal.
    let mut out = String::with_capacity(s.len());
    let mut ident = String::new();
    fn flush(ident: &mut String, out: &mut String) {
        if !ident.is_empty() {
            if ident != "mut" && ident != "pub" {
                out.push_str(ident);
            }
            ident.clear();
        }
    }
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
        } else {
            flush(&mut ident, &mut out);
            if !c.is_whitespace() {
                out.push(c);
            }
        }
    }
    flush(&mut ident, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn block(path: &str, search: &str, replace: &str) -> EditBlock {
        EditBlock {
            path: path.into(),
            search: search.into(),
            replace: replace.into(),
        }
    }

    #[test]
    fn rejects_syntax_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() -> i32 { 1 }\n").unwrap();
        let blocks = vec![block("a.rs", "1 }", "1 ")]; // drops closing brace
        let r = prefilter(dir.path(), &blocks, &Contract::unrestricted());
        assert!(matches!(r, Prefilter::RejectSyntax(_)), "{}", r.reason());
    }

    #[test]
    fn passes_valid_edit() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() -> i32 { 1 }\n").unwrap();
        let blocks = vec![block("a.rs", "1 }", "42 }")];
        let r = prefilter(dir.path(), &blocks, &Contract::unrestricted());
        assert!(r.passed(), "{}", r.reason());
    }

    #[test]
    fn enforces_scope() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "fn b() {}\n").unwrap();
        let blocks = vec![block("b.rs", "fn b() {}", "fn b() { let x = 1; }")];
        let contract = Contract {
            allowed_files: vec!["a.rs".into()],
            ..Default::default()
        };
        let r = prefilter(dir.path(), &blocks, &contract);
        assert!(matches!(r, Prefilter::RejectScope(_)), "{}", r.reason());
    }

    #[test]
    fn enforces_symbol_preservation() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn keep() -> i32 { 0 }\n").unwrap();
        // Replace the whole function with a differently-named one.
        let blocks = vec![block(
            "a.rs",
            "fn keep() -> i32 { 0 }",
            "fn gone() -> i32 { 1 }",
        )];
        let contract = Contract {
            require_symbol: Some(("a.rs".into(), "keep".into())),
            ..Default::default()
        };
        let r = prefilter(dir.path(), &blocks, &contract);
        assert!(matches!(r, Prefilter::RejectContract(_)), "{}", r.reason());
    }

    #[test]
    fn keeps_signature() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn f(x: i32) -> i32 { x }\n").unwrap();
        let blocks = vec![block("a.rs", "{ x }", "{ x + 1 }")];
        let contract = Contract {
            keep_signature: Some("fn f(x: i32) -> i32".into()),
            ..Default::default()
        };
        assert!(prefilter(dir.path(), &blocks, &contract).passed());

        let bad = vec![block(
            "a.rs",
            "fn f(x: i32) -> i32 { x }",
            "fn f(x: i64) -> i64 { x }",
        )];
        let r = prefilter(dir.path(), &bad, &contract);
        assert!(matches!(r, Prefilter::RejectContract(_)), "{}", r.reason());
    }
    #[test]
    fn signature_allows_mut_and_spacing() {
        // Adding `mut` to a param and changing inner spacing must be allowed;
        // only a real signature change (types/return/name) is rejected.
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "pub fn merge(intervals: Vec<(i32, i32)>) -> Vec<(i32, i32)> { intervals }\n",
        )
        .unwrap();
        let contract = Contract {
            require_symbol: Some(("a.rs".into(), "merge".into())),
            keep_signature: Some(
                "pub fn merge(intervals: Vec<(i32, i32)>) -> Vec<(i32, i32)>".into(),
            ),
            ..Default::default()
        };
        let ok = vec![block(
            "a.rs",
            "pub fn merge(intervals: Vec<(i32, i32)>) -> Vec<(i32, i32)> { intervals }",
            "pub fn merge(mut intervals: Vec<(i32,i32)>) -> Vec<(i32, i32)> { intervals.sort(); intervals }",
        )];
        assert!(prefilter(dir.path(), &ok, &contract).passed());
    }

    #[test]
    fn normalize_sig_canonicalizes() {
        assert_eq!(
            normalize_sig("pub fn merge(mut x: Vec<(i32,i32)>)"),
            normalize_sig("fn merge(x: Vec<(i32, i32)>)")
        );
    }
}
