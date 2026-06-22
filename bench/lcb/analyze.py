#!/usr/bin/env python3
"""Aggregate LiveCodeBench-hard results: frontier vs damascus across models."""
import json, glob
from collections import defaultdict

rows = []
for p in glob.glob("bench/lcb/results/*.jsonl"):
    for l in open(p):
        l = l.strip()
        if l:
            rows.append(json.loads(l))
if not rows:
    print("no results"); raise SystemExit

by = defaultdict(lambda: {"s": 0, "n": 0, "t": 0.0, "tam": 0})
for r in rows:
    a = by[r["config"]]
    a["s"] += r["solved"]; a["n"] += 1; a["t"] += r["seconds"]; a["tam"] += r.get("tampered", 0)
N = max(a["n"] for a in by.values())


def line(c):
    a = by[c]
    rate = 100 * a["s"] / a["n"] if a["n"] else 0
    tam = f" tamper={a['tam']}" if a["tam"] else ""
    return f"{c:32s} {a['s']:2d}/{a['n']:<2d} {rate:5.0f}%  {a['t']/a['n']:6.1f}s{tam}"


print(f"# LiveCodeBench-Hard (recent AtCoder, stdin) — {N} problems\n")
print("## Frontier agents")
for c in ("opus48", "codex55"):
    if c in by:
        print(line(c))
print("\n## damascus + open models (n=8)")
for c in sorted(c for c in by if c.startswith("damascus")):
    print(line(c))
fr = [100 * by[c]["s"] / by[c]["n"] for c in ("opus48", "codex55") if c in by and by[c]["n"]]
if fr:
    print(f"\nFrontier spread: {min(fr):.0f}%–{max(fr):.0f}%")
