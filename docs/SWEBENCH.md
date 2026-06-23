# Damascus on real SWE-bench Verified (official benchmark)

This is the **official, public code-patch benchmark** (SWE-bench Verified — the public proxy for the
AA index's DeepSWE / long-horizon SWE family), not the homemade LiveCodeBench set. Run with the
official `swebench` evaluator (v4.1.0) in Docker.

## Setup (real, reproducible)
- Docker installed; `swebench` harness validated: **gold patches resolve** on a flask/requests subset.
- Of 8 sampled instances, **4 are gold-resolvable** in this environment (the others fail even with
  the correct patch — known env flakiness for some old `requests` instances), so the fair denominator
  is those 4: `pallets__flask-5014`, `psf__requests-1142`, `psf__requests-2931`, `psf__requests-5414`.

## Damascus SWE adapter (`bench/swe/run.py`) — general, not instance-tuned
Per instance: clone the repo @ `base_commit`, install it in a throwaway venv, **localize** the buggy
file(s) from the issue (excluding test files), write a **self-reproduction test** as the verify gate
(SWE-bench hides the real tests), run the **Damascus Fold Loop** against that gate, and emit the git
diff. A **direct-fix fallback** (full-file rewrite for small files, search/replace for large ones)
produces a patch when the gated loop yields nothing. Patches are scored by the official evaluator.

## Result (gold-resolvable subset, n=4)
| Run | Resolved | Notes |
|-----|:--------:|-------|
| Damascus v1 | **0/4** | self-repro gate never passed; direct-fix failed on large files; one localization hit a test file |
| **Damascus v2** | **1/4 (25%)** | after fixing localization (exclude tests) + robust direct-fix. Resolved `psf__requests-5414` |

For reference, frontier agents (Codex/Claude Code) score **~65–75% on the full SWE-bench Verified**
(published). So on repo-scale agentic SWE, Damascus + open models is **far below frontier** — but
**not zero**: it genuinely resolved a real Verified instance.

## Honest analysis — why the gap, and what it would take
SWE-bench is exactly the regime Damascus was *not* built for, and the run makes the missing pieces
concrete:
1. **Localization is unreliable** with weak open models — it sometimes picks `setup.py` or a test
   file instead of the buggy source. Frontier agents localize well; Damascus needs a real
   retrieval/ranking layer (the `ask`-mode `RepoIndex` is a start but not enough).
2. **No real failing-test signal.** Damascus's power is its objective verify gate, but SWE-bench
   hides the tests. The agent must *write its own* reproduction, and weak models write unreliable
   ones — so the gate rarely fires and the Fold Loop has nothing to select on. This is the core
   mismatch: Damascus excels when given tests; SWE-bench withholds them.
3. **Iterative repo-scale debugging** (read error → navigate → edit → rerun across many turns) is
   what frontier agents do; Damascus does bounded best-of-N on a localized leaf.

**Verdict:** the official benchmark confirms the long-stated boundary — Damascus + cheap open models
ties frontier *medium* harnesses on **bounded, test-provided** coding (LiveCodeBench: 62% = Opus 4.8)
but is **far behind on repo-scale agentic SWE** (SWE-bench Verified: ~25% on a tiny subset vs frontier
~70%). Closing it is not tuning — it needs real **localization + reproduction-synthesis +
iterative-debug** subsystems, which is a research-grade build, plus likely a stronger base model for
the hardest localizations.

## Reproduce
```bash
# install: docker + swebench (see /tmp/swebench-venv), dataset princeton-nlp/SWE-bench_Verified
OPENROUTER_API_KEY=... SWE_N=8 python bench/swe/run.py        # writes predictions
python -m swebench.harness.run_evaluation \
  --dataset_name princeton-nlp/SWE-bench_Verified \
  --predictions_path swe_preds.jsonl --run_id damascus --max_workers 3
```
Raw predictions/reports: `bench/swe/results/`.
