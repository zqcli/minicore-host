# Host 0.13.0: first simplification batch

- Comparison baseline: `44355a312d52e2acd1ae5feffb663a6f2c2b1800`, Host `dev`.
- Design: SPEC v4.11.13 unchanged. No changes to public operations, replay
  representation, the `agent.send` schema or persistence formats.
- `host-provider`: Chat and Responses share a bounded SSE frame reader. The
  protocol adapters still own terminal events, replay and cancellation priority;
  Responses WebSocket framing is unchanged.
- `host-core`: Session metadata updates share activity-save waiting and retry
  on that specific contention. External controls still return their accepted
  receipt; tool-owned updates still await the committed result.
- `host-runtime` and `host-storage`: use the expected contiguous tool index
  to detect duplicates, and remove the unreachable second memory check after
  the memory journal's early return.
- Tests: exercise split network frames, multiple frames per chunk, partial
  end-of-stream, and buffered terminal frame interruption. Keep the existing
  metadata receipt, failure and persistence tests.

## Validation

Source verified: `12486d63048e5fa3266fd0ff5337fdb0df5b569e`, Linux x86_64,
pinned Rust 1.90.0.

- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline --keep-going -- -D warnings`: passed.
- `cargo test --workspace --all-targets --locked --offline --no-fail-fast`:
  all passed; `-- --list` enumerated 1,308 unique cases. Nested subprocess
  output is not counted as an extra case.
- `cargo test --workspace --doc --locked --offline`: 7 examples passed.
- `cargo metadata --locked --offline --format-version 1 --no-deps`: all 14
  workspace packages report 0.13.0.

No live model, macOS or Windows run in this local validation. TUI 0.13.0 was
independently validated against its unchanged Host 0.12.18 pin, not this Host
change. This is a local checkpoint; publication and independent review are
separate.
