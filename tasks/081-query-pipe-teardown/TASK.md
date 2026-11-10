# Order retained-query pipe fixture teardown

## Scope and provenance

Base: `8c5fe493f9b9e78e15ef831c532bc267d0c7fb53`, topic
`task/081-query-pipe-teardown`, isolated worktree `minicore-host-task081`.
Code: `643097c7a71daa94f09d0cf14988ea8579c15c50`.
This is a fresh reconstruction after the second execution-environment loss.
The old task081 source and logs are unavailable. The coordinator's earlier
92-run reproduction is historical reporting, not this task's new evidence.
The restored task054 implementation `512bc0fc1626e89c2e0d3533e9f5a1aebd6ac1bf`
was read as a code reference; task070 was independently reproduced below.

Only task070 query tests, task081 private test support, and this record change.
Production, SPEC v4.93, Runtime `69faf9196991524b5549499d6208a1eaf567bf56`,
all manifests, Cargo.lock, TUI, and remote refs stay unchanged. No push.
The requested Astra/max configuration is not independently attested.
The parent owns PROJECT.md, independent review, and aggregate integration.

## Fresh reproduction and source evidence

Evidence: `/workspace/shared/task081-query-validation/`; `env.sh` records the
exact Rust 1.90.0 environment, task-owned `CARGO_TARGET_DIR`, jobs=2, debug=0,
and disabled incremental compilation. Every Cargo dependency command uses
`--locked --offline`. No lockfile regeneration or task-specific fetch occurred.

1. Built the unchanged task070 module with
   `cargo test -p minicore-host-transport --test task070_query_recovery --no-run --message-format=json --locked --offline`.
   `original-build.jsonl`, `original-build.log`, and `original-executable.txt`
   preserve its actual output and executable. Original module SHA-256:
   `c6642230cdab0fa6f6d8770ee485a3c03459d956bc0de8101a67f15a3e555bbb`.
2. Repeated that executable with `--nocapture`, default harness concurrency,
   and the original two worker threads per test. The first 58 complete suites
   passed; run 59 failed with exit 101 at the original persistent unload/reload
   test's line 266, `server.await.unwrap().unwrap()`: real OS error 32,
   `BrokenPipe`. The other original test passed. See `original-repeat.log`
   and `original-red-summary.txt`. This reproduces the reported CI location
   on the new checkout, not a rerun of the unavailable CI environment.
3. Source inspection found the same teardown sequence in both tests.
   The unmodified `assert_retained` explicitly unsubscribes, then drops the
   same SDK subscription. `RpcSubscription::drop` still queues an idempotent
   unsubscribe request; `RpcClient::close` aborts both I/O tasks without
   ordering their actual endpoint drops. `serve_frames` returns real write
   errors or exits on request EOF.
4. The fresh controlled diagnostic calls the original `assert_retained`
   over real OS pipes. It gates the actual second unsubscribe response
   (`ended=false`), holds request EOF with a duplicate write descriptor,
   confirms the real response read descriptor has been dropped, then releases
   the real write. Keeping the original strict server unwrap produces exit
   101 and `BrokenPipe` deterministically (`controlled-strict-red.log`).
   Exact diagnostic sources are `task070-strict-red-main.rs` and
   `task081-strict-red-support.rs`; they are diagnostic evidence, not shipped
   acceptance assertions. Red support SHA-256:
   `b7e905171ba1d0636c2f1d646173c6c7a98caaf07396ff8b91cbc0d1233dbff1`.

## Test-only correction

A private request relay copies bytes unchanged into the same real OS pipe.
Teardown aborts and awaits only this relay, closing the request write end,
strictly awaits the actual server result while its response reader is alive,
and only then calls SDK close. The server join is neither aborted nor ignored.
A bounded five-second timeout reports a hang; there are no timing sleeps,
retries, reduced concurrency, or swallowed `BrokenPipe` errors.

Both original tests use this teardown. Reversing only the fixture imports
and these two calls yields the original source byte-for-byte, including all
canonical query/state/config/revision/load assertions and worker counts.

One new controlled regression covers both orders. Its deliberately negative
reader-first branch requires the exact OS `BrokenPipe`; the request-EOF-first
branch uses the same strict teardown as the two business tests and requires
successful server completion. The actual pending writer must be retired by
EOF, rather than released by a sleep. In both branches, original retained
queries remain identical and the existing owner completes another real Runtime
Turn after the observer closes. The scripted model is an explicit test fixture,
not a live provider or TUI check. No production correctness fix is inferred
from this test-fixture error.

## Completed checks at the fixed code

- `cargo test -p minicore-host-transport --test task070_query_recovery --locked --offline -- --nocapture`:
  passed all three cases (`focused-green.log`), including the actual
  `ended=false` response in both controlled close orders.
- The same fixed executable passed 200 consecutive complete suites with
  default harness concurrency (`fixed-repeat-200.log`). This is three unique
  cases exercised repeatedly, not 600 distinct tests.
- `cargo clippy -p minicore-host-transport --all-targets --locked --offline -- -D warnings`:
  passed (`transport-clippy.log`).
- `cargo fmt --all -- --check`, `./scripts/check-whitespace.sh`, original-source
  round-trip comparison, and protected-source checks passed (`source-checks.log`).
  Every manifest and the original lockfile match the base. Cargo.lock SHA-256:
  `1c372a170c3acdfe79e795e9bf3aa5c59b3dc935636c8ef507b02b72ce8eb125`.
  SPEC SHA-256 remains
  `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

- `cargo test -p minicore-host-transport --all-targets --locked --offline`:
  passed all 140 cases (`all-transport-tests.log`). No unrelated fixture failed
  in this run and no additional source changes were made.
- `cargo test -p minicore-host-transport --all-targets --locked --offline -- --list`:
  confirms 140 unique cases by executable plus test name, including the three
  task070 cases (`transport-test-list.log`, `test-counts.json`). This is the
  transport-crate count, not a whole-workspace acceptance count.
- `cargo test -p minicore-host-transport --doc --locked --offline`:
  one doctest passed (`transport-doctests.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc -p minicore-host-transport --no-deps --locked --offline`:
  passed (`transport-docs.log`).

Whole-workspace validation and its unique-test listing, independent review,
publication, live provider, and TUI acceptance are not claimed by this worker.
The final handoff is the code commit above plus this evidence-only record;
no task process or unfinished test run remains.
