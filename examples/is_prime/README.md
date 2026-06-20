# Example: implement `is_prime` with a local model

A minimal Rust crate whose tests fail because `is_prime` is unimplemented.
Damascus drives a small local model to a verified implementation.

## Run it

```bash
cd examples/is_prime
cargo test          # baseline: 2 tests FAIL (todo! panics)

damascus doctor     # confirm your models resolve
damascus run "Implement the is_prime function in src/lib.rs correctly so all tests pass. Do not modify the tests." --yes

cargo test          # now: 2 tests pass
```

## What happens

1. The **planner** decomposes the task (here, into a single step).
2. The **drafter** samples `candidates` edit-sets at rising temperatures.
3. Each candidate is applied in an isolated sandbox and run through `cargo build` + `cargo test`.
4. Candidates that fail the gate are discarded; the best passing one is selected and applied.
5. If none pass, the **reflexion repair** loop feeds the failure log back and retries.

The run is recorded under `.damascus/runs/<id>/`.

> Tip: bump `candidates`/`repair_rounds` in `damascus.toml` (or via
> `--candidates`/`--repair-rounds`) to spend more inference on harder tasks.
