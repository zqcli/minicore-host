# Own Session compaction and durable commits

Status: implementation in progress. No operational acceptance or push claimed.

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

## Implementation and verification

Pending. The intended gates include repeated reloadable coverage, same-Turn
compaction, A/F input rebuild, manual/startup exclusion, both cancellation claim
orders, original save continuation, prompt cancellation during save, missing
old results, and lifecycle joins. Test doubles and local HTTP tests will be
identified honestly; no live provider claim is made.

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

Full workspace lint/test/doc gate and exact unique counts are pending below.
Transport and independent review remain separate. No push.
