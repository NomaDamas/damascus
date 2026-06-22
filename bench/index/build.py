#!/usr/bin/env python3
"""Aggregate component results into the AA-aligned proxy index (INDEX.md).

Index = unweighted mean of per-component pass@1 (AA's aggregation). Efficiency =
pooled per-task mean wall-clock seconds. Components are explicit file globs so we
never mix an experiment (e.g. very-hard) into the canonical component.
"""
import glob, json
from collections import defaultdict

COMPONENTS = {
    "Implement (LCB-8)": ["bench/lcb/results/frontier.jsonl", "bench/lcb/results/damascus.jsonl",
                          "bench/lcb/results/implement_ensemble.jsonl"],
    "Terminal": ["bench/terminal/results/*.jsonl"],
}

VARIANT = {
    "opus48": "Claude Code · Opus 4.8",
    "codex55": "Codex · gpt-5.5",
    "damascus:ensemble:n16": "Damascus · ensemble (n16)",
    "damascus:ensemble:n8": "Damascus · ensemble (n8)",
    "damascus:ossbig:n8": "Damascus · gpt-oss-120b (n8)",
    "damascus:ossbig:n16": "Damascus · gpt-oss-120b (n16)",
    "damascus:kimi:n8": "Damascus · Kimi-K2.7 (n8)",
    "damascus:glm:n8": "Damascus · GLM-5.2 (n8)",
    "damascus:gemma:n8": "Damascus · Gemma-4-26b (n8)",
}


def load(globs):
    rows = []
    for g in globs:
        for p in glob.glob(g):
            for l in open(p):
                l = l.strip()
                if l:
                    rows.append(json.loads(l))
    return rows


def main():
    comp_scores = defaultdict(dict)   # variant -> component -> (solved, n)
    comp_time = defaultdict(lambda: [0.0, 0])
    components = []
    for comp, globs in COMPONENTS.items():
        rows = load(globs)
        if not rows:
            continue
        components.append(comp)
        agg = defaultdict(lambda: [0, 0])
        for r in rows:
            a = agg[r["config"]]
            a[0] += r["solved"]; a[1] += 1
            comp_time[r["config"]][0] += r["seconds"]; comp_time[r["config"]][1] += 1
        for cfg, (s, n) in agg.items():
            comp_scores[cfg][comp] = (s, n)

    # Build table
    print("# Damascus AA-aligned proxy index\n")
    print("pass@1 per component; **Index** = mean of available components. Proxy estimate, see docs/AA_INDEX.md.\n")
    header = "| Variant | " + " | ".join(components) + " | Index | avg s/task |"
    sep = "|" + "---|" * (len(components) + 3)
    print(header); print(sep)
    # order: frontier first, then damascus by index
    def variant_index(cfg):
        rates = [s / n for (s, n) in comp_scores[cfg].values() if n]
        return sum(rates) / len(rates) if rates else 0
    order = sorted(comp_scores, key=lambda c: (-variant_index(c)))
    for cfg in order:
        name = VARIANT.get(cfg, cfg)
        cells = []
        for comp in components:
            if comp in comp_scores[cfg]:
                s, n = comp_scores[cfg][comp]
                cells.append(f"{100*s/n:.0f}% ({s}/{n})")
            else:
                cells.append("—")
        idx = variant_index(cfg)
        t = comp_time[cfg]
        avg = t[0] / t[1] if t[1] else 0
        print(f"| {name} | " + " | ".join(cells) + f" | **{100*idx:.0f}%** | {avg:.0f} |")


if __name__ == "__main__":
    main()
