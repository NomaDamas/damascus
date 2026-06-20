//! Durable, resumable run state under `.damascus/`. Every step outcome is
//! appended so a run can be inspected or audited after the fact.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub const STATE_DIR: &str = ".damascus";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub id: String,
    pub task: String,
    pub started_at: String,
    pub model_drafter: String,
    pub candidates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub index: usize,
    pub title: String,
    pub status: String,
    pub detail: String,
    pub gates: String,
    pub candidates_tried: usize,
    pub repair_rounds: usize,
    pub recorded_at: String,
}

pub struct Ledger {
    dir: PathBuf,
}

impl Ledger {
    /// Create `.damascus/runs/<id>/` and write run metadata.
    pub fn create(root: &Path, meta: &RunMeta) -> Result<Self> {
        let dir = root.join(STATE_DIR).join("runs").join(&meta.id);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating ledger {}", dir.display()))?;
        std::fs::write(dir.join("run.json"), serde_json::to_string_pretty(meta)?)
            .context("writing run.json")?;
        // convenience pointer to the latest run
        let latest = root.join(STATE_DIR).join("latest");
        let _ = std::fs::write(latest, &meta.id);
        Ok(Ledger { dir })
    }

    pub fn record_step(&self, rec: &StepRecord) -> Result<()> {
        let line = serde_json::to_string(rec)? + "\n";
        let path = self.dir.join("steps.jsonl");
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    pub fn write_summary(&self, summary: &str) -> Result<()> {
        std::fs::write(self.dir.join("summary.md"), summary).context("writing summary.md")?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// A timestamped, filesystem-safe run id.
pub fn new_run_id() -> String {
    Utc::now().format("%Y%m%d-%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_and_records() {
        let d = tempdir().unwrap();
        let meta = RunMeta {
            id: "run1".into(),
            task: "do it".into(),
            started_at: Utc::now().to_rfc3339(),
            model_drafter: "local/m".into(),
            candidates: 3,
        };
        let l = Ledger::create(d.path(), &meta).unwrap();
        l.record_step(&StepRecord {
            index: 0,
            title: "t".into(),
            status: "success".into(),
            detail: "".into(),
            gates: "build:ok".into(),
            candidates_tried: 3,
            repair_rounds: 0,
            recorded_at: Utc::now().to_rfc3339(),
        })
        .unwrap();
        assert!(d.path().join(".damascus/runs/run1/run.json").exists());
        assert!(d.path().join(".damascus/runs/run1/steps.jsonl").exists());
        assert_eq!(
            std::fs::read_to_string(d.path().join(".damascus/latest")).unwrap(),
            "run1"
        );
    }
}
