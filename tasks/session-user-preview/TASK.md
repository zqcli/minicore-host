# First-user Session previews

## Scope

- Baseline: `929c6632c2c614d02f3ddae455f672a5bf096bc5`.
- Requested worker configuration: `gpt-6-astra`, `max` reasoning.
- Add optional `first_user_preview` to Session info/catalog and `session.json`.
  It is separate from the explicit title, bounded to 160 Unicode characters,
  with whitespace/control separators collapsed and an ellipsis for truncation.
- Capture only accepted `MessageSource::User` text, before model input wrappers.
  Agent messages, delegation notifications and native-only history are excluded.
- Use the existing activity metadata writer and reconciliation, including input
  arriving while another metadata save runs. No config revision is invented.
- Old records without the optional field recover it on their normal history load.
  Until then their catalog may still show an ID. No migration, startup history
  scan, per-picker history reads or separate title cache is introduced.

## Implementation

`SessionController` accepts and projects the preview; the existing Session
record owns its persistence. An unloaded old record waits for its original
history before a newly queued input may supply the fallback. Renaming and
clearing the explicit title never modify the preview.

Main files: `session/input.rs`, `session/loading.rs`, `session/metadata.rs`,
`session/persistent.rs`, `session/discovery.rs`, `types.rs`, `discovery.rs` and
`host-storage/src/records.rs`. Fixtures initialize the optional field as absent.

## Verification

Implementation checkpoint: `0a1c638b37ba6ec03d9300aea142810bdf918628`.
Compared with baseline `929c6632c2c614d02f3ddae455f672a5bf096bc5`.

Executed with Rust 1.90.0 and the locked dependency graph:

- `cargo fmt --all -- --check`: passed.
- `cargo test -p minicore-host-core --lib --locked`: 144 passed.
- `cargo test -p minicore-host-storage --locked`: 95 tests and 1 doctest passed
  (36 library tests, 59 integration tests; child process probes are not additional cases).
- `cargo clippy -p minicore-host-core --lib --locked -- -D warnings`: passed.

Coverage includes user/Agent provenance, Unicode and whitespace bounds,
acceptance during a metadata save, rename persistence, catalog-only reopen and
lazy recovery from pre-field history before a newly queued message.

TUI implementation checkpoint:
`5172ec9f962d51e85c743dad7d2254281b1a36c2`. Its real in-process Host picker test
covers first-input refresh while the model is still running, fresh frontend
catalog labels, repeated opening, title/preview/ID filtering and rename/clear.
The TUI validation used local Cargo patches to this exact Host worktree;
those patches and lockfile edits are not committed.

## Integration and limits

TUI consumes the field for `/sessions` labels and filtering, and distinguishes
manual titles from previews and empty-session IDs. The Host implementation
was published as [`13d084ffc3dccceb1770edd555f7e19ddc4edfbc`](https://github.com/zqcli/minicore-host/commit/13d084ffc3dccceb1770edd555f7e19ddc4edfbc).
See [PROJECT.md](../../PROJECT.md#current-checkpoint-host-01218) for the 0.12.18
Host/TUI pins and Host CI. These checks do not represent live-model acceptance
or independent review.
