#!/usr/bin/env python3
"""Aggregate BigCodeBench-Hard results: frontier baselines, damascus N-scaling,
ablations, with the frontier gap."""
import json, glob
from collections import defaultdict


def load():
    rows = []
    for p in glob.glob("bench/bcb/results/*.jsonl"):
        for l in open(p):
            l = l.strip()
            if l:
                rows.append(json.loads(l))
    return rows


def main():
    rows = load()
    if not rows:
        print("no results"); return
    by = defaultdict(lambda: {"s": 0, "n": 0, "t": 0.0, "tam": 0})
    for r in rows:
        a = by[r["config"]]
        a["s"] += r["solved"]; a["n"] += 1; a["t"] += r["seconds"]; a["tam"] += r.get("tampered", 0)
    N = max(a["n"] for a in by.values())

    def line(c):
        a = by[c]
        rate = 100 * a["s"] / a["n"] if a["n"] else 0
        tam = f"  tamper={a['tam']}" if a["tam"] else ""
        return f"{c:36s} {a['s']:2d}/{a['n']:<2d}  {rate:5.0f}%   {a['t']/a['n']:6.1f}s{tam}"

    print(f"# BigCodeBench-Hard (safe subset) — {N} tasks\n")
    print("## Frontier baselines")
    fr = []
    for c in ("opus48", "codex55"):
        if c in by:
            print(line(c)); fr.append(100 * by[c]["s"] / by[c]["n"])
    front = max(fr) if fr else 0

    for m in ("gemma", "ossbig"):
        sweep = sorted([c for c in by if c.startswith(f"damascus:{m}:n") and c.count(":") == 2],
                       key=lambda c: int(c.split(":n")[1]))
        if sweep:
            print(f"\n## damascus + {m} (N-scaling)")
            for c in sweep:
                print(line(c))

    abl = sorted(c for c in by if ":no_" in c)
    if abl:
        print("\n## Ablations (damascus:gemma:n8)")
        if "damascus:gemma:n8" in by:
            print(line("damascus:gemma:n8") + "   [full]")
        for c in abl:
            print(line(c))

    print(f"\nFrontier line (best of Opus 4.8 / Codex 5.5): {front:.0f}%")
    best_dam = max((100 * by[c]["s"] / by[c]["n"] for c in by if c.startswith("damascus")), default=0)
    print(f"Best damascus config so far: {best_dam:.0f}%   gap to frontier: {front-best_dam:.0f} pts")


if __name__ == "__main__":
    main()
