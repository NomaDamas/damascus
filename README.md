<p align="center">
  <img src="assets/logo.svg" alt="Damascus" width="560">
</p>

<p align="center">
  <b>A CLI coding agent built for local & open-source LLMs.</b><br>
  Damascus folds many cheap model passes through an objective verifier — so modest models produce verified, frontier-grade code.
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#how-it-works">How it works</a> ·
  <a href="#configuration">Config</a> ·
  <a href="#design">Design</a>
</p>

---

## Why Damascus

Frontier coding models are excellent in a single shot — but they are expensive and slow.
Local and open-source models (Qwen, GLM, DeepSeek, Gemma, Gemini Flash, gpt-mini class) are
cheap and fast, but less reliable *per call*.

Damascus closes that gap by spending **inference and structure** instead of model size. Like
[Damascus steel](https://en.wikipedia.org/wiki/Damascus_steel) — modest iron folded many times
into a superior blade — Damascus folds many cheap model passes through an **objective verifier**
into code that builds, passes its tests, and clears your linter.

The model never certifies its own work. **A change is accepted only if it provably passes your
gates.** Quality comes from the process, not the model.

> Bring your own local model (Ollama, llama.cpp, vLLM) or any OpenAI-compatible endpoint
> (OpenRouter, Google AI Studio, OpenAI). No data leaves your machine when you run locally.

## The Fold Loop

For every atomic step of a task:

```
        ┌─────────────┐
 task → │  1. ATOMIZE │  planner model → smallest independently-verifiable steps
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ 2. GENERATE │  best-of-N candidate edit-sets (rising temperatures, optional ensemble)
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ 3. VERIFY   │  apply each candidate in an isolated sandbox; run build + check + lint
        │   (the gate)│  ← failing candidates are discarded. This is the forcing function.
        └──────┬──────┘
               ▼
        ┌─────────────┐
        │ 4. SELECT   │  fewest diagnostics → smallest diff → LLM judge tie-break
        └──────┬──────┘
               ▼
   none passed? → REPAIR (reflexion on the failure log) → re-ATOMIZE the hard step → recurse
               ▼
        ┌─────────────┐
        │ 5. COMMIT   │  apply the winner to the real tree, record the ledger, advance
        └─────────────┘
```

The levers that turn a weak model into a reliable one are all **test-time scaling**:
best-of-N sampling, reflexion repair, recursive decomposition, and ensemble diversity.

## Install

From source (requires [Rust](https://rustup.rs) 1.80+):

```bash
git clone https://github.com/NomaDamas/damascus
cd damascus
cargo install --path .
```

This installs the `damascus` binary into `~/.cargo/bin`.

## Quick start

```bash
# 1. Drop a starter config in your project
damascus init

# 2. Edit damascus.toml to point each role at your model, then sanity-check it
damascus doctor            # offline checks
damascus doctor --probe    # makes one tiny live call per role

# 3. Run a task. Damascus plans, generates, verifies, and only keeps what passes.
damascus run "implement the is_prime function in src/lib.rs so the tests pass"
```

`damascus run` modifies files in the current directory and runs your configured verify
commands. Use `-y/--yes` to skip the confirmation prompt (e.g. in CI or overnight runs).

### Live example (a 7B local model)

Against a repo with a failing `is_prime` test, using **gemma4 (planner) + qwen2.5-coder:7b (drafter)**
on local Ollama. Notice the 7B model gets it *wrong 3 times out of 4* — and the verifier keeps
the one candidate that is actually right:

```
=== Damascus ===
  forging with drafter=local/qwen2.5-coder:7b  best-of-4  repair-rounds=2

[plan] decomposing task into atomic steps…
[ok] plan ready: 1 step(s)
  1. Implement the correct logic for is_prime

Step 1/1 Implement the correct logic for is_prime
[draft] sampling 4 candidate(s)…
  candidate 0: fail  build:FAIL check:FAIL @t0.30
  candidate 1: fail  build:FAIL check:FAIL @t0.55
  candidate 2: PASS  build:ok check:ok @t0.80
  candidate 3: fail  apply error: SEARCH text not found (or ambiguous)
[ok] step accepted (build:ok check:ok)
[review] running final critique…
[ok] final critique: LGTM

[ok] done: 1/1 steps verified
```

Then `cargo test` independently confirms 2/2 tests pass. The model is unreliable per call; the
*process* is reliable. See [`examples/is_prime`](examples/is_prime) for the full walkthrough.

## How it works

| Stage | What it does | Why it helps a weak model |
|-------|--------------|---------------------------|
| **Atomize** | Decompose into the fewest independently-verifiable steps | Small atoms have high per-step success rates |
| **Best-of-N** | Sample N candidate edit-sets at rising temperatures | More tries; the verifier picks the winner |
| **Verify gate** | Build + per-step check + lint, each in its own sandbox | Objective truth; the model can't fake it |
| **Select** | Fewest diagnostics → smallest diff → judge | Deterministic where possible, LLM only as tie-break |
| **Repair** | Reflexion: feed the failure log back and resample | Turns near-misses into passes |
| **Re-atomize** | Recursively split a stubborn step | Shrinks the problem until the model can solve it |

Edits use deterministic **search/replace blocks** (not fragile unified diffs), so applicability
is checked in Rust *before* any verifier runs. Every run is recorded under `.damascus/` for audit.

## Configuration

`damascus.toml` (created by `damascus init`):

```toml
[providers.local]
base_url = "http://localhost:11434/v1"   # Ollama / llama.cpp / vLLM
api_key_env = "OLLAMA_API_KEY"           # optional for local

[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[providers.google]
base_url = "https://generativelanguage.googleapis.com/v1beta/openai"
api_key_env = "GEMINI_API_KEY"

[models]                                  # "provider/model"; mix providers freely
planner  = "local/gemma4:e4b"
drafter  = "local/qwen2.5-coder:7b"
judge    = "local/gemma4:e4b"
repairer = "local/qwen2.5-coder:7b"

[scaling]
candidates = 3        # best-of-N samples per step
repair_rounds = 2     # reflexion retries when nothing passes
max_recursion = 2     # how deep a hard step may be re-decomposed
max_steps = 40        # global runaway guard
temperature = 0.4
temperature_step = 0.25   # added per candidate for diversity

[verify]              # the forcing functions — set these for your stack
build = "cargo build"
test  = "cargo test"
lint  = "cargo clippy -- -D warnings"
timeout_secs = 600
```

Override scaling per-run: `damascus run --candidates 5 --repair-rounds 3 "…"`.

### Provider notes

- **Ollama / llama.cpp / vLLM** — point `base_url` at the OpenAI-compatible endpoint; no key needed locally.
- **OpenRouter** — set `OPENROUTER_API_KEY`; use ids like `openrouter/deepseek/deepseek-chat`.
- **Google AI Studio** — set `GEMINI_API_KEY`; use the `/v1beta/openai` base URL.
- **OpenAI / Azure / Groq / others** — any OpenAI-compatible `/chat/completions` endpoint works.

## Commands

| Command | Description |
|---------|-------------|
| `damascus init [--force]` | Write a starter `damascus.toml` |
| `damascus doctor [--probe]` | Validate config; `--probe` makes a live test call per role |
| `damascus config` | Print the resolved configuration and its source path |
| `damascus plan "<task>"` | Decompose a task into steps (read-only, no changes) |
| `damascus run "<task>" [-y]` | Run the full Fold Loop on the current repo |

## Design

```
src/
  provider.rs      OpenAI-compatible client + ChatProvider trait (mockable)
  config.rs        providers, model roles, scaling & verify knobs
  edits.rs         deterministic search/replace block parse + apply
  sandbox.rs       throwaway per-candidate working copies
  verify.rs        the objective gate (build/check/lint, timeouts, scoring)
  generate.rs      best-of-N sampling + single-shot repair
  select.rs        diagnostics/diff/judge selection
  plan.rs          task → atomic steps (robust JSON extraction)
  context.rs       focused file context for each step
  orchestrator.rs  the Fold Loop
  ledger.rs        durable .damascus/ run state
  ui.rs            legible colored output
```

Damascus is provider-agnostic, ships as a single static binary, and is covered by unit tests
plus an offline end-to-end test of the whole loop (mock provider — `cargo test`).

## Prior art & inspiration

Damascus stands on ideas from the agent-harness ecosystem — multi-role orchestration
([oh-my-opencode](https://github.com/code-yeongyu/oh-my-openagent)), the
`plan → execute → verified completion` workflow and durable state
([oh-my-codex](https://github.com/Yeachan-Heo/oh-my-codex),
[lazycodex](https://github.com/code-yeongyu/lazycodex)), and Ralph-style persistent
completion loops — and focuses them on a single thesis: **make small, local models punch
above their weight with verification-gated test-time scaling.**

## License

[MIT](LICENSE) © NomaDamas
