#!/usr/bin/env python3
"""BigCodeBench-Hard agent benchmark driver (safe-lib subset, no Docker).

Each (agent, task) runs in an isolated temp dir with a stub `solution.py`
(function signature + docstring) and `test_solution.py` (the BigCodeBench tests).
The agent must implement `task_func`. Scoring = pytest passes AND the test file
is byte-identical afterwards (cheating guard). MPLBACKEND=Agg for headless plots.

Configs identical to the polyglot driver:
  opus48 | codex55 | damascus:<model>:n<N>[:no_slice|no_filter|no_decompose|r<R>]
"""
import argparse, hashlib, json, os, shutil, subprocess, time, tempfile
from pathlib import Path

VENV_PY = "/tmp/bench-venv/bin/python"
TASKS = {t["task_id"]: t for t in (json.loads(l) for l in open("/tmp/bcb_hard_safe.jsonl"))}
SAMPLE = json.load(open("/tmp/bcb_sample.json"))  # validated task ids

OPENROUTER_MODELS = {"gemma": "google/gemma-4-26b-a4b-it", "ossbig": "openai/gpt-oss-120b"}


def envx():
    e = dict(os.environ)
    e["MPLBACKEND"] = "Agg"
    return e


def sh(cmd, cwd, timeout):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, timeout=timeout, env=envx(),
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        rc = p.returncode
    except subprocess.TimeoutExpired:
        rc = -1
    return rc, time.time() - t0


def pytest_passes(work):
    p = subprocess.run([VENV_PY, "-m", "pytest", "-q", "test_solution.py"],
                       cwd=work, env=envx(), stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=120)
    return p.returncode == 0


def fhash(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def setup(task):
    work = Path(tempfile.mkdtemp(prefix="bcb-"))
    stub = task["complete_prompt"].rstrip("\n") + "\n    pass\n"
    (work / "solution.py").write_text(stub)
    (work / "test_solution.py").write_text("from solution import *\n\n" + task["test"])
    return work


def prompt_for(task):
    return ("Implement the function `task_func` in solution.py so that the tests in "
            "test_solution.py pass. Run the tests to check. Do NOT modify the test file.\n\n"
            "Specification:\n" + task["complete_prompt"])


def write_toml(work, model, n, repair):
    (work / "damascus.toml").write_text(f"""
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
[models]
planner  = "openrouter/{model}"
drafter  = "openrouter/{model}"
judge    = "openrouter/{model}"
repairer = "openrouter/{model}"
[scaling]
candidates = {n}
repair_rounds = {repair}
max_recursion = 1
max_steps = 6
concurrency = 12
[verify]
test = "{VENV_PY} -m pytest -q test_solution.py"
timeout_secs = 90
""")


def build_cmd(config, work, prompt):
    if config == "opus48":
        return ["claude", "-p", "--model", "claude-opus-4-8",
                "--permission-mode", "bypassPermissions", prompt]
    if config == "codex55":
        return ["codex", "exec", "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox", "-m", "gpt-5.5", prompt]
    if config.startswith("damascus:"):
        parts = config.split(":")
        model = OPENROUTER_MODELS[parts[1]]
        n, repair, flags = 8, 1, []
        for tok in parts[2:]:
            if tok in ("no_slice", "no_filter", "no_decompose"):
                flags.append("--" + tok.replace("_", "-"))
            elif tok.startswith("n") and tok[1:].isdigit():
                n = int(tok[1:])
            elif tok.startswith("r") and tok[1:].isdigit():
                repair = int(tok[1:])
        write_toml(work, model, n, repair)
        return ["damascus", "run", prompt, "--yes", "--quiet"] + flags
    raise ValueError(config)


def run_one(config, tid, timeout):
    task = TASKS[tid]
    work = setup(task)
    try:
        before = fhash(work / "test_solution.py")
        rc, secs = sh(build_cmd(config, work, prompt_for(task)), str(work), timeout)
        tampered = fhash(work / "test_solution.py") != before
        solved = (not tampered) and pytest_passes(str(work))
        return {"config": config, "task": tid, "solved": int(solved),
                "seconds": round(secs, 1), "tampered": int(tampered)}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--configs", required=True)
    ap.add_argument("--sample", type=int, default=len(SAMPLE))
    ap.add_argument("--timeout", type=int, default=300)
    a = ap.parse_args()
    sample = SAMPLE[: a.sample]
    out = open(a.out, "a")
    for config in a.configs.split(","):
        if not config:
            continue
        solved = 0
        for tid in sample:
            r = run_one(config, tid, a.timeout)
            solved += r["solved"]
            out.write(json.dumps(r) + "\n"); out.flush()
            mark = "PASS" if r["solved"] else ("TAMPER" if r["tampered"] else "fail")
            print(f"  {config:38s} {tid:18s} {mark:6s} {r['seconds']:6.1f}s", flush=True)
        print(f"== {config}: {solved}/{len(sample)} ==", flush=True)
    out.close()


if __name__ == "__main__":
    main()
