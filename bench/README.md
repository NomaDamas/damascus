# Damascus benchmark

A small end-to-end benchmark: run each task under `tasks/` through the full Fold Loop with one
or more models, then check whether `cargo test` passes afterwards (the ground truth) and how long
the run took.

## Run

```bash
# build the binary first
cargo build --release

# default models (OpenRouter); reads OPENROUTER_API_KEY (or sources /home/cheol/.hermes/.env)
bench/run.sh

# or pick your own models / endpoint
bench/run.sh google/gemma-4-26b-a4b-it openai/gpt-oss-120b
OPENROUTER_API_KEY=... bench/run.sh qwen/qwen3-coder
```

Tunables (env): `BENCH_CANDIDATES`, `BENCH_REPAIR_ROUNDS`, `BENCH_CONCURRENCY`,
`BENCH_MAX_RECURSION`, `BENCH_TIMEOUT`, `BENCH_GATE_TIMEOUT`.

## Tasks

| task | kind |
|------|------|
| `is_prime` | implement a single function |
| `fizzbuzz` | implement a function returning `Vec<String>` |
| `roman` | integer → Roman numeral |
| `stack` | implement a method in a **second file** (`src/stack.rs`) — exercises AST slicing |
| `merge_intervals` | sort + merge overlapping intervals |
| `bugfix_median` | **fix a bug** in an existing function (even-length median) |

## Latest results

Config: `candidates=6`, `repair_rounds=1`, `concurrency=6`, gates `cargo build` + `cargo test`.
Models served via OpenRouter. Time is wall-clock per task (plan → best-of-N → filter → verify).

| model | solved | solve-rate | avg seconds |
|-------|:------:|:----------:|:-----------:|
| `google/gemma-4-26b-a4b-it` | 6/6 | **100%** | **21.0** |
| `openai/gpt-oss-120b` | 6/6 | **100%** | 38.5 |

Per-task (seconds):

| task | gemma-4-26b-a4b | gpt-oss-120b |
|------|:---------------:|:------------:|
| is_prime | 25.1 | 16.8 |
| fizzbuzz | 20.9 | 45.8 |
| roman | 15.9 | 37.4 |
| stack | 19.3 | 21.2 |
| merge_intervals | 13.8 | 55.3 |
| bugfix_median | 31.0 | 54.7 |

### Takeaways

- **Both modest models solve every task at 100%** when wrapped in the Fold Loop — the objective
  gate guarantees that whatever is accepted actually builds and passes the tests.
- The smaller MoE (**gemma-4-26b-a4b**, ~4B active params) is **~1.8× faster** than gpt-oss-120b at
  equal solve-rate — exactly the "fast, cheap worker" profile Damascus is designed to exploit.
- Earlier runs with a smaller budget (`candidates=6`) surfaced two harder failures; raising
  `candidates` to 16 lifted **gpt-oss-120b from 4/6 to 6/6** — a direct demonstration of
  test-time scaling. (One gemma failure turned out to be an over-strict signature contract in the
  harness, since fixed — the benchmark earned its keep.)

Raw data: `results/bench-*.tsv`.
