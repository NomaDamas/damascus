//! Deterministic edit application.
//!
//! Weak models produce unreliable unified diffs, so Damascus uses aider-style
//! search/replace blocks instead. Parsing and application are 100% deterministic
//! Rust — the probabilistic part (the model) only proposes; this module decides
//! whether the proposal is even applicable before any verifier runs.
//!
//! Block grammar (the path is the line immediately above the SEARCH marker,
//! ignoring a code-fence line):
//!
//! ```text
//! path/to/file.rs
//! <<<<<<< SEARCH
//! old code (empty => create a new file)
//! =======
//! new code
//! >>>>>>> REPLACE
//! ```

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, bail, Result};

const SEARCH: &str = "<<<<<<< SEARCH";
const DIVIDER: &str = "=======";
const REPLACE: &str = ">>>>>>> REPLACE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditBlock {
    pub path: String,
    pub search: String,
    pub replace: String,
}

/// Parse zero or more edit blocks from arbitrary model output. Surrounding prose
/// and ``` fences are tolerated.
pub fn parse_blocks(text: &str) -> Result<Vec<EditBlock>> {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() == SEARCH {
            let path = find_path_above(&lines, i).ok_or_else(|| {
                anyhow!("SEARCH block at line {} has no file path above it", i + 1)
            })?;
            // collect search body until divider
            let mut j = i + 1;
            let mut search = String::new();
            while j < lines.len() && lines[j].trim_end() != DIVIDER {
                search.push_str(lines[j]);
                search.push('\n');
                j += 1;
            }
            if j >= lines.len() {
                bail!("unterminated SEARCH block (missing `{DIVIDER}`)");
            }
            j += 1; // skip divider
            let mut replace = String::new();
            while j < lines.len() && lines[j].trim_end() != REPLACE {
                replace.push_str(lines[j]);
                replace.push('\n');
                j += 1;
            }
            if j >= lines.len() {
                bail!("unterminated block (missing `{REPLACE}`)");
            }
            blocks.push(EditBlock {
                path,
                search: strip_trailing_newline(&search),
                replace: strip_trailing_newline(&replace),
            });
            i = j + 1;
        } else {
            i += 1;
        }
    }
    Ok(blocks)
}

fn strip_trailing_newline(s: &str) -> String {
    s.strip_suffix('\n').unwrap_or(s).to_string()
}

/// The file path is the nearest non-empty line above the SEARCH marker, skipping
/// an opening code fence.
fn find_path_above(lines: &[&str], search_idx: usize) -> Option<String> {
    let mut k = search_idx;
    while k > 0 {
        k -= 1;
        let t = lines[k].trim();
        if t.is_empty() || t.starts_with("```") {
            continue;
        }
        // Strip common decorations like backticks or trailing colon.
        let cleaned = t.trim_matches('`').trim_end_matches(':').trim();
        if cleaned.is_empty() {
            continue;
        }
        return Some(cleaned.to_string());
    }
    None
}

/// Reject paths that escape the project root (`..`, absolute paths).
fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        bail!("refusing absolute path `{rel}`");
    }
    for c in rel_path.components() {
        if matches!(c, Component::ParentDir) {
            bail!("refusing path with `..`: `{rel}`");
        }
    }
    Ok(root.join(rel_path))
}

/// Outcome of applying an edit set, used by selection to prefer smaller diffs.
#[derive(Debug, Default, Clone)]
pub struct ApplyReport {
    pub files_changed: BTreeMap<String, ChangeKind>,
    /// Total lines emitted in replace bodies (a cheap diff-size proxy).
    pub touched_lines: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
}

/// Apply every block to the tree rooted at `root`. Returns an error if any block
/// cannot be applied unambiguously — that failure is *signal*, not noise: it
/// tells the repair loop the candidate was malformed.
pub fn apply_blocks(root: &Path, blocks: &[EditBlock]) -> Result<ApplyReport> {
    if blocks.is_empty() {
        bail!("no edit blocks found in model output");
    }
    let mut report = ApplyReport::default();
    for b in blocks {
        let abs = safe_join(root, &b.path)?;
        let existed = abs.exists();
        let current = if existed {
            std::fs::read_to_string(&abs).map_err(|e| anyhow!("reading {}: {e}", b.path))?
        } else {
            String::new()
        };

        let new_content = if b.search.trim().is_empty() {
            // Empty SEARCH => create/overwrite with the replace body.
            b.replace.clone()
        } else {
            let replaced = replace_once(&current, &b.search, &b.replace)
                .ok_or_else(|| anyhow!("SEARCH text not found (or ambiguous) in `{}`", b.path))?;
            replaced
        };

        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&abs, ensure_final_newline(&new_content))
            .map_err(|e| anyhow!("writing {}: {e}", b.path))?;

        report.touched_lines += b.replace.lines().count().max(1);
        report.files_changed.insert(
            b.path.clone(),
            if existed {
                ChangeKind::Modified
            } else {
                ChangeKind::Created
            },
        );
    }
    Ok(report)
}

fn ensure_final_newline(s: &str) -> String {
    if s.is_empty() || s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

/// Replace the first occurrence of `search` in `haystack`. Tries an exact match
/// first, then a whitespace-tolerant match (trailing whitespace per line and a
/// uniform indent shift), which absorbs the small formatting drift typical of
/// weaker models.
fn replace_once(haystack: &str, search: &str, replace: &str) -> Option<String> {
    if let Some(pos) = haystack.find(search) {
        let mut out = String::with_capacity(haystack.len() - search.len() + replace.len());
        out.push_str(&haystack[..pos]);
        out.push_str(replace);
        out.push_str(&haystack[pos + search.len()..]);
        return Some(out);
    }
    flexible_replace(haystack, search, replace)
}

fn needle_too_long(hay: &[&str], needle: &[&str]) -> bool {
    needle.len() > hay.len()
}

fn flexible_replace(haystack: &str, search: &str, replace: &str) -> Option<String> {
    let hay_lines: Vec<&str> = haystack.lines().collect();
    let search_lines: Vec<&str> = search.lines().collect();
    if search_lines.is_empty() || needle_too_long(&hay_lines, &search_lines) {
        return None;
    }
    let norm = |s: &str| s.trim_end().to_string();
    let needle: Vec<String> = search_lines.iter().map(|l| norm(l)).collect();

    let mut start = None;
    'outer: for i in 0..=hay_lines.len().saturating_sub(needle.len()) {
        for (k, want) in needle.iter().enumerate() {
            if norm(hay_lines[i + k]) != *want {
                continue 'outer;
            }
        }
        start = Some(i);
        break;
    }
    let start = start?;
    let end = start + needle.len();

    let mut out_lines: Vec<String> = Vec::with_capacity(hay_lines.len());
    out_lines.extend(hay_lines[..start].iter().map(|s| s.to_string()));
    out_lines.extend(replace.lines().map(|s| s.to_string()));
    out_lines.extend(hay_lines[end..].iter().map(|s| s.to_string()));
    Some(out_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_single_block() {
        let text = "\
Here is the change:
src/lib.rs
<<<<<<< SEARCH
fn old() {}
=======
fn new() {}
>>>>>>> REPLACE
done.";
        let blocks = parse_blocks(text).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].path, "src/lib.rs");
        assert_eq!(blocks[0].search, "fn old() {}");
        assert_eq!(blocks[0].replace, "fn new() {}");
    }

    #[test]
    fn parses_block_inside_fence() {
        let text = "```rust\nsrc/a.rs\n<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE\n```";
        let blocks = parse_blocks(text).unwrap();
        assert_eq!(blocks[0].path, "src/a.rs");
    }

    #[test]
    fn applies_modification() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nbeta\n").unwrap();
        let blocks = vec![EditBlock {
            path: "f.txt".into(),
            search: "beta".into(),
            replace: "gamma".into(),
        }];
        let rep = apply_blocks(dir.path(), &blocks).unwrap();
        assert_eq!(rep.files_changed.get("f.txt"), Some(&ChangeKind::Modified));
        let out = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(out, "alpha\ngamma\n");
    }

    #[test]
    fn creates_new_file_with_empty_search() {
        let dir = tempdir().unwrap();
        let blocks = vec![EditBlock {
            path: "new/mod.rs".into(),
            search: "".into(),
            replace: "pub fn x() {}".into(),
        }];
        let rep = apply_blocks(dir.path(), &blocks).unwrap();
        assert_eq!(
            rep.files_changed.get("new/mod.rs"),
            Some(&ChangeKind::Created)
        );
        let out = std::fs::read_to_string(dir.path().join("new/mod.rs")).unwrap();
        assert_eq!(out, "pub fn x() {}\n");
    }

    #[test]
    fn flexible_match_tolerates_trailing_whitespace() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.rs"), "let x = 1;   \nlet y = 2;\n").unwrap();
        let blocks = vec![EditBlock {
            path: "f.rs".into(),
            search: "let x = 1;".into(),
            replace: "let x = 42;".into(),
        }];
        apply_blocks(dir.path(), &blocks).unwrap();
        let out = std::fs::read_to_string(dir.path().join("f.rs")).unwrap();
        assert!(out.contains("let x = 42;"));
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let blocks = vec![EditBlock {
            path: "../escape.txt".into(),
            search: "".into(),
            replace: "x".into(),
        }];
        assert!(apply_blocks(dir.path(), &blocks).is_err());
    }

    #[test]
    fn missing_search_is_error() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "hello\n").unwrap();
        let blocks = vec![EditBlock {
            path: "f.txt".into(),
            search: "nonexistent".into(),
            replace: "x".into(),
        }];
        assert!(apply_blocks(dir.path(), &blocks).is_err());
    }
    #[test]
    fn search_longer_than_file_is_error_not_panic() {
        // Regression: flexible_replace used to index out of bounds when the
        // SEARCH spanned more lines than the target file.
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "only one line\n").unwrap();
        let blocks = vec![EditBlock {
            path: "f.txt".into(),
            search: "line one\nline two\nline three".into(),
            replace: "x".into(),
        }];
        assert!(apply_blocks(dir.path(), &blocks).is_err());
    }
}
