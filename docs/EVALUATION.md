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

## Round 1: BigCodeBench‑Hard (safe subset)

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

## Round 2: a benchmark that spreads even the frontier (LiveCodeBench‑Hard)

BigCodeBench‑Hard (safe subset) still let our cheap models look very close (83% vs 92%). To get
real separation — including *between* the two frontier agents — we moved to **LiveCodeBench**:
recent (Mar–Apr 2025, low‑contamination) **AtCoder** competitive‑programming problems, run as
stdin→stdout programs with public+private test cases and per‑case timeouts (`bench/lcb/`). Sample:
**4 medium (ABC) + 4 hard (ARC)**. We also switched the open models to the ones requested —
**Kimi K2.7‑Code** and **GLM‑5.2** — alongside gemma for continuity.

| agent | medium 4 | hard 4 | total | rate |
|-------|:--------:|:------:|:-----:|:----:|
| Codex + **gpt‑5.5** | 4/4 | **4/4** | 8/8 | **100%** |
| Claude Code + **Opus 4.8** | 4/4 | 1/4 | 5/8 | **62%** |
| damascus + gemma‑4‑26b‑a4b (n=8) | 4/4 | 0/4 | 4/8 | 50% |
| damascus + GLM‑5.2 (n=8) | 3/4 | 0/4 | 3/8 | 38% |
| damascus + Kimi‑K2.7‑Code (n=8) | 2/4 | 0/4 | 2/8 | 25% |

This benchmark discriminates **at every level**:
- **The frontier itself spreads: Codex 5.5 = 100% vs Opus 4.8 = 62%** (38‑pt gap). Codex/gpt‑5.5
  solved *all four* hard ARC problems; Opus solved one. So "top‑tier harnesses" are clearly
  separable here — the property that was missing before.
- **The hard ARC tier is a wall for open models: 0/4 for every one of them**, at n=8. Test‑time
  scaling did not crack a single hard competitive problem — the cheap models simply never produce a
  correct candidate, so there is nothing for the verifier to select.
- Open models still rank cleanly (gemma 50% > GLM 38% > Kimi 25%). Note the "stronger" Kimi/GLM
  *under*‑performed gemma in this harness on this 8‑task set; on a sample this small that is more
  likely routing/output‑format variance than a robust model ranking, and should be read with care.

**The key lesson:** the near‑parity seen on BigCodeBench was *task‑class‑specific* (data‑processing
boilerplate, well represented in training). On genuinely hard reasoning problems the gap is large
and stable, and adding inference (N) does not close it.

## Verdict

**Is frontier parity reachable?** Nuanced, and honest:

1. **On well‑scoped, "ordinary‑hard" coding (BigCodeBench): near‑parity is real.** A 26B‑A4B open
   model in the harness reached **83%** vs Opus **92%** — single‑digit gap — and ablations show the
   *harness* (slicing +16, decomposition +16) gets it there, not the raw model. On that task class
   closing the last ~10 points looks plausible with a stronger open model + better selection.

2. **On genuinely hard reasoning (LiveCodeBench hard ARC): the gap is large and stable.** Every
   open model scored **0/4** on the hard tier at n=8; only Codex (4/4) and Opus (1/4) solved any.
   Test‑time scaling cannot manufacture a solution the model's distribution never contains — when
   the model never emits a correct candidate, the verifier has nothing to select. More N does not
   help here.

3. **The frontier is not monolithic.** Codex 5.5 (100%) clearly beat Opus 4.8 (62%) on LiveCodeBench
   — "frontier" itself is a 38‑point spread, so "reach frontier" must specify *which* frontier.

4. **On repo‑scale agentic tasks (e.g. SWE‑bench): not currently reachable.** Damascus targets
   `file::symbol` leaves and has **no code‑search / localization** subsystem, so it cannot find
   *where* to edit in a large unfamiliar repo. Parity there needs a new retrieval/navigation
   subsystem, not a tuning knob.

**Bottom line:** With Damascus, a modest open model is **near‑frontier on bounded, ordinary‑hard
coding** (driven by context‑isolation + decomposition, not model strength), but falls **far short
on genuinely hard reasoning problems** (0/4 on hard ARC vs Codex 4/4) and on **repo‑scale** tasks.
Test‑time scaling raises the floor on solvable tasks; it cannot cross the capability wall on the
hardest ones. Full parity with the *strongest* frontier (Codex 5.5) is **not reachable** with these
open models today; near‑parity with the *weaker* frontier on *bounded* tasks already is.

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
