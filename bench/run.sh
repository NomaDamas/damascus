#!/usr/bin/env bash
# Damascus benchmark: run every task under bench/tasks/ with one or more models,
# measure solve-rate (does `cargo test` pass afterwards?) and wall-clock time.
#
# Usage:
#   bench/run.sh                       # default models
#   bench/run.sh google/gemma-4-26b-a4b-it openai/gpt-oss-120b
#
# Requires an OPENROUTER_API_KEY. If /home/cheol/.hermes/.env exists it is sourced.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
TASKS_DIR="$HERE/tasks"
RESULTS_DIR="$HERE/results"
ENV_FILE="${DAMASCUS_ENV_FILE:-/home/cheol/.hermes/.env}"

# --- config (overridable via env) ---
CANDIDATES="${BENCH_CANDIDATES:-6}"
REPAIR_ROUNDS="${BENCH_REPAIR_ROUNDS:-1}"
CONCURRENCY="${BENCH_CONCURRENCY:-6}"
MAX_RECURSION="${BENCH_MAX_RECURSION:-1}"
TIMEOUT="${BENCH_TIMEOUT:-900}"          # per-task wall-clock cap (seconds)
GATE_TIMEOUT="${BENCH_GATE_TIMEOUT:-120}" # per build/test command timeout

DAMASCUS_BIN="${DAMASCUS_BIN:-$REPO/target/release/damascus}"

DEFAULT_MODELS=("google/gemma-4-26b-a4b-it" "openai/gpt-oss-120b")
if [ "$#" -gt 0 ]; then MODELS=("$@"); else MODELS=("${DEFAULT_MODELS[@]}"); fi

[ -f "$ENV_FILE" ] && { set -a; . "$ENV_FILE"; set +a; }
if [ -z "${OPENROUTER_API_KEY:-}" ]; then
  echo "error: OPENROUTER_API_KEY not set (looked in $ENV_FILE)" >&2
  exit 1
fi
if [ ! -x "$DAMASCUS_BIN" ]; then
  echo "error: damascus binary not found at $DAMASCUS_BIN (run: cargo build --release)" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"
STAMP="$(date +%Y%m%d-%H%M%S)"
TSV="$RESULTS_DIR/bench-$STAMP.tsv"
printf "model\ttask\tsolved\tseconds\n" > "$TSV"

echo "Damascus benchmark  ($STAMP)"
echo "models: ${MODELS[*]}"
echo "config: candidates=$CANDIDATES repair_rounds=$REPAIR_ROUNDS concurrency=$CONCURRENCY"
echo "results: $TSV"
echo

for model in "${MODELS[@]}"; do
  echo "==================== model: $model ===================="
  for task_dir in "$TASKS_DIR"/*/; do
    task="$(basename "$task_dir")"
    work="$(mktemp -d "/tmp/dmsc-bench-${task}-XXXX")"
    cp -r "$task_dir"/. "$work"/
    prompt="$(cat "$work/prompt.txt")"
    rm -f "$work/prompt.txt"

    cat > "$work/damascus.toml" <<EOF
[providers.openrouter]
base_url = "https://openrouter.ai/api/v1"
api_key_env = "OPENROUTER_API_KEY"

[models]
planner  = "openrouter/$model"
drafter  = "openrouter/$model"
judge    = "openrouter/$model"
repairer = "openrouter/$model"

[scaling]
candidates = $CANDIDATES
repair_rounds = $REPAIR_ROUNDS
max_recursion = $MAX_RECURSION
max_steps = 8
concurrency = $CONCURRENCY

[verify]
build = "cargo build"
test = "cargo test"
timeout_secs = $GATE_TIMEOUT
EOF

    printf "  %-12s " "$task"
    start=$(date +%s.%N)
    ( cd "$work" && timeout "$TIMEOUT" "$DAMASCUS_BIN" run "$prompt" --yes --quiet ) \
        > "$work/damascus.log" 2>&1
    end=$(date +%s.%N)
    elapsed=$(awk "BEGIN{printf \"%.1f\", $end-$start}")

    # ground-truth check: do the tests pass now?
    if ( cd "$work" && cargo test --quiet >/dev/null 2>&1 ); then
      solved=1; mark="PASS"
    else
      solved=0; mark="fail"
    fi
    printf "%s  %6ss\n" "$mark" "$elapsed"
    printf "%s\t%s\t%s\t%s\n" "$model" "$task" "$solved" "$elapsed" >> "$TSV"
    rm -rf "$work"
  done
  echo
done

echo "==================== summary ===================="
awk -F'\t' 'NR>1 {
  total[$1]++; if ($3==1) solved[$1]++; time[$1]+=$4
}
END {
  printf "%-32s %8s %12s %14s\n", "model", "solved", "solve-rate", "avg-seconds"
  for (m in total) {
    printf "%-32s %5d/%-2d %11.0f%% %14.1f\n", m, solved[m], total[m], 100*solved[m]/total[m], time[m]/total[m]
  }
}' "$TSV"
echo
echo "full results: $TSV"
