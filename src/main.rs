//! Damascus — forge frontier-grade code from local, open-source models.
//!
//! The binary wires the CLI to the Fold Loop. Library internals live in the
//! sibling modules and are unit-tested independently.

use damascus::{config, context, orchestrator, plan, provider, ui};

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};

use config::Config;
use orchestrator::Orchestrator;
use provider::{ChatRequest, Message, ModelRef, OpenAiClient};
use ui::Ui;

#[derive(Parser)]
#[command(
    name = "damascus",
    version,
    about = "Forge frontier-grade code from local, open-source models.",
    long_about = "Damascus wraps cheap or local LLMs in a verify-gated test-time-scaling loop \
(best-of-N + reflexion repair + recursive decomposition) so modest models produce \
frontier-quality, verified changes."
)]
struct Cli {
    /// Path to damascus.toml (defaults to CWD then ~/.config/damascus).
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long, global = true)]
    no_color: bool,

    /// Suppress progress output.
    #[arg(long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter damascus.toml in the current directory.
    Init {
        /// Overwrite an existing config.
        #[arg(long)]
        force: bool,
    },
    /// Validate configuration and (optionally) probe providers live.
    Doctor {
        /// Make a tiny live request to each role's provider.
        #[arg(long)]
        probe: bool,
    },
    /// Print the resolved configuration and its source path.
    Config,
    /// Decompose a task into atomic steps and print them (no changes made).
    Plan {
        /// The task description.
        task: Vec<String>,
    },
    /// Run the full Fold Loop on the current repository.
    Run {
        /// The task description.
        task: Vec<String>,
        /// Apply changes without the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
        /// Override best-of-N candidate count.
        #[arg(long)]
        candidates: Option<usize>,
        /// Override reflexion repair rounds.
        #[arg(long)]
        repair_rounds: Option<usize>,
        /// Ablation: disable AST slicing / micro-patch contract (whole-file context).
        #[arg(long)]
        no_slice: bool,
        /// Ablation: disable the deterministic syntax/contract pre-filter.
        #[arg(long)]
        no_filter: bool,
        /// Ablation: disable recursive re-atomization of hard steps.
        #[arg(long)]
        no_decompose: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let color = !cli.no_color && std::io::stderr().is_terminal();
    let ui = Ui::new(color, cli.quiet);

    if let Err(e) = run(cli, ui).await {
        ui.error(&format!("{e:#}"));
        std::process::exit(1);
    }
}

async fn run(cli: Cli, ui: Ui) -> Result<()> {
    match cli.command {
        Command::Init { force } => cmd_init(force, ui),
        Command::Doctor { probe } => cmd_doctor(cli.config, probe, ui).await,
        Command::Config => cmd_config(cli.config),
        Command::Plan { task } => cmd_plan(cli.config, join(task), ui).await,
        Command::Run {
            task,
            yes,
            candidates,
            repair_rounds,
            no_slice,
            no_filter,
            no_decompose,
        } => {
            let ablation = orchestrator::Ablation {
                no_slice,
                no_filter,
                no_decompose,
            };
            cmd_run(
                cli.config,
                join(task),
                yes,
                candidates,
                repair_rounds,
                ablation,
                ui,
            )
            .await
        }
    }
}

fn join(parts: Vec<String>) -> String {
    parts.join(" ").trim().to_string()
}

fn load_config(path: Option<PathBuf>) -> Result<(Config, PathBuf)> {
    match path {
        Some(p) => Ok((Config::load(&p)?, p)),
        None => Config::discover(),
    }
}

fn cmd_init(force: bool, ui: Ui) -> Result<()> {
    let path = PathBuf::from(config::CONFIG_FILE);
    if path.exists() && !force {
        return Err(anyhow!(
            "{} already exists (use --force to overwrite)",
            path.display()
        ));
    }
    std::fs::write(&path, Config::template())
        .with_context(|| format!("writing {}", path.display()))?;
    ui.success(&format!("wrote {}", path.display()));
    ui.dim(
        "Edit it to point each role at your local/open-source model, then run `damascus doctor`.",
    );
    Ok(())
}

fn cmd_config(path: Option<PathBuf>) -> Result<()> {
    let (cfg, p) = load_config(path)?;
    println!("# config source: {}", p.display());
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

async fn cmd_doctor(path: Option<PathBuf>, probe: bool, ui: Ui) -> Result<()> {
    let (cfg, p) = load_config(path)?;
    ui.phase("doctor", &format!("config: {}", p.display()));

    let mut ok = true;
    for (name, pcfg) in &cfg.providers {
        let key = pcfg.resolve_api_key();
        let key_state = match (&pcfg.api_key_env, &key) {
            (_, Some(_)) => "key: present".to_string(),
            (Some(var), None) => format!("key: MISSING (${var})"),
            (None, None) => "key: none (ok for local)".to_string(),
        };
        ui.dim(&format!(
            "  provider {name}: {} | {key_state}",
            pcfg.base_url
        ));
    }

    for (role, value) in [
        ("planner", &cfg.models.planner),
        ("drafter", &cfg.models.drafter),
        ("judge", &cfg.models.judge),
        ("repairer", &cfg.models.repairer),
    ] {
        match ModelRef::parse(value) {
            Ok(m) if cfg.providers.contains_key(&m.provider) => {
                ui.dim(&format!("  role {role}: {m}"));
            }
            Ok(m) => {
                ui.error(&format!(
                    "  role {role}: provider `{}` not configured",
                    m.provider
                ));
                ok = false;
            }
            Err(e) => {
                ui.error(&format!("  role {role}: {e}"));
                ok = false;
            }
        }
    }

    if cfg.verify.build.is_none() && cfg.verify.test.is_none() && cfg.verify.lint.is_none() {
        ui.warn("no [verify] commands set — the objective gate will be weak. Configure build/test/lint.");
    } else {
        ui.dim(&format!(
            "  verify: build={} test={} lint={}",
            cfg.verify.build.as_deref().unwrap_or("-"),
            cfg.verify.test.as_deref().unwrap_or("-"),
            cfg.verify.lint.as_deref().unwrap_or("-"),
        ));
    }

    if probe {
        let client = build_client(&cfg)?;
        for (role, value) in [
            ("planner", &cfg.models.planner),
            ("drafter", &cfg.models.drafter),
        ] {
            let model = ModelRef::parse(value)?;
            ui.phase("probe", &format!("{role} -> {model}"));
            let req = ChatRequest {
                model,
                messages: vec![Message::user("Reply with exactly: DAMASCUS-OK")],
                temperature: 0.0,
                max_tokens: Some(16),
            };
            use provider::ChatProvider;
            match client.complete(req).await {
                Ok(resp) => ui.success(&format!("  {role}: {}", resp.trim())),
                Err(e) => {
                    ui.error(&format!("  {role}: {e}"));
                    ok = false;
                }
            }
        }
    }

    if ok {
        ui.success("doctor: configuration looks healthy");
        Ok(())
    } else {
        Err(anyhow!("doctor found problems (see above)"))
    }
}

fn build_client(cfg: &Config) -> Result<OpenAiClient> {
    OpenAiClient::new(
        &cfg.providers,
        Duration::from_secs(cfg.verify.timeout_secs.max(120)),
    )
}

async fn cmd_plan(path: Option<PathBuf>, task: String, ui: Ui) -> Result<()> {
    if task.is_empty() {
        return Err(anyhow!(
            "provide a task, e.g. `damascus plan \"add a CLI flag\"`"
        ));
    }
    let (cfg, _) = load_config(path)?;
    let client = build_client(&cfg)?;
    let summary = context::repo_summary(&PathBuf::from("."));
    ui.phase("plan", "decomposing…");
    let plan = plan::make_plan(&client, &cfg, &task, &summary).await?;
    for (i, s) in plan.steps.iter().enumerate() {
        println!("{}. {}", i + 1, s.title);
        if !s.detail.is_empty() {
            println!("   {}", s.detail);
        }
        if let Some(c) = &s.check {
            println!("   check: {c}");
        }
    }
    Ok(())
}

async fn cmd_run(
    path: Option<PathBuf>,
    task: String,
    yes: bool,
    candidates: Option<usize>,
    repair_rounds: Option<usize>,
    ablation: orchestrator::Ablation,
    ui: Ui,
) -> Result<()> {
    if task.is_empty() {
        return Err(anyhow!(
            "provide a task, e.g. `damascus run \"fix the failing test\"`"
        ));
    }
    let (mut cfg, _) = load_config(path)?;
    if let Some(n) = candidates {
        cfg.scaling.candidates = n.max(1);
    }
    if let Some(r) = repair_rounds {
        cfg.scaling.repair_rounds = r;
    }

    if !yes {
        eprint!(
            "Damascus will modify files in {} and run your verify commands. Continue? [y/N] ",
            std::env::current_dir()?.display()
        );
        std::io::stderr().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            ui.warn("aborted");
            return Ok(());
        }
    }

    let client = Arc::new(build_client(&cfg)?);
    let root = std::env::current_dir()?;
    let orch = Orchestrator::with_ablation(client.as_ref(), &cfg, root, ui, ablation);
    let outcome = orch.run(&task).await?;

    eprintln!();
    if outcome.all_passed() {
        ui.success(&format!(
            "done: {}/{} steps verified",
            outcome.steps_succeeded, outcome.steps_total
        ));
    } else {
        ui.warn(&format!(
            "finished with issues: {}/{} steps verified, {} failed",
            outcome.steps_succeeded, outcome.steps_total, outcome.steps_failed
        ));
    }
    if let Some(review) = &outcome.final_review {
        ui.dim("\nFinal critique:");
        println!("{review}");
    }
    if !outcome.all_passed() {
        std::process::exit(2);
    }
    Ok(())
}
