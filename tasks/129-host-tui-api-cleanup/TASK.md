# Host/TUI 0.13.2 API cleanup

- Host code checkpoint `62038d5c34507d0bea42d26ccdb318fbace5731a` on local dev, based on published `fdc07b97618504b81a4fb536caf36dcbd4bf456a`.
- TUI baseline `c71cb3e186eff5b05612ba85ca940947583541c0`: reuse the Session subscription baseline; refresh model choices separately from routine catalog updates.
- [Agent tool](../agent-send-followup/TASK.md): omitted send mode is still steer; explicit follow_up reaches the next Turn, not an active wait.
- [Runtime](../assistant-message-unification/TASK.md): one Rust Assistant representation, existing serialized shapes and provider replay behavior retained.
- [Session update](../128-session-update-entry/TASK.md): the single Host API is `session_update`. The WSS route and its Saving/Committed receipt are unchanged; the tool's internal path still waits for the save.
- Storage owns the disk journal only. Opening a journal on a memory handle returns `InvalidStorageMode` without I/O; Core still owns real memory histories. No storage format migration.
- The TUI validation build uses the exact Host checkpoint via local Git. A published TUI must instead lock to the equivalent reachable GitHub commit. No main merge or tag is part of this task.

## Verification

Local runs use Linux and synthetic providers. Cross-platform CI and live model
behavior are separate from this local gate.

- Host: `HOME=<isolated test home> ./scripts/check.sh` (Rust 1.90.0);
  formatting, whitespace, strict Clippy and Rustdoc, 28 Python checks, 1,308
  Rust tests and 7 doctests passed. An initial run stopped because the
  cloud account's default log directory was read-only; no product code
  changes were used to bypass it.
- Web baseline comparison: using the existing WSS suite and adapting only
  the test fixture's Host config to the current providers/models format,
  the published Host 0.13.0 and locally built 0.13.2 both passed 12 of 14,
  including `session.update`. Both had the same two pre-existing timeouts:
  background compaction notification and idle failure pause. Web source
  and CI are not changed by this task.
- TUI 0.13.2 local paired checkpoint
  `1756c42c3bd03aab98b74db20f795ed30d89d365`: strict lockfile resolution
  confirmed all 13 embedded Host crates came from the Host code checkpoint.
  Formatting and `cargo +stable test --locked --offline --all-targets --no-fail-fast`
  passed (585 tests, five ignored; Rust 1.100.1, Linux, isolated test HOME).
  The Agent card fixture verifies that one delegation spans two Turn ranges
  after a follow-up; four UI snapshots changed only the version label.
