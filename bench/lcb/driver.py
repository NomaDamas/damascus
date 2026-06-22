#!/usr/bin/env python3
"""LiveCodeBench (hard, stdin) agent benchmark — recent AtCoder problems that
break frontier models. Safe: solutions are stdin->stdout programs run with a
per-case timeout. Scoring = all test cases pass AND the test harness/cases are
byte-identical afterwards (cheating guard).

Configs: opus48 | codex55 | damascus:<model>:n<N>[:ablations]
Models: gemma, ossbig, kimi (k2.7-code), glm (5.2).
"""
import argparse, hashlib, json, os, shutil, subprocess, time, tempfile
from pathlib import Path

VENV_PY = "/tmp/bench-venv/bin/python"
SAMPLE = json.load(open("/tmp/lcb_sample.json"))

# Allowed open models only (no Chinese models per project policy).
OPENROUTER_MODELS = {
    "gemma": "google/gemma-4-26b-a4b-it",
    "gemma31": "google/gemma-4-31b-it",
    "ossbig": "openai/gpt-oss-120b",
    "nemotron": "nvidia/nemotron-3-ultra-550b-a55b",
    "nemosuper": "nvidia/nemotron-3-super-120b-a12b",
}

RUN_TESTS = r'''import json, subprocess, sys
cases = json.load(open("cases.json"))
for i, c in enumerate(cases):
    try:
        p = subprocess.run([sys.executable, "solution.py"], input=c["input"],
                           capture_output=True, text=True, timeout=8)
    except subprocess.TimeoutExpired:
        print(f"case {i}: TIMEOUT"); sys.exit(1)
    if p.stdout.strip() != c["output"].strip():
        print(f"case {i}: MISMATCH"); sys.exit(1)
print("all pass"); sys.exit(0)
'''


def sh(cmd, cwd, timeout):
    t0 = time.time()
    try:
        subprocess.run(cmd, cwd=cwd, timeout=timeout, env=dict(os.environ),
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        pass
    return time.time() - t0


def fhash(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def setup(prob):
    work = Path(tempfile.mkdtemp(prefix="lcb-"))
    (work / "solution.py").write_text("# Read input from stdin, print the answer to stdout.\n")
    (work / "cases.json").write_text(json.dumps(prob["cases"]))
    (work / "run_tests.py").write_text(RUN_TESTS)
    (work / "problem.md").write_text(prob["question"])
    return work


def prompt_for(prob):
    return ("Write a complete Python 3 program in solution.py that reads from standard input "
            "and writes the answer to standard output, solving the problem below. It must run as "
            "`python solution.py`. Output only via stdout.\n\n" + prob["question"])


def scores(work):
    p = subprocess.run([VENV_PY, "run_tests.py"], cwd=work,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=200)
    return p.returncode == 0


ENSEMBLE = ["openai/gpt-oss-120b", "google/gemma-4-31b-it"]


def write_toml(work, model, n, repair):
    if model == "ensemble":
        plan = ENSEMBLE[0]
        drafters = "[" + ", ".join(f'"openrouter/{m}"' for m in ENSEMBLE) + "]"
    else:
        plan = model
        drafters = f'["openrouter/{model}"]'
    (work / "damascus.toml").write_text(f"""
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
[models]
planner  = "openrouter/{plan}"
drafter  = "openrouter/{plan}"
judge    = "openrouter/{plan}"
repairer = "openrouter/{plan}"
drafters = {drafters}
[scaling]
candidates = {n}
repair_rounds = {repair}
max_recursion = 1
max_steps = 4
concurrency = 12
[verify]
test = "{VENV_PY} run_tests.py"
timeout_secs = 120
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
        model = "ensemble" if parts[1] == "ensemble" else OPENROUTER_MODELS[parts[1]]
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


def run_one(config, prob, timeout):
    work = setup(prob)
    try:
        guard = fhash(work / "run_tests.py") + fhash(work / "cases.json")
        secs = sh(build_cmd(config, work, prompt_for(prob)), str(work), timeout)
        tampered = (fhash(work / "run_tests.py") + fhash(work / "cases.json")) != guard
        solved = (not tampered) and scores(str(work))
        return {"config": config, "task": prob["id"], "solved": int(solved),
                "seconds": round(secs, 1), "tampered": int(tampered)}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--configs", required=True)
    ap.add_argument("--sample", type=int, default=len(SAMPLE))
    ap.add_argument("--timeout", type=int, default=400)
    a = ap.parse_args()
    sample = SAMPLE[: a.sample]
    out = open(a.out, "a")
    for config in a.configs.split(","):
        if not config:
            continue
        solved = 0
        for prob in sample:
            r = run_one(config, prob, a.timeout)
            solved += r["solved"]
            out.write(json.dumps(r) + "\n"); out.flush()
            mark = "PASS" if r["solved"] else ("TAMPER" if r["tampered"] else "fail")
            print(f"  {config:34s} {r['task']:14s} {mark:6s} {r['seconds']:6.1f}s", flush=True)
        print(f"== {config}: {solved}/{len(sample)} ==", flush=True)
    out.close()


if __name__ == "__main__":
    main()
