# Contributing to Damascus

Thanks for your interest! Damascus is a small, focused Rust codebase.

## Development

```bash
cargo fmt --all          # format
cargo clippy --all-targets -- -D warnings   # lint (CI enforces -D warnings)
cargo test --all         # unit + offline end-to-end tests
```

The full Fold Loop is covered offline by `tests/fold_loop.rs` via a mock `ChatProvider`, so you
do not need a model or network to test most changes.

## Guidelines

- Keep the deterministic core (edits, sandbox, verify, select) deterministic and well-tested.
- New model backends should implement `ChatProvider` rather than special-casing the orchestrator.
- Prefer small, verifiable changes — it's the whole philosophy of the tool.
- Add or update tests for any behavior change.

## Trying it end to end

Pull a small local model and run the example:

```bash
ollama pull qwen2.5-coder:7b
cd examples/is_prime && damascus run "implement is_prime so the tests pass" --yes
```
