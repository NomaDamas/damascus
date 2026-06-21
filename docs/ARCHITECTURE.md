# Architecture

Damascus is a high-throughput, verify-gated coding harness specialized for **fast, cheap,
narrow-context** models (Gemma, Qwen, gpt-oss, DeepSeek, …). Frontier harnesses assume a slow,
expensive, very smart model with a huge context window. Damascus assumes the opposite worker
profile and is built to extract frontier-grade output from it: slice the work tiny, generate
many candidates in parallel, and reject the bad ones with deterministic machinery instead of
trusting the model.

The design is MECE across four subsystems.

## 1. Context Isolation & Compression — `ast.rs`, `slice.rs`

OSS models lose accuracy as context grows, so we never hand them a whole file.

- `ast.rs` wraps **tree-sitter** for Rust, Python, JavaScript, TypeScript and Go. It detects the
  language, reports syntax errors, and extracts named definitions (functions, methods, structs,
  enums, traits, types, classes, interfaces) with their byte/line spans and signature header.
- `slice.rs` builds a repo-wide `RepoIndex` (every supported file parsed once) and produces a
  **slice**: the target definition plus the *signatures* of the types/functions it references,
  capped to ~3.5K tokens (`DEFAULT_MAX_CHARS`). Small referenced types are inlined in full; large
  ones are reduced to `signature { … }`. Concentration over volume.

## 2. High-Throughput Generation — `generate.rs`

Cheap + fast means we can afford to sample a lot.

- `sample_candidates` runs a **two-track rollout**: the lower half of N samples exploits around
  `temperature` (focused), the upper half explores around `explore_temperature` (diverse).
- Requests are issued concurrently with a `buffer_unordered(concurrency)` cap, so N can be large
  (8, 32, 64) without overwhelming the endpoint. Samples with no parseable edits are dropped.

## 3. Deterministic Multi-Stage Filter — `filter.rs`

Dozens of candidates must be triaged *fast* and *without an LLM*. Stages run cheapest-first:

1. **Apply** — edits are applied in memory (`edits::compute_changes`); inapplicable patches are
   rejected immediately (no disk, no build).
2. **Syntax (Stage 1)** — each changed file is parsed with tree-sitter; any parse error rejects
   the candidate. This kills a large fraction of garbage in microseconds.
3. **Contract (Stage 2)** — enforces the leaf's micro-patch rules: edits must stay within the
   allowed file(s) (scope), the target symbol must still exist, and (optionally) its signature
   must be preserved.
4. **Verify (Stage 3)** — only survivors pay for a sandboxed `build → check → lint`
   (`verify.rs`, in a throwaway copy from `sandbox.rs`).

The funnel (`N generated → S passed filter → P verified`) is printed every step.

## 4. Hierarchical Goal Tree — `plan.rs`, `tree.rs`, `orchestrator.rs`

The harness owns the plan; the model never edits it.

- `plan.rs` decomposes a task into ordered steps. A step may name a target `file` and `symbol`.
- `tree.rs::plan_leaf` turns a step into a `LeafPlan`: a tight slice as context plus a `Contract`
  (allowed files, required symbol, signature to keep). When no symbol resolves it falls back to
  file scope; when no file is named it falls back to whole-repo context with no restriction.
- The model only ever submits a **micro-patch** for one leaf. A patch that violates the contract
  is discarded by the filter. A passing winner is mechanically merged into the real tree, the
  changed files are recorded, and the tree advances. A weak planner is caged by system rules.

## The Fold Loop (per step, `orchestrator.rs`)

```
build RepoIndex → plan_leaf (slice + contract)
        │
        ▼
generate best-of-N (two-track, concurrent)
        │
        ▼
filter funnel:  apply → syntax → contract  →  survivors
        │                                          │
        │ none survive                             ▼
        ▼                                   sandbox verify (stage 3)
reflexion repair (R rounds, contract-checked)      │
        │ still none                                ▼
        ▼                                     select best (diag → diff → judge)
recursive re-atomize (depth < max)                 │
        │ sub-steps pass → success                  ▼
        ▼                                    merge winner → record → advance
     step failed
```

Nothing is accepted until it provably parses, honors the contract, builds, passes its check,
and clears lints. Quality is produced by the process, not the model.

## Module map

| Module | Responsibility |
|--------|----------------|
| `provider` | `ChatProvider` trait + OpenAI-compatible client (OpenAI/OpenRouter/Google/Ollama/vLLM/llama.cpp + mock) |
| `config` | providers, model roles, scaling (N, repair, recursion, concurrency, two-track temps), verify gates |
| `ast` | tree-sitter: language detection, syntax-error check, symbol extraction |
| `slice` | `RepoIndex` + sub-file AST slicing with dependency signatures |
| `edits` | search/replace block parse, in-memory `compute_changes`, on-disk `apply_blocks` |
| `filter` | deterministic prefilter: apply → syntax → contract |
| `tree` | goal-tree leaf planning: slice + `Contract` derivation |
| `sandbox` | throwaway per-candidate working copies |
| `verify` | objective gate (build/check/lint, timeouts, diagnostics) |
| `generate` | two-track concurrent best-of-N + single-shot repair |
| `select` | passing-candidate ranking (diagnostics → diff → judge) |
| `plan` | task → atomic steps with optional file/symbol targets |
| `context` | whole-file fallback context |
| `orchestrator` | the Fold Loop; owns budgets and the changed-file ledger |
| `ledger` | durable `.damascus/runs/<id>/` state |
| `ui` | legible colored progress + filter funnel |

## Testing

- Unit tests live beside each module (`cargo test --lib`): tree-sitter slicing, the filter stages,
  contract enforcement, two-track temperatures, edit application, plan parsing, selection, …
- `tests/fold_loop.rs` drives the whole orchestrator with an in-process mock `ChatProvider`,
  covering the happy path and the reflexion-repair path with **no network**.
- `bench/` is an end-to-end benchmark (real models via OpenRouter) reporting solve-rate and time.

## Extension points

- **Ensemble**: point `planner`/`drafter`/`judge`/`repairer` at different models.
- **More languages**: add a tree-sitter grammar and a `Lang` arm.
- **Richer gates**: `verify` runs arbitrary shell — add coverage thresholds, fuzzers, property tests.
- **Bigger throughput**: raise `candidates` and `concurrency` for endpoints that batch well.
