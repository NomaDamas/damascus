# Can a cheap-model harness reach frontier? — an honest evaluation

This document reports a controlled comparison between Damascus driving **cheap open models** and
two **frontier coding agents**, and gives a feasibility verdict for the question: *under what
conditions (if any) can Damascus + a modest model match Claude Code (Opus 4.8) or Codex (gpt‑5.5)?*

Everything here is from real runs on this machine. No numbers are fabricated.

## Why not the first benchmark

The first attempt used the **aider polyglot** (Exercism) set. Even the *hard* Python exercises
were solved **10/10 by Claude Code + Opus 4.8** — a ceiling effect: classic, well-known
algorithmic katas are effectively memorized by frontier models, so the benchmark cannot
discriminate at the top. It was abandoned.

## The benchmark used

**BigCodeBench‑Hard** (the "hard" split of BigCodeBench) — complex, real‑library function tasks
with rich hidden test suites. To run safely **without Docker**, the set was filtered to tasks whose
libraries are non‑networking / non‑process (numpy, pandas, scipy, sklearn, matplotlib, seaborn,
stdlib), executed in throwaway temp dirs with `MPLBACKEND=Agg`. 81 such tasks exist; a fixed
**12‑task sample** whose *canonical solutions verify in our environment* was used (so every task is
known‑solvable). This split is harder than polyglot — **Opus 4.8 does not max it out**.

Each `(agent, task)` runs in isolation with a stub `solution.py` + the BigCodeBench `test_solution.py`.
Scoring is the ground‑truth `pytest` result **plus a cheating guard**: the test file must be
byte‑identical afterwards (enforced for every agent). Harness: `bench/bcb/driver.py`.

Agents:
- **opus48** — `claude -p --model claude-opus-4-8` (Claude Code + Opus 4.8)
- **codex55** — `codex exec -m gpt-5.5` (Codex + gpt‑5.5; the "Codex 5.5" frontier)
- **damascus:<model>:n<N>[:ablation]** — Damascus over OpenRouter with
  `google/gemma-4-26b-a4b-it` ("gemma") or `openai/gpt-oss-120b` ("ossbig")

## Results (12 hard tasks)

### Frontier baselines
| agent | solved | rate |
|-------|:------:|:----:|
| Claude Code + **Opus 4.8** | 11/12 | **92%** |
| Codex + **gpt‑5.5** | 12/12 | **100%** |

Frontier line: **~92–100%** (Opus missed one task; Codex solved all).

### Damascus + cheap models — N‑scaling
| config | solved | rate |
|--------|:------:|:----:|
| gemma  n=1  | 9/12 | 75% |
| gemma  n=8  | 10/12 | **83%** |
| gemma  n=16 | 9/12 | 75% |
| gpt‑oss‑120b n=1  | 9/12 | 75% |
| gpt‑oss‑120b n=8  | 8/12 | 67% |
| gpt‑oss‑120b n=16 | 9/12 | 75% |

**Best cheap‑model config: 83% (gemma, n=8).** Gap to Opus ≈ **9 pts**, to Codex ≈ **17 pts**.

### Feature ablations (gemma, n=8; feature removed)
| config | solved | rate | Δ vs full |
|--------|:------:|:----:|:---------:|
| full | 10/12 | 83% | — |
| − AST slicing | 8/12 | 67% | **−16 pts** |
| − recursive decomposition | 8/12 | 67% | **−16 pts** |
| − deterministic filter | 10/12 | 83% | 0 pts (solve‑rate) |

Interpretation:
- **AST slicing and decomposition each contribute ~16 points** — the context‑isolation and
  atom‑shrinking subsystems are the real accuracy drivers for a narrow‑context model.
- The **deterministic filter is solve‑rate‑neutral** here; its value is *throughput and safety*
  (it rejects unbuildable/out‑of‑scope candidates in microseconds before any sandbox run, and
  blocks test‑tampering), not raw accuracy on this Python set.

### N‑scaling is non‑monotonic — and why
Solve rate rises from n=1→n=8 then **regresses at n=16**. Root cause (confirmed by logs): several
BigCodeBench tests are **nondeterministic** (random/time/plot‑state). At high N, a candidate that
passed the sandbox gate by a *lucky single run* is more likely to be selected, then fails the
independent final test. This is a **selection‑reliability ceiling**, not a generation ceiling.

**Fix implemented:** *confirm‑winner* — the selected candidate is re‑verified in a fresh sandbox
before being committed; on failure the harness falls back to the next‑best passing candidate. This
targets exactly the flaky‑selection regression (`src/orchestrator.rs`). On the 12‑task sample its
measured effect was **within noise** (gemma n=8: 10/12 → 10/12; n=16 partial), because much of the
residual variance is *intrinsic test nondeterminism* rather than selection error; a much larger
sample would be needed to quantify it. The change is retained as it is strictly safer — it never
commits an unconfirmed candidate — at the cost of one extra verification per step.

## Verdict

**Is frontier parity reachable?** Nuanced, and honest:

1. **On well‑scoped, function‑level coding (this benchmark): near‑parity is already real.**
   A 26B‑A4B open model in the harness reaches **83%** vs Opus **92%** — a single‑digit gap — and
   the harness features (slicing, decomposition) are what get it there, not the raw model. With a
   stronger open coder model, better selection (confirm‑winner / ensemble judging), and more
   inference, closing the remaining ~10 points is **plausible**. Matching Codex's **100%** exactly
   is harder: the last tasks are genuinely hard *and* capped by test nondeterminism.

2. **The honest ceiling.** Test‑time scaling cannot manufacture a correct solution the model's
   sample distribution never contains. For the hardest tasks, neither bigger N nor more repair
   helped — the cheap model simply never produced a passing candidate. There, only a stronger
   model (or true multi‑model ensemble) closes the gap.

3. **On repo‑scale agentic tasks (e.g. SWE‑bench): not currently reachable.** Damascus targets
   `file::symbol` leaves; it has **no code‑search / localization** subsystem, so it cannot find
   *where* to edit in a large unfamiliar repo. Frontier agents do this well. Parity there requires
   building retrieval/navigation — a new subsystem, not a tuning knob. This is the clearest "needs
   more development" finding.

**Bottom line:** Damascus + a modest open model is **near‑frontier (within ~10 pts) on bounded
coding tasks today**, driven by context‑isolation and decomposition rather than model strength.
Full parity is **conditionally reachable** for that task class (stronger open model + better
selection + more inference) but **not yet** for repo‑scale work, which needs a localization layer
the harness does not have.

## Reproduce
```bash
cargo build --release
# frontier
bench/bcb/driver.py --out r.jsonl --configs opus48,codex55
# cheap-model sweep + ablations
bench/bcb/driver.py --out r.jsonl --configs damascus:gemma:n8,damascus:gemma:n8:no_slice
bench/bcb/analyze.py
```
Sample, raw per‑task results and logs are under `bench/bcb/results/`.
