# Task 053: Reconstruct business diagnostics

## Recovery boundary

The old environment and unpushed worktrees were lost. This is new implementation
and validation based on fixed recovered source
`c872e1d4d039684fa124d8b5a28fab8efc4ab11c`, not a claim that the complete prior
diagnostics implementation was restored byte for byte. Verbatim visible source,
patches, historical evidence and explicit gaps were saved separately under
`/workspace/shared/recovery-053` before this stage began.

The old production `6714259` passed 835 unique cases and 4 doctests; that is
historical evidence only. Its previous independent approval and integrated gate
do not validate this new tree. Missing functions and regression coverage must be
implemented and tested anew. SPEC v4.93, Runtime 0.10.0 at
`69faf9196991524b5549499d6208a1eaf567bf56`, TUI and dependency versions stay fixed.
The requested engineering configuration is gpt-6-astra/max, not an attestation of
the effective runtime. No worker push; the coordinator owns integration/backup.

## Initial file ownership

This slice owns provider production code and new provider diagnostics tests.
Transport is initially limited to an independent diagnostics module and pure
regressions. Its client/operations files and core/shared types remain coordinated
with task071. RPC/lib wiring requires a separate narrow handoff. No edits to
configuration, PreparedTurnBinding, local tools, Turn/query/history/core_adapter
or shared exports belong to this initial slice.

The six existing diagnostics/tracing dependency edges are already restored by
`f238626`; no manifest/lock edit is needed. Initial Cargo.lock SHA256:
`1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.

## Provider reconstruction

`host-provider/src/diagnostics.rs` is copied from the complete source actually
printed before the loss, retained as `provider-diagnostics.complete.rs`. The
`lib.rs` request/response wiring is newly implemented against the recovered base.
The diagnostic is constructed on the actual request future's first poll, so an
unpolled start does not fabricate an attempted transport. A transparent stream
observer records only actual status, error kind/delivery, usage and monotonic
elapsed duration; drop is not success or cancellation. The provider keeps its
actual Runtime loop/request identity and explicitly clears unrelated Host spans.
No Model wrapper, retry, deadline change, body capture or stream transformation
is added. Runtime remains the only retry owner.

New tests use actual Runtime and the existing loopback HTTP/SSE fixture. They are
not live-provider validation and do not complete owner/HostInfo/RPC coverage.
Pinned Runtime has no source retry-wait elapsed measurement; it must remain
absent rather than inferred from observer latency or Retry-After.

### Provider validation checkpoint

The new compound regression covers actual Runtime retry after HTTP 429, original
loop/request IDs, known and absent usage, real HTTP 401, pending SSE drop, real
request cancellation, an unpolled start, unrelated-span isolation, and body/key
redaction. It also completes real work before installation and after the writer
has closed. The latter is a closed-output degradation check, not a disk-full or
filesystem-write-failure claim. No observations are registered on the Runtime.

Fresh commands use Rust 1.90, the locked offline dependencies, two Cargo jobs,
debug information disabled, incremental disabled, and this worktree's separate
target directory:

- `cargo test -p minicore-host-provider --all-targets --locked --offline`: 44
  unique tests passed (5 unit, 1 new compound regression, 38 HTTP/Runtime).
- `cargo clippy -p minicore-host-provider --all-targets --locked --offline -- -D warnings`:
  passed after removing a needless lifetime from the new test helper.
- `RUSTDOCFLAGS=-Dwarnings cargo doc -p minicore-host-provider --no-deps --locked --offline`:
  passed.
- `cargo test -p minicore-host-provider --doc --locked --offline`: 1 doctest
  passed. The all-targets `-- --list` run confirmed the 44 unique cases above.
- `cargo fmt --all --check` and `git diff --check`: passed.
- `cmp` with the rescued complete provider helper: identical. The new request
  wiring and tests are not claimed to match lost source.

The initial new regression had two fixture-only compile errors (the Runtime
attempt-count array comparison and construction of a typed test identity); the
first lint run then found the needless lifetime. Those outputs remain separate
from the successful reruns under `/workspace/shared/task053-recovered-provider-*`.
They do not imply that any historical lost gate was rerun successfully.

The entire workspace gate, independent review, owner diagnostics, HostInfo
health binding, transport integration, process/stdio and actual filesystem
failure regressions have not been completed in this reconstruction phase.
