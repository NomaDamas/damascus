# Architecture

Damascus is a verify-gated, test-time-scaling coding harness. Its central claim is that
**process, not model size, produces reliable code** — so it is engineered to make the
deterministic parts (edit application, sandboxing, verification, selection) trustworthy and
to confine the probabilistic part (the model) to proposing candidates that the deterministic
machinery then accepts or rejects.

## Module map

| Module | Responsibility |
|--------|----------------|
| `provider` | `ChatProvider` trait + OpenAI-compatible HTTP client. One abstraction covers OpenAI, OpenRouter, Google AI Studio, Ollama, vLLM, llama.cpp, and the in-process test mock. `ModelRef::parse` splits `provider/model` on the first `/` only, so slash-containing model ids survive. |
| `config` | Providers, model roles (`planner`/`drafter`/`judge`/`repairer`), scaling knobs (`candidates`, `repair_rounds`, `max_recursion`, `max_steps`, temperatures), and verify gates. Discovered in CWD then `~/.config/damascus`. |
| `edits` | Deterministic search/replace block parser and applier. Exact match first, then a whitespace-tolerant line match. Rejects path traversal and ambiguous/oversized searches *before* anything runs. |
| `sandbox` | A throwaway copy of the project per candidate, skipping heavy/derived dirs (`target`, `node_modules`, `.git`, …). Self-deletes on drop. |
| `verify` | Runs `build` → per-step `check` (or global `test`) → `lint`, each as a timed `sh -c` in the sandbox. Produces a `Verdict` with pass/fail, a diagnostics count, and failure logs. |
| `generate` | Best-of-N: N concurrent samples at rising temperatures; drops samples with no parseable edits. Plus a single-shot reflexion `repair_once`. |
| `select` | Ranks passing candidates: fewest diagnostics → smallest diff → LLM judge tie-break (judge consulted only on ties). |
| `plan` | Asks the planner for a JSON step array; robust balanced-bracket extraction tolerates prose/fences; falls back to a single step. |
| `context` | Builds the small, focused context a weak model needs: a repo listing plus the contents of files the step references. |
| `orchestrator` | The Fold Loop. Owns the per-step state machine and the global step budget. |
| `ledger` | Durable `.damascus/runs/<id>/` state: `run.json`, `steps.jsonl`, `summary.md`, and a `latest` pointer. |
| `ui` | Legible, optionally-colored progress on stderr. |

## The Fold Loop (per step)

```
generate best-of-N
      │
      ▼
verify each candidate in its own sandbox  ──►  passing set
      │                                              │
      │ none pass                                    ▼
      ▼                                        select best
reflexion repair (R rounds, conditioned        (diag → diff → judge)
 on the failure log)                                 │
      │ still none                                    ▼
      ▼                                        apply winner to real tree
recursively re-atomize (depth < max)                 │
      │ sub-steps all pass → success                  ▼
      ▼                                            record ledger
   else → step failed
```

### Why these choices

- **Search/replace over diffs.** Weak models produce unappliable unified-diff hunks. Block
  application is deterministic and verifiable; a malformed candidate becomes *signal* fed to repair.
- **Sandbox per candidate.** A wrong or malicious candidate can never touch the real tree; only a
  verified winner is re-applied to the working directory.
- **Objective gate first, judge last.** Selection is deterministic whenever build/test/lint and
  diff size can decide it; the LLM judge is a bounded tie-breaker, not the arbiter.
- **Budgets everywhere.** `candidates`, `repair_rounds`, `max_recursion`, and `max_steps` bound
  cost so a stubborn step can't run away.

## Testing

- Unit tests live beside each module (`cargo test --lib`).
- `tests/fold_loop.rs` drives the entire orchestrator with an in-process mock `ChatProvider`,
  exercising both the happy path and the reflexion-repair path with **no network**.

## Extension points

- **Ensemble diversity**: point `drafter`/`repairer`/`judge` at different models.
- **New backends**: anything OpenAI-compatible already works; other shapes need only a new
  `ChatProvider` impl.
- **Richer gates**: `verify` runs arbitrary shell, so coverage thresholds, fuzzers, or property
  tests slot in as additional commands.
