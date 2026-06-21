//! The Fold Loop.
//!
//! For every atomic step:
//!   1. generate best-of-N candidate edit-sets (test-time scaling),
//!   2. verify each in an isolated sandbox (the objective gate),
//!   3. select the best passing candidate,
//!   4. if none pass, run reflexion repair, then recursively re-atomize,
//!   5. apply the winner to the real tree and record it.
//!
//! Quality is produced by the *process*, not the model: nothing is accepted
//! until it provably builds, passes its check, and clears lints.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use anyhow::{anyhow, Result};
use chrono::Utc;

use crate::config::Config;
use crate::context;
use crate::edits::apply_blocks;
use crate::filter::{prefilter, Contract, Prefilter};
use crate::generate::{repair_once, sample_candidates, Candidate};
use crate::ledger::{new_run_id, Ledger, RunMeta, StepRecord};
use crate::plan::{make_plan, Plan, Step};
use crate::prompts;
use crate::provider::{ChatProvider, ChatRequest, Message};
use crate::sandbox::Sandbox;
use crate::select::{select_best, Scored};
use crate::slice::RepoIndex;
use crate::tree;
use crate::ui::Ui;
use crate::verify::{verify, Verdict};

pub struct Orchestrator<'a> {
    provider: &'a dyn ChatProvider,
    cfg: &'a Config,
    root: PathBuf,
    ui: Ui,
}

#[derive(Debug, Default)]
pub struct RunOutcome {
    pub steps_total: usize,
    pub steps_succeeded: usize,
    pub steps_failed: usize,
    pub final_review: Option<String>,
}

impl RunOutcome {
    pub fn all_passed(&self) -> bool {
        self.steps_failed == 0 && self.steps_total > 0
    }
}

enum StepStatus {
    Success {
        gates: String,
        candidates: usize,
        repairs: usize,
    },
    Failed {
        reason: String,
    },
}

/// Result of verifying a batch of candidates.
enum Collected {
    Passing { scored: Vec<Scored>, tried: usize },
    NonePassed { failure_log: String },
}

impl<'a> Orchestrator<'a> {
    pub fn new(provider: &'a dyn ChatProvider, cfg: &'a Config, root: PathBuf, ui: Ui) -> Self {
        Orchestrator {
            provider,
            cfg,
            root,
            ui,
        }
    }

    pub async fn run(&self, task: &str) -> Result<RunOutcome> {
        self.ui.banner(
            &self.cfg.models.drafter,
            self.cfg.scaling.candidates,
            self.cfg.scaling.repair_rounds,
        );

        self.ui.phase("plan", "decomposing task into atomic steps…");
        let summary = context::repo_summary(&self.root);
        let plan: Plan = make_plan(self.provider, self.cfg, task, &summary).await?;
        self.ui
            .success(&format!("plan ready: {} step(s)", plan.steps.len()));
        for (i, s) in plan.steps.iter().enumerate() {
            self.ui.dim(&format!("  {}. {}", i + 1, s.title));
        }

        let meta = RunMeta {
            id: new_run_id(),
            task: task.to_string(),
            started_at: Utc::now().to_rfc3339(),
            model_drafter: self.cfg.models.drafter.clone(),
            candidates: self.cfg.scaling.candidates,
        };
        let ledger = Ledger::create(&self.root, &meta)?;
        self.ui
            .dim(&format!("  ledger: {}", ledger.dir().display()));

        let mut outcome = RunOutcome::default();
        let mut changed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let total = plan.steps.len();
        let mut budget = self.cfg.scaling.max_steps;

        for (idx, step) in plan.steps.iter().enumerate() {
            if budget == 0 {
                self.ui.warn("step budget exhausted; stopping");
                break;
            }
            budget -= 1;
            outcome.steps_total += 1;
            self.ui.step(idx, total, &step.title);

            let status = self
                .process_step(task, step, 0, &mut budget, &mut changed)
                .await;
            let record = self.record_for(idx, step, &status, &mut outcome);
            ledger.record_step(&record)?;
        }

        if outcome.steps_succeeded > 0 {
            self.ui.phase("review", "running final critique…");
            if let Some(review) = self.final_review(task, &changed).await {
                outcome.final_review = Some(review);
            }
        }

        ledger.write_summary(&self.summary_md(&outcome))?;
        Ok(outcome)
    }

    fn record_for(
        &self,
        idx: usize,
        step: &Step,
        status: &StepStatus,
        outcome: &mut RunOutcome,
    ) -> StepRecord {
        match status {
            StepStatus::Success {
                gates,
                candidates,
                repairs,
            } => {
                self.ui.success(&format!("step accepted ({gates})"));
                outcome.steps_succeeded += 1;
                StepRecord {
                    index: idx,
                    title: step.title.clone(),
                    status: "success".into(),
                    detail: step.detail.clone(),
                    gates: gates.clone(),
                    candidates_tried: *candidates,
                    repair_rounds: *repairs,
                    recorded_at: Utc::now().to_rfc3339(),
                }
            }
            StepStatus::Failed { reason } => {
                self.ui.error(&format!("step failed: {reason}"));
                outcome.steps_failed += 1;
                StepRecord {
                    index: idx,
                    title: step.title.clone(),
                    status: "failed".into(),
                    detail: reason.clone(),
                    gates: String::new(),
                    candidates_tried: 0,
                    repair_rounds: 0,
                    recorded_at: Utc::now().to_rfc3339(),
                }
            }
        }
    }

    /// Process a single step. Recurses (re-atomization) up to `max_recursion`.
    fn process_step<'s>(
        &'s self,
        task: &'s str,
        step: &'s Step,
        depth: usize,
        budget: &'s mut usize,
        changed: &'s mut std::collections::BTreeSet<String>,
    ) -> Pin<Box<dyn Future<Output = StepStatus> + 's>> {
        Box::pin(async move {
            // Index the repo fresh so slices/contracts reflect the current tree.
            let index = RepoIndex::build(&self.root);
            let leaf = tree::plan_leaf(step, &index);
            let (ctx, contract, micro) = match &leaf {
                Some(lp) => {
                    self.ui.dim(&format!(
                        "  leaf: {} ({})",
                        lp.label,
                        if lp.is_micro {
                            "micro-patch, scoped"
                        } else {
                            "file-scoped"
                        }
                    ));
                    (lp.context.clone(), lp.contract.clone(), lp.is_micro)
                }
                None => (
                    context::file_context(&self.root, &format!("{} {}", step.title, step.detail)),
                    Contract::unrestricted(),
                    false,
                ),
            };
            let drafter = match self.cfg.models.drafter_ref() {
                Ok(m) => m,
                Err(e) => {
                    return StepStatus::Failed {
                        reason: e.to_string(),
                    }
                }
            };
            // The repairer role may point at a different model for diversity;
            // fall back to the drafter if it cannot be resolved.
            let repairer = self
                .cfg
                .models
                .repairer_ref()
                .unwrap_or_else(|_| drafter.clone());

            // --- 1. Best-of-N generation ---
            self.ui.phase(
                "draft",
                &format!("sampling {} candidate(s)…", self.cfg.scaling.candidates),
            );
            let candidates = sample_candidates(
                self.provider,
                &drafter,
                self.cfg,
                if micro {
                    prompts::micro_patch_system()
                } else {
                    prompts::drafter_system()
                },
                if micro {
                    prompts::micro_patch_user(task, step, &ctx)
                } else {
                    prompts::drafter_user(task, step, &ctx)
                },
                self.cfg.scaling.candidates,
            )
            .await;

            // --- 2 & 3. Verify + select ---
            let mut last_failure = match self.collect(step, candidates, &contract).await {
                Collected::Passing { scored, tried } => {
                    let best = select_best(self.provider, self.cfg, task, step, &scored).await;
                    let winner = &scored[best];
                    match apply_blocks(&self.root, &winner.candidate.blocks) {
                        Ok(report) => changed.extend(report.files_changed.into_keys()),
                        Err(e) => {
                            return StepStatus::Failed {
                                reason: format!("applying winner: {e}"),
                            }
                        }
                    }
                    return StepStatus::Success {
                        gates: winner.verdict.summary(),
                        candidates: tried,
                        repairs: 0,
                    };
                }
                Collected::NonePassed { failure_log } => failure_log,
            };

            // --- 4a. Reflexion repair ---
            for round in 0..self.cfg.scaling.repair_rounds {
                self.ui.phase(
                    "repair",
                    &format!(
                        "reflexion round {}/{}…",
                        round + 1,
                        self.cfg.scaling.repair_rounds
                    ),
                );
                let user = prompts::repair_user(task, step, &last_failure, &ctx);
                let temp = self.cfg.scaling.temperature_for(round);
                let cand = match repair_once(
                    self.provider,
                    &repairer,
                    self.cfg,
                    if micro {
                        prompts::micro_patch_system()
                    } else {
                        prompts::drafter_system()
                    },
                    user,
                    temp,
                )
                .await
                {
                    Ok(Some(c)) => c,
                    _ => continue,
                };
                // Same deterministic filter applies to repairs.
                let changes = match prefilter(&self.root, &cand.blocks, &contract) {
                    Prefilter::Pass(c) => c,
                    other => {
                        self.ui.candidate(0, false, &other.reason());
                        last_failure = other.reason();
                        continue;
                    }
                };
                match self.verify_changes(step, &changes).await {
                    Ok(verdict) if verdict.passed => {
                        self.ui.candidate(0, true, &verdict.summary());
                        match apply_blocks(&self.root, &cand.blocks) {
                            Ok(report) => changed.extend(report.files_changed.into_keys()),
                            Err(e) => {
                                return StepStatus::Failed {
                                    reason: format!("applying repair: {e}"),
                                }
                            }
                        }
                        return StepStatus::Success {
                            gates: verdict.summary(),
                            candidates: 1,
                            repairs: round + 1,
                        };
                    }
                    Ok(verdict) => {
                        self.ui.candidate(0, false, &verdict.summary());
                        if let Some(log) = verdict.first_failure_log() {
                            last_failure = log.to_string();
                        }
                    }
                    Err(e) => last_failure = e.to_string(),
                }
            }

            // --- 4b. Recursive re-atomization ---
            if depth < self.cfg.scaling.max_recursion && *budget > 0 {
                self.ui
                    .phase("atomize", "step is hard; decomposing further…");
                let seed = if step.detail.trim().is_empty() {
                    &step.title
                } else {
                    &step.detail
                };
                let sub = make_plan(
                    self.provider,
                    self.cfg,
                    seed,
                    &context::repo_summary(&self.root),
                )
                .await
                .unwrap_or_default();
                if sub.steps.len() > 1 {
                    let mut all_ok = true;
                    for (i, ss) in sub.steps.iter().enumerate() {
                        if *budget == 0 {
                            all_ok = false;
                            break;
                        }
                        *budget -= 1;
                        self.ui
                            .dim(&format!("    sub-step {}: {}", i + 1, ss.title));
                        if let StepStatus::Failed { .. } = self
                            .process_step(task, ss, depth + 1, budget, changed)
                            .await
                        {
                            all_ok = false;
                            break;
                        }
                    }
                    if all_ok {
                        return StepStatus::Success {
                            gates: "via-decomposition".into(),
                            candidates: 0,
                            repairs: self.cfg.scaling.repair_rounds,
                        };
                    }
                }
            }

            StepStatus::Failed {
                reason: format!("no candidate passed: {}", truncate(&last_failure, 200)),
            }
        })
    }

    /// Run the deterministic filter funnel over every candidate, then verify the
    /// survivors. Returns passing (scored) or none-passed with the best log.
    async fn collect(
        &self,
        step: &Step,
        candidates: Vec<Candidate>,
        contract: &Contract,
    ) -> Collected {
        let total = candidates.len();
        if total == 0 {
            return Collected::NonePassed {
                failure_log: "model produced no parseable edit blocks".into(),
            };
        }
        let mut scored = Vec::new();
        let mut failure_log = String::from("all candidates failed verification");
        let mut syntax_ok = 0usize;
        for cand in candidates {
            // Stages 1 & 2: deterministic, no sandbox, no LLM.
            let changes = match prefilter(&self.root, &cand.blocks, contract) {
                Prefilter::Pass(c) => {
                    syntax_ok += 1;
                    c
                }
                other => {
                    self.ui.candidate(cand.index, false, &other.reason());
                    failure_log = other.reason();
                    continue;
                }
            };
            // Stage 3: sandboxed build/test, only for survivors.
            match self.verify_changes(step, &changes).await {
                Ok(verdict) => {
                    let summary = format!("{} @t{:.2}", verdict.summary(), cand.temperature);
                    self.ui.candidate(cand.index, verdict.passed, &summary);
                    if verdict.passed {
                        scored.push(Scored {
                            candidate: cand,
                            verdict,
                            touched_lines: changes.report.touched_lines,
                        });
                    } else if let Some(log) = verdict.first_failure_log() {
                        failure_log = log.to_string();
                    }
                }
                Err(e) => {
                    self.ui.candidate(
                        cand.index,
                        false,
                        &format!("verify error: {}", truncate(&e.to_string(), 80)),
                    );
                    failure_log = e.to_string();
                }
            }
        }
        self.ui.dim(&format!(
            "  funnel: {total} generated → {syntax_ok} passed filter → {} verified",
            scored.len()
        ));
        let tried = scored.len();
        if scored.is_empty() {
            Collected::NonePassed { failure_log }
        } else {
            Collected::Passing { scored, tried }
        }
    }

    /// Materialize precomputed changes into a fresh sandbox and run the gate.
    async fn verify_changes(
        &self,
        step: &Step,
        changes: &crate::edits::Changes,
    ) -> Result<Verdict> {
        let sandbox = Sandbox::create(&self.root, &step.title)?;
        for (rel, content) in &changes.contents {
            let abs = sandbox.path().join(rel);
            if let Some(parent) = abs.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&abs, content).map_err(|e| anyhow!("writing {rel} in sandbox: {e}"))?;
        }
        Ok(verify(sandbox.path(), &self.cfg.verify, step.check.as_deref()).await)
    }

    async fn final_review(
        &self,
        task: &str,
        changed: &std::collections::BTreeSet<String>,
    ) -> Option<String> {
        let model = self.cfg.models.judge_ref().ok()?;
        let summary = if changed.is_empty() {
            context::repo_summary(&self.root)
        } else {
            // Show the actual post-change contents of the files we touched.
            let mut s = String::new();
            for rel in changed {
                if let Ok(content) = std::fs::read_to_string(self.root.join(rel)) {
                    let body: String = content.lines().take(200).collect::<Vec<_>>().join("\n");
                    s.push_str(&format!("--- {rel} ---\n{body}\n\n"));
                }
            }
            s
        };
        let req = ChatRequest {
            model,
            messages: vec![
                Message::system(prompts::judge_system()),
                Message::user(prompts::final_critic_user(task, &summary)),
            ],
            temperature: 0.0,
            max_tokens: self.cfg.scaling.max_tokens,
        };
        let review = self.provider.complete(req).await.ok()?;
        let trimmed = review.trim();
        if trimmed.eq_ignore_ascii_case("LGTM") {
            self.ui.success("final critique: LGTM");
            None
        } else {
            self.ui.warn("final critique raised notes (see summary)");
            Some(trimmed.to_string())
        }
    }

    fn summary_md(&self, o: &RunOutcome) -> String {
        let mut s = format!(
            "# Damascus run summary\n\n- steps total: {}\n- succeeded: {}\n- failed: {}\n",
            o.steps_total, o.steps_succeeded, o.steps_failed
        );
        if let Some(r) = &o.final_review {
            s.push_str(&format!("\n## Final critique\n\n{r}\n"));
        }
        s
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}
