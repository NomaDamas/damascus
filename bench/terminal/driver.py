#!/usr/bin/env python3
"""Terminal/agentic family (Terminal-Bench analog, safe local proxy).

Each task seeds input files in an isolated dir; the agent must write solve.py
that processes them and prints the answer. Scoring = stdout matches expected AND
the checker/inputs are byte-identical afterwards (cheating guard).

Configs identical to bench/lcb/driver.py.
"""
import argparse, hashlib, json, os, shutil, subprocess, time, tempfile
from pathlib import Path

HERE = Path(__file__).parent
VENV_PY = "/tmp/bench-venv/bin/python"
TASKS = json.load(open(HERE / "tasks.json"))
OPENROUTER_MODELS = {"gemma": "google/gemma-4-26b-a4b-it", "ossbig": "openai/gpt-oss-120b",
                     "kimi": "moonshotai/kimi-k2.7-code", "glm": "z-ai/glm-5.2"}
ENSEMBLE = ["openai/gpt-oss-120b", "moonshotai/kimi-k2.7-code", "z-ai/glm-5.2",
            "google/gemma-4-26b-a4b-it"]

RUN_TESTS = r'''import json, subprocess, sys
spec = json.load(open(".task.json"))
stdin = spec.get("stdin", "")
try:
    p = subprocess.run([sys.executable, "solve.py"], input=stdin, capture_output=True, text=True, timeout=15)
except subprocess.TimeoutExpired:
    print("TIMEOUT"); sys.exit(1)
if p.stdout.strip() != spec["expected"].strip():
    print("MISMATCH"); sys.exit(1)
print("ok"); sys.exit(0)
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


def setup(task):
    work = Path(tempfile.mkdtemp(prefix="term-"))
    for name, content in task.get("files", {}).items():
        (work / name).write_text(content)
    (work / "solve.py").write_text("# Read the input file(s) described in the task; print the answer to stdout.\n")
    (work / "run_tests.py").write_text(RUN_TESTS)
    (work / ".task.json").write_text(json.dumps({"expected": task["expected"], "stdin": task.get("stdin", "")}))
    return work


def prompt_for(task):
    extra = f"\nYour program will receive this on standard input: {task['stdin']!r}" if task.get("stdin") else ""
    return (task["instructions"] + extra +
            "\n\nWrite the program in solve.py. Do not modify any other file.")


def passes(work):
    p = subprocess.run([VENV_PY, "run_tests.py"], cwd=work,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    return p.returncode == 0


def write_toml(work, model, n):
    drafters = ("[" + ", ".join(f'"openrouter/{m}"' for m in ENSEMBLE) + "]") if model == "ensemble" \
        else f'["openrouter/{model}"]'
    plan = ENSEMBLE[0] if model == "ensemble" else model
    (work / "damascus.toml").write_text(f"""
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
[models]
planner = "openrouter/{plan}"
drafter = "openrouter/{plan}"
judge = "openrouter/{plan}"
repairer = "openrouter/{plan}"
drafters = {drafters}
[scaling]
candidates = {n}
repair_rounds = 1
max_recursion = 1
max_steps = 3
concurrency = 12
[verify]
test = "{VENV_PY} run_tests.py"
timeout_secs = 60
""")


def build_cmd(config, work, prompt):
    if config == "opus48":
        return ["claude", "-p", "--model", "claude-opus-4-8", "--permission-mode", "bypassPermissions", prompt]
    if config == "codex55":
        return ["codex", "exec", "--skip-git-repo-check", "--dangerously-bypass-approvals-and-sandbox", "-m", "gpt-5.5", prompt]
    if config.startswith("damascus:"):
        parts = config.split(":")
        model = "ensemble" if parts[1] == "ensemble" else OPENROUTER_MODELS[parts[1]]
        n = next((int(t[1:]) for t in parts[2:] if t.startswith("n") and t[1:].isdigit()), 8)
        write_toml(work, model, n)
        return ["damascus", "run", prompt, "--yes", "--quiet"]
    raise ValueError(config)


def run_one(config, task, timeout):
    work = setup(task)
    try:
        guard = fhash(work / "run_tests.py") + fhash(work / ".task.json")
        secs = sh(build_cmd(config, work, prompt_for(task)), str(work), timeout)
        tampered = (fhash(work / "run_tests.py") + fhash(work / ".task.json")) != guard
        solved = (not tampered) and passes(str(work))
        return {"config": config, "task": task["id"], "solved": int(solved),
                "seconds": round(secs, 1), "tampered": int(tampered)}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--configs", required=True)
    ap.add_argument("--timeout", type=int, default=200)
    a = ap.parse_args()
    out = open(a.out, "a")
    for config in a.configs.split(","):
        if not config:
            continue
        solved = 0
        for task in TASKS:
            r = run_one(config, task, a.timeout)
            solved += r["solved"]
            out.write(json.dumps(r) + "\n"); out.flush()
            mark = "PASS" if r["solved"] else ("TAMPER" if r["tampered"] else "fail")
            print(f"  {config:34s} {r['task']:16s} {mark:6s} {r['seconds']:6.1f}s", flush=True)
        print(f"== {config}: {solved}/{len(TASKS)} ==", flush=True)
    out.close()


if __name__ == "__main__":
    main()
