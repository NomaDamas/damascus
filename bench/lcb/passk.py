#!/usr/bin/env python3
"""Directly measure pass@k for open models on hard problems: does the model's
sample distribution contain ANY correct solution? If pass@N>0 then a verify-gated
harness (like Damascus) would solve it given enough N; if it stays 0, no amount of
test-time scaling helps. Concurrent sampling, local test execution.

Usage: passk.py --model google/gemma-4-26b-a4b-it --n 64 --tasks arc196_a,arc196_c
"""
import argparse, json, os, re, subprocess, tempfile, urllib.request, concurrent.futures as cf

KEY = os.environ["OPENROUTER_API_KEY"]
SAMPLE = {o["id"]: o for o in json.load(open("/tmp/lcb_sample.json"))}


def sample_one(model, prompt, temp):
    body = json.dumps({"model": model, "temperature": temp,
                       "messages": [{"role": "user", "content": prompt}]}).encode()
    req = urllib.request.Request("https://openrouter.ai/api/v1/chat/completions", data=body,
                                 headers={"Authorization": f"Bearer {KEY}", "Content-Type": "application/json"})
    try:
        r = json.load(urllib.request.urlopen(req, timeout=180))
        msg = r["choices"][0]["message"]
        return msg.get("content") or msg.get("reasoning") or ""
    except Exception:
        return ""


def extract_code(text):
    text = text or ""
    blocks = re.findall(r"```(?:python|py)?\n(.*?)```", text, re.S)
    if blocks:
        return max(blocks, key=len)
    return text if ("def " in text or "import " in text or "input(" in text) else ""


def runs_ok(code, cases):
    if not code.strip():
        return False
    d = tempfile.mkdtemp(prefix="passk-")
    sol = os.path.join(d, "s.py")
    open(sol, "w").write(code)
    try:
        for c in cases:
            try:
                p = subprocess.run(["/tmp/bench-venv/bin/python", sol], input=c["input"],
                                   capture_output=True, text=True, timeout=6)
            except subprocess.TimeoutExpired:
                return False
            if p.stdout.strip() != c["output"].strip():
                return False
        return True
    finally:
        import shutil; shutil.rmtree(d, ignore_errors=True)


def passk(model, task_id, n):
    t = SAMPLE[task_id]
    prompt = ("Write a complete Python 3 program that reads stdin and writes stdout, solving this "
              "competitive programming problem. Output the program in a ```python code block.\n\n" + t["question"])
    cases = t["cases"][:6]
    temps = [0.2 + 0.9 * (i / max(1, n - 1)) for i in range(n)]  # 0.2 .. 1.1 spread
    with cf.ThreadPoolExecutor(max_workers=16) as ex:
        outs = list(ex.map(lambda tp: sample_one(model, prompt, tp), temps))
    solved = sum(runs_ok(extract_code(o), cases) for o in outs)
    return solved


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", required=True)
    ap.add_argument("--n", type=int, default=64)
    ap.add_argument("--tasks", required=True)
    a = ap.parse_args()
    for tid in a.tasks.split(","):
        s = passk(a.model, tid, a.n)
        verdict = f"pass@{a.n}=YES ({s}/{a.n})" if s > 0 else f"pass@{a.n}=NO (0/{a.n})"
        print(f"{a.model:34s} {tid:12s} {verdict}", flush=True)


if __name__ == "__main__":
    main()
