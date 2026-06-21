#!/usr/bin/env python3
"""Unified coding-agent benchmark driver over the aider polyglot (Exercism) set.

Each (agent, problem) runs in an isolated copy containing only the stub, the
test, and the instructions. The agent must make the tests pass. Scoring is the
ground truth `pytest` result AND a cheating guard: the test file must be
byte-identical afterwards (no agent is allowed to edit the tests).

Agents are invoked headless:
  - opus48      : claude -p --model claude-opus-4-8 --permission-mode bypassPermissions
  - codex55     : codex exec --dangerously-bypass-approvals-and-sandbox -m gpt-5.5
  - gjc55       : gjc -p --model gpt-5.5 --yolo
  - damascus:... : damascus run (config built per spec: model, N, ablations)

Usage:
  driver.py --polyglot /tmp/polyglot-benchmark --out results.jsonl \
            --configs opus48,codex55,damascus:gemma:n8 [--sample N] [--timeout 360]
"""
import argparse, hashlib, json, os, shutil, subprocess, sys, time, tempfile
from pathlib import Path

VENV_PY = "/tmp/bench-venv/bin/python"

# Fixed HARD sample (Python exercises): constraint solving, interpreters,
# parsers, reactive systems, game logic. Even frontier models do not max these.
SAMPLE = [
    "zebra-puzzle", "react", "forth", "sgf-parsing", "pov",
    "rest-api", "dominoes", "food-chain", "connect", "two-bucket",
]

OPENROUTER_MODELS = {
    "gemma": "google/gemma-4-26b-a4b-it",
    "ossbig": "openai/gpt-oss-120b",
}


def sh(cmd, cwd, timeout, env=None):
    t0 = time.time()
    try:
        p = subprocess.run(cmd, cwd=cwd, timeout=timeout, env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        rc = p.returncode
    except subprocess.TimeoutExpired:
        rc = -1
    return rc, time.time() - t0


def pytest_passes(workdir, test_file):
    p = subprocess.run([VENV_PY, "-m", "pytest", "-q", test_file],
                       cwd=workdir, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return p.returncode == 0


def file_hash(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def setup_problem(polyglot, ex):
    src = Path(polyglot) / "python" / "exercises" / "practice" / ex
    stub = next(f for f in src.glob("*.py")
                if not f.name.endswith("_test.py") and f.parent == src)
    test = next(src.glob("*_test.py"))
    instr = src / ".docs" / "instructions.md"
    work = Path(tempfile.mkdtemp(prefix=f"poly-{ex}-"))
    shutil.copy(stub, work / stub.name)
    shutil.copy(test, work / test.name)
    instructions = instr.read_text() if instr.exists() else ""
    return work, stub.name, test.name, instructions


def prompt_for(stub, test, instructions):
    return (f"Implement the solution in {stub} so that the tests in {test} pass. "
            f"Run the tests to check your work. Do NOT modify {test} or any test file.\n\n"
            f"Instructions:\n{instructions}")


def write_damascus_toml(work, model_id, candidates, repair, test_file):
    (work / "damascus.toml").write_text(f"""
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
[models]
planner  = "openrouter/{model_id}"
drafter  = "openrouter/{model_id}"
judge    = "openrouter/{model_id}"
repairer = "openrouter/{model_id}"
[scaling]
candidates = {candidates}
repair_rounds = {repair}
max_recursion = 1
max_steps = 6
concurrency = 12
[verify]
test = "{VENV_PY} -m pytest -q {test_file}"
timeout_secs = 90
""")


def build_cmd(config, work, stub, test, prompt):
    """Return (argv, env, label) for a config string."""
    env = dict(os.environ)
    if config == "opus48":
        return (["claude", "-p", "--model", "claude-opus-4-8",
                 "--permission-mode", "bypassPermissions", prompt], env)
    if config == "codex55":
        return (["codex", "exec", "--skip-git-repo-check",
                 "--dangerously-bypass-approvals-and-sandbox", "-m", "gpt-5.5", prompt], env)
    if config == "gjc55":
        return (["gjc", "-p", "--model", "gpt-5.5", "--yolo", prompt], env)
    if config.startswith("damascus:"):
        # damascus:<model>:n<N>[:no_slice|no_filter|no_decompose|r<rounds>]
        parts = config.split(":")
        model = OPENROUTER_MODELS[parts[1]]
        n = 8
        repair = 1
        flags = []
        for tok in parts[2:]:
            if tok.startswith("n"):
                n = int(tok[1:])
            elif tok.startswith("r"):
                repair = int(tok[1:])
            elif tok in ("no_slice", "no_filter", "no_decompose"):
                flags.append("--" + tok.replace("_", "-"))
        write_damascus_toml(work, model, n, repair, test)
        return (["damascus", "run", prompt, "--yes", "--quiet"] + flags, env)
    raise ValueError(f"unknown config {config}")


def run_one(config, polyglot, ex, timeout):
    work, stub, test, instructions = setup_problem(polyglot, ex)
    try:
        test_hash_before = file_hash(work / test)
        prompt = prompt_for(stub, test, instructions)
        argv, env = build_cmd(config, work, stub, test, prompt)
        rc, secs = sh(argv, str(work), timeout, env)
        tampered = file_hash(work / test) != test_hash_before
        solved = (not tampered) and pytest_passes(str(work), test)
        return {"config": config, "exercise": ex, "solved": int(solved),
                "seconds": round(secs, 1), "rc": rc, "tampered": int(tampered)}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--polyglot", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--configs", required=True, help="comma-separated config specs")
    ap.add_argument("--sample", type=int, default=len(SAMPLE))
    ap.add_argument("--timeout", type=int, default=360)
    args = ap.parse_args()

    sample = SAMPLE[: args.sample]
    configs = [c for c in args.configs.split(",") if c]
    out = open(args.out, "a")
    for config in configs:
        solved = 0
        for ex in sample:
            r = run_one(config, args.polyglot, ex, args.timeout)
            solved += r["solved"]
            out.write(json.dumps(r) + "\n"); out.flush()
            mark = "PASS" if r["solved"] else ("TAMPER" if r["tampered"] else "fail")
            print(f"  {config:38s} {ex:14s} {mark:6s} {r['seconds']:6.1f}s", flush=True)
        print(f"== {config}: {solved}/{len(sample)} ==", flush=True)
    out.close()


if __name__ == "__main__":
    main()
