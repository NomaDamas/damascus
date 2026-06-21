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

## A different worker profile

Claude Code and the original Codex assume a *slow, expensive, very smart* model with a huge
context window. The local/open models we target have the opposite profile: **slightly less
clever, a narrow context window — but astonishingly fast and 10×+ cheaper.** Forcing the
frontier playbook onto them loses. Damascus is built around their real strengths — *massive,
cheap throughput* — with four subsystems:

| | Frontier harness | Damascus (OSS-specialized) |
|---|---|---|
| **Context** | dump the whole file/repo (200K) | **AST-sliced** snippets (<4K): the target definition + dependency signatures |
| **Generation** | one or two careful tries | **massively parallel** best-of-N (8/32/64) at two temperatures |
| **Verification** | the model reviews itself | a **deterministic 3-stage filter**: parser → contract → sandbox |
| **Edit scope** | roam freely across files | a **micro-patch** to one harness-designated leaf only |

## The Fold Loop

For every atomic leaf of a task:

```
        ┌──────────────┐
 task → │ 1. ATOMIZE   │  planner → smallest verifiable steps, each targeting a file::symbol
        └──────┬───────┘
               ▼
        ┌──────────────┐
        │ 2. SLICE     │  tree-sitter cuts a <4K-token slice: the target def + dep signatures
        └──────┬───────┘
               ▼
        ┌──────────────┐
        │ 3. GENERATE  │  best-of-N micro-patches, two-track temperatures, concurrent
        └──────┬───────┘
               ▼
        ┌──────────────┐
        │ 4. FILTER    │  apply → syntax(parser) → contract(scope+signature) → sandbox build/test
        │  (the funnel)│  ← deterministic, no LLM; garbage is rejected in microseconds
        └──────┬───────┘
               ▼
        ┌──────────────┐
        │ 5. SELECT    │  fewest diagnostics → smallest diff → LLM judge tie-break
        └──────┬───────┘
               ▼
   none survive? → REPAIR (reflexion on the failure log) → re-ATOMIZE the hard leaf → recurse
               ▼
        ┌──────────────┐
        │ 6. MERGE     │  apply the winner to the real tree, record the ledger, advance
        └──────────────┘
```

The harness owns the plan tree; the model never edits it — it only submits a micro-patch for the
single leaf it is handed, and only a patch that **provably parses, honors the contract, builds,
and passes the tests** is merged. Quality comes from the process, not the model. The levers are
all test-time scaling: best-of-N, reflexion repair, recursive decomposition, ensemble diversity.

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

### Live example

A repo with a failing `is_prime` test, drafter = **qwen2.5-coder:7b** on local Ollama. The 7B
model gets it *wrong 3 of 4 times* — the deterministic filter + sandbox keep the one that is
actually right:

```
=== Damascus ===
  forging with drafter=local/qwen2.5-coder:7b  best-of-4  repair-rounds=2

[plan] decomposing task into atomic steps…
[ok] plan ready: 1 step(s)

Step 1/1 Implement the correct logic for is_prime
  leaf: src/lib.rs::is_prime (micro-patch, scoped)
[draft] sampling 4 candidate(s)…
  candidate 0: fail  syntax error in src/lib.rs
  candidate 1: PASS  build:ok check:ok @t0.30
  candidate 2: fail  contract: target signature changed
  candidate 3: PASS  build:ok check:ok @t0.90
  funnel: 4 generated → 2 passed filter → 2 verified
[ok] step accepted (build:ok check:ok)

[ok] done: 1/1 steps verified
```

`cargo test` then independently confirms the tests pass. The model is unreliable per call; the
*process* is reliable. See [`examples/is_prime`](examples/is_prime) for the full walkthrough.

## Benchmark

`bench/` runs each task through the full Fold Loop and checks whether `cargo test` passes
afterwards. Latest run (`candidates=6`, gates = `cargo build` + `cargo test`, models via
OpenRouter):

| model | solved | solve-rate | avg seconds |
|-------|:------:|:----------:|:-----------:|
| `google/gemma-4-26b-a4b-it` | 6/6 | **100%** | **21.0** |
| `openai/gpt-oss-120b` | 6/6 | **100%** | 38.5 |

Both modest models solve every task — and the smaller MoE is **~1.8× faster** at equal solve-rate.
Tasks range from single-function implementations to a cross-file method and a bug-fix. Reproduce
with `bench/run.sh`; details in [`bench/README.md`](bench/README.md).

## How it works

| Stage | What it does | Why it helps a fast/narrow model |
|-------|--------------|----------------------------------|
| **Atomize** | Decompose into the fewest verifiable leaves, each a `file::symbol` | Small atoms have high per-step success |
| **Slice** | tree-sitter cuts a <4K-token snippet: target def + dependency signatures | Keeps the narrow context window razor-focused |
| **Best-of-N** | Sample N micro-patches concurrently, two temperature tracks | Exploits cheap throughput; the filter picks the winner |
| **Filter** | apply → syntax(parser) → contract(scope/signature) → sandbox build/test | Rejects garbage deterministically, no LLM |
| **Select** | Fewest diagnostics → smallest diff → judge | Deterministic where possible, LLM only as tie-break |
| **Repair** | Reflexion: feed the failure log back and resample | Turns near-misses into passes |
| **Re-atomize** | Recursively split a stubborn leaf | Shrinks the problem until the model can solve it |

Edits use deterministic **search/replace blocks** (not fragile unified diffs), syntax-checked with
tree-sitter *before* any sandbox runs. Every run is recorded under `.damascus/` for audit.

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
candidates = 8          # high-throughput best-of-N (raise to 16/32 for hard tasks)
repair_rounds = 2       # reflexion retries when nothing passes
max_recursion = 2       # how deep a hard leaf may be re-decomposed
max_steps = 40          # global runaway guard
temperature = 0.3       # focus-track base temperature
temperature_step = 0.2  # focus-track ramp
explore_temperature = 0.9  # explore-track temperature (the diverse half)
concurrency = 8         # max model requests in flight (throughput knob)

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
  ast.rs           tree-sitter: language detect, syntax check, symbol extraction
  slice.rs         repo index + sub-file AST slicing with dependency signatures
  edits.rs         search/replace parse, in-memory compute_changes, on-disk apply
  filter.rs        deterministic 3-stage filter (apply → syntax → contract)
  tree.rs          goal-tree leaf planning: slice + contract derivation
  sandbox.rs       throwaway per-candidate working copies
  verify.rs        the objective gate (build/check/lint, timeouts, scoring)
  generate.rs      two-track concurrent best-of-N + single-shot repair
  select.rs        diagnostics/diff/judge selection
  plan.rs          task → atomic steps with optional file/symbol targets
  context.rs       whole-file fallback context
  orchestrator.rs  the Fold Loop
  ledger.rs        durable .damascus/ run state
  ui.rs            legible colored output + filter funnel
```

Damascus supports Rust, Python, JavaScript, TypeScript and Go (tree-sitter), is provider-agnostic,
ships as a single static binary, and is covered by 50+ unit tests plus an offline end-to-end test
of the whole loop (mock provider — `cargo test`). See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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
