//! Hierarchical Goal Tree manager.
//!
//! The harness owns the plan tree; the model never edits it. For each leaf the
//! manager hands the model a *tight* context (an AST slice when possible) and a
//! *contract* that mechanically bounds what a valid micro-patch may do. The
//! filter enforces the contract; only a passing patch is merged and the tree
//! advances. This is how a weak model's poor planning is caged by system rules.

use crate::filter::Contract;
use crate::plan::Step;
use crate::slice::{RepoIndex, DEFAULT_MAX_CHARS};

/// The materialized instructions for one leaf: what the model sees and the rules
/// its output must satisfy.
pub struct LeafPlan {
    /// The (small) context handed to the model.
    pub context: String,
    /// The contract the candidate must satisfy to be eligible.
    pub contract: Contract,
    /// True when a tight AST slice + scoped contract is in effect.
    pub is_micro: bool,
    /// A short label for logs (e.g. `src/lib.rs::is_prime`).
    pub label: String,
}

/// Build the leaf plan for a step against the repo index. Returns `None` when no
/// concrete target is available, signaling the caller to use whole-repo context
/// with an unrestricted contract.
pub fn plan_leaf(step: &Step, index: &RepoIndex) -> Option<LeafPlan> {
    let file = step.file.as_ref()?;

    // Best case: a specific symbol -> AST slice + signature/scope contract.
    if let Some(symbol) = &step.symbol {
        if let Some(slice) = index.slice_symbol(file, symbol, DEFAULT_MAX_CHARS) {
            let keep_sig = step.keep_signature.unwrap_or(true);
            let contract = Contract {
                allowed_files: vec![file.clone()],
                require_symbol: Some((file.clone(), symbol.clone())),
                keep_signature: if keep_sig {
                    Some(slice.target.signature.clone())
                } else {
                    None
                },
            };
            return Some(LeafPlan {
                context: slice.snippet,
                contract,
                is_micro: true,
                label: format!("{file}::{symbol}"),
            });
        }
    }

    // Fallback: a target file but no resolvable symbol -> scope to the file.
    if let Some(source) = index.file_source(file) {
        let context = format!("// file: {file}\n{source}");
        let contract = Contract {
            allowed_files: vec![file.clone()],
            require_symbol: None,
            keep_signature: None,
        };
        return Some(LeafPlan {
            context,
            contract,
            is_micro: true,
            label: file.clone(),
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::tempdir;

    fn step(file: &str, symbol: Option<&str>) -> Step {
        Step {
            title: "t".into(),
            file: Some(file.into()),
            symbol: symbol.map(|s| s.into()),
            ..Default::default()
        }
    }

    #[test]
    fn micro_leaf_has_slice_and_contract() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }\n",
        )
        .unwrap();
        let idx = RepoIndex::build(dir.path());
        let leaf = plan_leaf(&step("lib.rs", Some("add")), &idx).unwrap();
        assert!(leaf.is_micro);
        assert_eq!(leaf.contract.allowed_files, vec!["lib.rs".to_string()]);
        assert_eq!(
            leaf.contract.require_symbol,
            Some(("lib.rs".into(), "add".into()))
        );
        assert!(leaf
            .contract
            .keep_signature
            .as_deref()
            .unwrap()
            .contains("fn add"));
        assert!(leaf.context.contains("edit ONLY this"));
    }

    #[test]
    fn keep_signature_can_be_disabled() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), "fn f() -> i32 { 1 }\n").unwrap();
        let idx = RepoIndex::build(dir.path());
        let mut s = step("lib.rs", Some("f"));
        s.keep_signature = Some(false);
        let leaf = plan_leaf(&s, &idx).unwrap();
        assert!(leaf.contract.keep_signature.is_none());
    }

    #[test]
    fn file_only_leaf_scopes_to_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let idx = RepoIndex::build(dir.path());
        let leaf = plan_leaf(&step("a.rs", Some("ghost")), &idx).unwrap();
        // symbol missing -> falls back to file scope, still restricted
        assert_eq!(leaf.contract.allowed_files, vec!["a.rs".to_string()]);
        assert!(leaf.contract.require_symbol.is_none());
    }

    #[test]
    fn no_target_returns_none() {
        let dir = tempdir().unwrap();
        let idx = RepoIndex::build(dir.path());
        let s = Step {
            title: "t".into(),
            ..Default::default()
        };
        assert!(plan_leaf(&s, &idx).is_none());
        // also: a file not in the index
        assert!(plan_leaf(&step("nope.txt", None), &idx).is_none());
        let _ = Path::new("x"); // silence unused import in some cfgs
    }
}
