#!/usr/bin/env python3
"""Run Damascus on real SWE-bench Verified instances and emit predictions for the
official swebench evaluator.

Per instance: clone the repo @ base_commit, install it in a throwaway venv,
localize the buggy file(s) from the issue, write a self-reproduction test (the
agent's own gate — SWE-bench hides the real tests), run the Damascus Fold Loop
with that gate, then emit the git diff as the model patch (excluding the repro).

This is a *general* agentic SWE flow (issue -> localize -> reproduce -> fix),
not tuned to any instance.
"""
import json, os, re, subprocess, sys, tempfile, shutil, urllib.request

OR_KEY = os.environ["OPENROUTER_API_KEY"]
DAMASCUS = os.environ.get("DAMASCUS_BIN", "/home/cheol/projects/onpremis_coding_agent/target/release/damascus")
MODELS = os.environ.get("SWE_MODELS", "openrouter/openai/gpt-oss-120b,openrouter/google/gemma-4-31b-it")
N = int(os.environ.get("SWE_N", "8"))


def chat(model, prompt, max_tokens=4000, temperature=0.2):
    model = model.removeprefix("openrouter/")  # raw OpenRouter API wants provider/model
    body = json.dumps({"model": model, "temperature": temperature, "max_tokens": max_tokens,
                       "messages": [{"role": "user", "content": prompt}]}).encode()
    req = urllib.request.Request("https://openrouter.ai/api/v1/chat/completions", data=body,
                                 headers={"Authorization": f"Bearer {OR_KEY}", "Content-Type": "application/json"})
    r = json.load(urllib.request.urlopen(req, timeout=240))
    m = r["choices"][0]["message"]
    return m.get("content") or m.get("reasoning") or ""


def run(cmd, cwd=None, timeout=600, env=None):
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, env=env,
                          capture_output=True, text=True)


def clone(repo, base_commit, dest):
    url = f"https://github.com/{repo}.git"
    run(["git", "clone", "--quiet", url, dest], timeout=600)
    run(["git", "checkout", "-q", base_commit], cwd=dest, timeout=120)
    run(["git", "config", "user.email", "a@b.c"], cwd=dest)
    run(["git", "config", "user.name", "d"], cwd=dest)


def make_venv(repo_dir):
    venv = os.path.join(repo_dir, ".swevenv")
    run(["uv", "venv", venv, "--python", "3.11"], timeout=120)
    py = os.path.join(venv, "bin", "python")
    # install the repo (editable) + pytest; tolerate failures (best effort)
    run(["uv", "pip", "install", "--python", py, "-q", "-e", ".", "pytest"], cwd=repo_dir, timeout=600)
    return py


def file_list(repo_dir, limit=400):
    out = []
    for root, dirs, files in os.walk(repo_dir):
        dirs[:] = [d for d in dirs if d not in (".git", ".swevenv", "tests", "test", "node_modules", "docs")]
        for f in files:
            if f.endswith(".py"):
                rel = os.path.relpath(os.path.join(root, f), repo_dir)
                out.append(rel)
            if len(out) >= limit:
                return out
    return out


def localize(model, issue, files):
    listing = "\n".join(files[:300])
    p = (f"Issue:\n{issue[:4000]}\n\nSource files:\n{listing}\n\n"
         "Which 1-3 source files most likely need editing to fix this issue? "
         "Reply with ONLY the file paths, one per line.")
    resp = chat(model, p, max_tokens=300)
    picks = [l.strip().strip("`-* ") for l in resp.splitlines() if l.strip()]
    return [f for f in picks if f in files][:3]


def write_repro(model, issue, target_files, repo_dir):
    p = (f"Issue:\n{issue[:4000]}\n\nLikely files: {target_files}\n\n"
         "Write a minimal pytest reproduction in a single file that FAILS on the current "
         "(buggy) code and will PASS once the issue is fixed. Import from the installed package. "
         "Output ONLY the python code in a ```python block.")
    resp = chat(model, p, max_tokens=1500)
    m = re.findall(r"```(?:python)?\n(.*?)```", resp, re.S)
    code = m[0] if m else resp
    open(os.path.join(repo_dir, "repro_test.py"), "w").write(code)


def damascus_fix(repo_dir, py, issue, target_files):
    toml = f"""
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"
[models]
planner  = "{MODELS.split(',')[0]}"
drafter  = "{MODELS.split(',')[0]}"
judge    = "{MODELS.split(',')[0]}"
repairer = "{MODELS.split(',')[0]}"
drafters = [{", ".join(json.dumps(m) for m in MODELS.split(','))}]
[scaling]
candidates = {N}
repair_rounds = 2
max_recursion = 1
max_steps = 6
concurrency = 10
[verify]
test = "{py} -m pytest -q repro_test.py"
timeout_secs = 120
"""
    open(os.path.join(repo_dir, "damascus.toml"), "w").write(toml)
    task = (f"Fix this issue by editing only the source file(s): {', '.join(target_files)}. "
            f"Make the reproduction in repro_test.py pass. Do not edit repro_test.py.\n\nIssue:\n{issue[:4000]}")
    run([DAMASCUS, "run", task, "--yes", "--quiet"], cwd=repo_dir, timeout=900,
        env=dict(os.environ))


def git_patch(repo_dir):
    # exclude scratch artifacts from the patch
    run(["git", "rm", "-q", "--cached", "-r", "--ignore-unmatch", ".swevenv", "damascus.toml",
         "repro_test.py", ".damascus"], cwd=repo_dir)
    for p in ("repro_test.py", "damascus.toml"):
        fp = os.path.join(repo_dir, p)
        if os.path.exists(fp):
            os.remove(fp)
    shutil.rmtree(os.path.join(repo_dir, ".swevenv"), ignore_errors=True)
    shutil.rmtree(os.path.join(repo_dir, ".damascus"), ignore_errors=True)
    r = run(["git", "diff"], cwd=repo_dir, timeout=120)
    return r.stdout

def direct_fix(model, issue, target_files, repo_dir):
    """General fallback when the verify-gated loop yields no patch: ask the model
    for the full corrected contents of the localized file(s) and apply them."""
    for tf in target_files:
        fp = os.path.join(repo_dir, tf)
        if not os.path.exists(fp):
            continue
        cur = open(fp).read()
        if len(cur) > 16000:
            continue
        p = (f"Issue:\n{issue[:4000]}\n\nFile {tf} (current contents):\n```\n{cur}\n```\n\n"
             "Output the COMPLETE corrected contents of this file that fixes the issue, "
             "in a single ```python block. Keep all unrelated code unchanged.")
        resp = chat(model, p, max_tokens=8000)
        m = re.findall(r"```(?:python)?\n(.*?)```", resp, re.S)
        if m and len(m[0].strip()) > 50:
            open(fp, "w").write(m[0])


def main():
    insts = json.load(open(os.environ.get("SWE_GOLD", "/tmp/swe_gold.json")))
    model0 = MODELS.split(",")[0]
    preds = []
    for inst in insts:
        iid = inst["instance_id"]
        print(f"=== {iid} ({inst['repo']}) ===", flush=True)
        work = tempfile.mkdtemp(prefix=f"swe-{iid}-")
        repo_dir = os.path.join(work, "repo")
        try:
            clone(inst["repo"], inst["base_commit"], repo_dir)
            py = make_venv(repo_dir)
            files = file_list(repo_dir)
            tgt = localize(model0, inst["problem_statement"], files) or files[:1]
            print("  localized:", tgt, flush=True)
            write_repro(model0, inst["problem_statement"], tgt, repo_dir)
            damascus_fix(repo_dir, py, inst["problem_statement"], tgt)
            patch = git_patch(repo_dir)
            if not patch.strip():
                print("  no verified patch; direct-fix fallback", flush=True)
                direct_fix(model0, inst["problem_statement"], tgt, repo_dir)
                patch = run(["git", "diff"], cwd=repo_dir, timeout=120).stdout
            print(f"  patch: {len(patch)} bytes", flush=True)
            preds.append({"instance_id": iid, "model_name_or_path": "damascus",
                          "model_patch": patch})
        except Exception as e:
            print("  ERROR", repr(e)[:200], flush=True)
            preds.append({"instance_id": iid, "model_name_or_path": "damascus", "model_patch": ""})
        finally:
            shutil.rmtree(work, ignore_errors=True)
    out = os.environ.get("SWE_OUT", "/tmp/swe_preds.jsonl")
    with open(out, "w") as f:
        for p in preds:
            f.write(json.dumps(p) + "\n")
    print("wrote", out, "patches:", sum(1 for p in preds if p["model_patch"]))


if __name__ == "__main__":
    main()
