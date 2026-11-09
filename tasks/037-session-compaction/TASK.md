# Own Session compaction and durable commits

Status: implemented and full workspace gate passed. Independent review and
transport integration remain separate. No push.

## Scope and fixed inputs

- Base cdb8ee144ef92ff29c1025148e05b5a9bf50f2e3, isolated branch
  feat/session-compaction-owner. Only the reviewed Session integration diff is
  intended for integration; unrelated local-tools work is quarantined.
- Terminal-claim prerequisite f4101527ec2949f4eeee3f4cac81cac74631c61f,
  fixture hashing 0fb766e and d2edb504, dependency bootstrap ad5ec42.
- SPEC v4.93 unchanged; Runtime 0.10.0 at
  69faf9196991524b5549499d6208a1eaf567bf56 unchanged.
- host-compaction engine f5c43f5 is a persistence-free calculation dependency.
- minicore-agent f36a873 is code reference only. Its owner cancellation claim,
  utility projection and test patterns were inspected. No summary file,
  quotas, recovery tickets or whole-Turn settlement behavior is adopted.
- Storage coverage marker and journal validation are owned separately in task036.
  Transport, local-tools, registry, workspace backend and TUI are not edited.

## Implemented owner path

- Manual admission and Turn preparation share the Session lock. The utility has
  an owned task and its exact operation receipt; queues and explicit pause are
  unchanged. Manual resources bind only model, reasoning, prompt and ToolSpecs.
  The published resource snapshot is captured before admission recheck.
- Automatic compaction wraps the real Runtime request prompt, using the frozen
  public Turn settings and model. The raw native transcript remains append-only.
  Original per-Loop raw base and accepted cursor preserve the actual unhanded
  suffix and allow prior tool batches in the same public Turn to be covered.
- The existing history save binding now distinguishes Turn boundaries and the
  exact manual/automatic compaction boundary. Marker registration is the single
  commit/cancel claim; an owned task carries append through dropped prompt
  waiters. Only confirmed save installs the projection. Explicit resume keeps
  the original candidate/range and does not rerun utility or tools.
- Runtime terminal callbacks wait at the compaction barrier before accepting any
  raw suffix. Save failure returns the original write fault and leaves the
  report suffix for explicit reconciliation. Force, close, Agent stop, load,
  unload and resource retirement honor utility/save ownership and task joins.
- Public compact/cancel DTOs are in `b03239b`. They distinguish live phases,
  original save faults, unchanged/cancelled/failed outcomes and confirmed
  compaction. Utility Usage remains separate from ordinary response indexes.

## Focused verification

With the task's fixed isolated Rust environment and target:

- `cargo test -p minicore-host-protocol --locked --offline`: passed, including
  two new strict compaction fact cases.
- `cargo test -p minicore-host-core --lib compaction --locked --offline`: passed
  14 Session/Runtime/tempfile tests. These include cumulative coverage/reload,
  one long public Turn, A/F steering, both admission orders, another Session,
  both commit/cancel orders, no-write/full-write/prefix-write original saves,
  real prompt cancellation and terminal callback fencing, missing old results,
  graceful/force/close/unload, and memory no-files/unchanged raw content.
- `cargo test -p minicore-host-resources --test task023 compaction --locked
  --offline`: passed the two new real loopback HTTP/SSE owner tests and updated
  supported-compaction capability test. Manual utility uses actual production
  model/prompt/schema preparation without creating a Turn or binding tools.

Self-review reproduced a valid historical missing-result tail being rejected
only at the marker commit after a new unhanded input arrived. Planning now keeps
that final accepted incomplete exchange until a later accepted record closes
its durable boundary. It does not protect the whole Turn, invent a result or
spend a utility call on the known-invalid cut. The regression passes.

An older large-verbatim HTTP fixture now explicitly disables compaction, since
its two-response server and assertion deliberately test uncompressed history.
No production fallback or silent disabling was added.

## Final code and aggregate verification

Owned commits to integrate, after the coordinator's clean prerequisites:

- `b03239b`: typed protocol facts, compact/cancel receipts and narrow Faults.
- `7171d8d436c00ce311ad4abe27520a6f779f4e39`: actual Session owner integration,
  resource-only manual binding, coverage/save/lifecycle boundaries and tests.
- `6893969eee546fd1469f236b3b596fa575c816ae`: strict cancelled/unchanged outcome
  decoding. Serde's tagged unit variants otherwise ignore unknown fields; a
  private strict wire shape preserves the public Rust constructors and JSON.

The parent-owned storage README correction `b69d0b8` was picked as `0b31670`
only after the exact `7171d8d` aggregate gate completed. Final production code
verification ran at `0b316707316bcf7de775a1ea1037acb79fa342db`, including the
protocol correction and unchanged core/resource source:

```sh
CARGO_NET_OFFLINE=true ./scripts/check.sh
cargo test --workspace --all-targets --locked --offline -- --list
cargo metadata --locked --offline --format-version 1
sha256sum SPEC.md
```

Passed whitespace and formatting, strict all-target workspace Clippy, **606
unique target/test cases**, **3 workspace doctests**, and strict rustdoc. The
list command, not nested storage subprocess output, supplies the unique count.
This work adds 14 core owner cases, 2 production-resource HTTP cases and 2 strict
protocol cases. The earlier exact `7171d8d` full gate also passed.

Metadata retains one Runtime 0.10.0 at the fixed `69faf919` revision with no
patch or version change. SPEC SHA-256 remains
`f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.

Validation logs, test lists and metadata are retained in the separate
`minicore-host-session-compaction-validation` directory. The final aggregate
log is `final-protocol-gate.log`; `final-test-list.txt` records its test inventory.
Only documentation changed afterward, with whitespace/format checks repeated.

The 606-test total includes unrelated local-tools code already present in the
quarantined base. It is not acceptance of that module. Integrate only the owned
commits into clean dev, not this worktree's full ancestry. No transport, Runtime,
engine, local-tools, registry, workspace-backend or TUI source was changed.
Independent review, transport adapter verification and clean-dev integration
remain with the coordinator. No live provider credentials or TUI were tested.
