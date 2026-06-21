#!/usr/bin/env python3
"""Aggregate polyglot benchmark JSONL into tables: frontier baselines, the
damascus N-scaling curve, and feature ablations."""
import json, sys, glob
from collections import defaultdict


def load(paths):
    rows = []
    for p in paths:
        for line in open(p):
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def agg(rows):
    by = defaultdict(lambda: {"solved": 0, "n": 0, "secs": 0.0, "tamper": 0})
    for r in rows:
        a = by[r["config"]]
        a["solved"] += r["solved"]; a["n"] += 1
        a["secs"] += r["seconds"]; a["tamper"] += r.get("tampered", 0)
    return by


def line(cfg, a):
    rate = 100 * a["solved"] / a["n"] if a["n"] else 0
    avg = a["secs"] / a["n"] if a["n"] else 0
    t = f"  (tamper={a['tamper']})" if a["tamper"] else ""
    return f"{cfg:34s} {a['solved']:2d}/{a['n']:<2d}  {rate:5.0f}%   {avg:6.1f}s{t}"


def main():
    rows = load(glob.glob("bench/polyglot/results/*.jsonl"))
    if not rows:
        print("no results yet"); return
    by = agg(rows)
    n_problems = max(a["n"] for a in by.values())

    print(f"# Polyglot benchmark — {n_problems} Python exercises (Exercism), pytest-gated\n")
    print("config                              solved  rate   avg-time")
    print("-" * 64)

    print("\n## Frontier baselines")
    for c in ("opus48", "codex55", "gjc55"):
        if c in by: print(line(c, by[c]))
    front = max((100 * by[c]["solved"] / by[c]["n"]) for c in ("opus48", "codex55", "gjc55") if c in by and by[c]["n"])

    for model in ("gemma", "ossbig"):
        sweep = sorted([c for c in by if c.startswith(f"damascus:{model}:n") and c.count(":") == 2],
                       key=lambda c: int(c.split(":n")[1]))
        if sweep:
            print(f"\n## damascus + {model}  (N-scaling)")
            for c in sweep: print(line(c, by[c]))

    abl = [c for c in by if ":no_" in c]
    if abl:
        print("\n## Ablations (damascus:gemma:n8, feature removed)")
        base = "damascus:gemma:n8"
        if base in by: print(line(base + " [full]", by[base]))
        for c in sorted(abl): print(line(c, by[c]))

    print(f"\nFrontier line (best of Opus/Codex): {front:.0f}%")


if __name__ == "__main__":
    main()
