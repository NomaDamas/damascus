# Strategies for maximizing fast/cheap/small models in coding

A complete, honest inventory of the techniques considered for getting frontier-grade coding out of
small, fast, cheap models — what is implemented, what was tested, results, and what was deferred and
**why**. Hard rule throughout: **Damascus is a general-purpose harness. No strategy may overfit to a
specific benchmark** (no problem-specific logic, no peeking at test internals, no per-task tuning).
Every feature works on an arbitrary repo with an arbitrary user `verify` command.

## Implemented & validated

| Strategy | What it does | Result | General? |
|---|---|---|---|
| **Best-of-N sampling** | N candidate edit-sets per step | core lever; realizes the model's pass@N ceiling | ✅ |
| **Objective verify gate** | accept only if build/check/lint pass | the forcing function; no self-certification | ✅ |
| **AST sub-file slicing** | give the model only the target def + dep signatures | **+16 pts** (ablated) | ✅ |
| **Recursive decomposition** | split a hard step into verifiable sub-steps | **+16 pts** (ablated) | ✅ |
| **Deterministic 3-stage filter** | apply→syntax(parser)→contract before sandbox | rejects garbage fast, no LLM; throughput/safety | ✅ |
| **Reflexion repair** | feed failure log back, resample | turns near-misses into passes | ✅ |
| **Confirm-winner re-verify** | re-verify the selected candidate before commit | kills flaky single-pass selection | ✅ |
| **Multi-model ensemble** | spread best-of-N across a model pool | helps **only** with complementary, comparably-strong members | ✅ |
| **Even-spread temperature** | distribute N samples across [t, explore_t] | fixed a bug where high-N piled hot samples and *regressed* | ✅ |
| **Early-exit on first pass** | stop verifying once one candidate passes | big time/cost cut (exploits speed), no quality loss on objective gates | ✅ |
| **Sequential refinement** | repair includes the closest failing attempt + "diagnose the cause" | builds on the nearest miss instead of resampling cold | ✅ |
| **pass@k measurement** | quantify whether a solution exists in the model's distribution | diagnostic; revealed the true walls | ✅ |

## Tested and rejected (honest negative results)

| Strategy | Why rejected |
|---|---|
| **Reason-first / inline CoT prompt** | Regressed GLM 38%→25% and caused a stdin-wait hang; reverted. OpenCode's "medium→max" lift is the whole feedback loop, not a one-line prompt. |
| **Bigger model (Nemotron-3-Ultra-550B)** | *Worse* than gemma-4-31b (38% vs 62%): slow, TLE-prone. Bigger ≠ better. |
| **Ensemble with a weak/slow member** | Dragged the pool *below* the best single model (round-robins budget onto the weak member). |
| **Raising N past ~8 (this set)** | Plateaus: the residual problems are `pass@64=0` (no sample is ever correct). N raises reliability on p>0 problems, never crosses p=0. |

## Designed but deferred (with reasons)

| Strategy | Why deferred |
|---|---|
| **Cascade / router (cheap→escalate)** | Sound in general, but **no allowed open model is stronger than gemma-4-31b** (the 550B is worse), so there is nothing to escalate *to* — zero benefit today. Worth adding when a genuinely stronger tier exists; not worth refactoring a working harness for no gain. |
| **Problem-specific hard decomposition** | The only harness lever that could cross a `pass@0` wall, but decomposing a single competitive problem into verifiable sub-pieces generically (without per-problem logic) is unsolved — and a problem-specific version would be **overfitting**, which is forbidden. |
| **LSP-diagnostic feedback tool** | High-value (OpenCode's edge) but a larger build; the user `verify` command already surfaces compiler output into repair today. |
| **Test-generation to strengthen the gate** | General, but can also reject correct solutions; medium value, needs care. |
| **Few-shot exemplars** | Low risk but adds tokens and rarely helps on novel competitive problems. |

## The honest conclusion

On the Implement family, **Damascus already realizes the model's full achievable ceiling efficiently**
(gemma-4-31b: raw single-shot ~48% → Damascus 62% = pass@8-oracle = Opus 4.8), and does so faster
than general agents (which flounder with a weak model). The remaining gap to the strongest frontier
(Codex 100%) is **model capability on the hardest problems** (`pass@64=0` for every allowed open
model), not a missing harness trick. The levers that remain are therefore *model-side* (a stronger
allowed open model) or *a general decomposition that provably raises p without overfitting* — an open
research problem, not a tuning knob.
