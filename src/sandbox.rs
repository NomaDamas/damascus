//! Isolated working copies for candidate verification.
//!
//! Each best-of-N candidate is applied and verified in its own throwaway copy of
//! the project, so a malformed or wrong candidate can never corrupt the real
//! tree. Heavy/derived directories are skipped to keep copies cheap.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

/// Directory names never copied into a sandbox.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".damascus",
    ".venv",
    "venv",
    "dist",
    "build",
    ".next",
    ".cargo",
    "__pycache__",
];

/// A temporary copy of the project that deletes itself on drop.
pub struct Sandbox {
    pub root: PathBuf,
    _keep: bool,
}

impl Sandbox {
    /// Create a fresh copy of `src` under a unique temp directory.
    pub fn create(src: &Path, tag: &str) -> Result<Self> {
        let base =
            std::env::temp_dir().join(format!("damascus-{}-{}", sanitize(tag), unique_suffix()));
        std::fs::create_dir_all(&base)
            .with_context(|| format!("creating sandbox {}", base.display()))?;
        copy_tree(src, &base)?;
        Ok(Sandbox {
            root: base,
            _keep: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if !self._keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(24)
        .collect()
}

fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{n}")
}

/// Copy a directory tree, skipping [`SKIP_DIRS`] at any depth.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src)
        .into_iter()
        .filter_entry(|e| !is_skipped(e.path(), src))
    {
        let entry = entry.context("walking source tree")?;
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).ok();
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn is_skipped(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    if path.is_dir() {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            return SKIP_DIRS.contains(&name);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn copies_files_and_skips_heavy_dirs() {
        let src = tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("src")).unwrap();
        std::fs::write(src.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(src.path().join("target/debug")).unwrap();
        std::fs::write(src.path().join("target/debug/x"), "binary").unwrap();

        let sb = Sandbox::create(src.path(), "test").unwrap();
        assert!(sb.path().join("src/main.rs").exists());
        assert!(!sb.path().join("target").exists());
    }

    #[test]
    fn sandbox_cleans_up_on_drop() {
        let src = tempdir().unwrap();
        std::fs::write(src.path().join("a.txt"), "hi").unwrap();
        let path = {
            let sb = Sandbox::create(src.path(), "drop").unwrap();
            sb.path().to_path_buf()
        };
        assert!(!path.exists());
    }
}
