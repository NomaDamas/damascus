//! Legible, colored terminal output. The Fold Loop is only trustworthy if you
//! can watch it work: every phase, candidate, and verdict is surfaced.

use owo_colors::OwoColorize;

#[derive(Clone, Copy)]
pub struct Ui {
    pub color: bool,
    pub quiet: bool,
}

impl Ui {
    pub fn new(color: bool, quiet: bool) -> Self {
        Ui { color, quiet }
    }

    pub fn banner(&self, model: &str, candidates: usize, repair: usize) {
        if self.quiet {
            return;
        }
        let title = "  Damascus  ";
        if self.color {
            eprintln!("\n{}", title.black().on_bright_white().bold());
        } else {
            eprintln!("\n=== Damascus ===");
        }
        self.dim(&format!(
            "  forging with drafter={model}  best-of-{candidates}  repair-rounds={repair}\n"
        ));
    }

    pub fn phase(&self, label: &str, msg: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            eprintln!("{} {}", format!("[{label}]").bright_cyan().bold(), msg);
        } else {
            eprintln!("[{label}] {msg}");
        }
    }

    pub fn step(&self, idx: usize, total: usize, title: &str) {
        if self.quiet {
            return;
        }
        let head = format!("Step {}/{}", idx + 1, total);
        if self.color {
            eprintln!("\n{} {}", head.bright_magenta().bold(), title.bold());
        } else {
            eprintln!("\n{head} {title}");
        }
    }

    pub fn candidate(&self, idx: usize, passed: bool, summary: &str) {
        if self.quiet {
            return;
        }
        let mark = if passed { "PASS" } else { "fail" };
        if self.color {
            let m = if passed {
                mark.bright_green().bold().to_string()
            } else {
                mark.red().to_string()
            };
            eprintln!("  candidate {idx}: {m}  {}", summary.dimmed());
        } else {
            eprintln!("  candidate {idx}: {mark}  {summary}");
        }
    }

    pub fn success(&self, msg: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            eprintln!("{} {}", "✓".bright_green().bold(), msg.green());
        } else {
            eprintln!("[ok] {msg}");
        }
    }

    pub fn warn(&self, msg: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            eprintln!("{} {}", "!".yellow().bold(), msg.yellow());
        } else {
            eprintln!("[warn] {msg}");
        }
    }

    pub fn error(&self, msg: &str) {
        if self.color {
            eprintln!("{} {}", "✗".bright_red().bold(), msg.red());
        } else {
            eprintln!("[error] {msg}");
        }
    }

    pub fn dim(&self, msg: &str) {
        if self.quiet {
            return;
        }
        if self.color {
            eprintln!("{}", msg.dimmed());
        } else {
            eprintln!("{msg}");
        }
    }
}
