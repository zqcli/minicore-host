# Task096: Composed backend checkpoint and evidence handoff

## Fixed scope

Source 50a1b2fedae73ce5c8b0a461a5bd6ed0edce68c8 is published as
53167d3117323d2b126c453fbb53fa980bdb72ee, exact tree
b616d753d0d35622ccefa10dc7752b7b2df21c46. Compare published ccadc3013d084a47a3a9cfcd48fcfffdeca34253.
The last five commits add real compaction diagnostics, original Fault handoff and
query-fixture integration; no Runtime/event policy change. The cumulative map 99
retains every previous remote object and records actual API authors. Main is unchanged.

This handoff also integrates scoped reports 091/092 and the separately measured
Task090 evidence, without changing production after the fixed gate. Source report
commits e39c1c8/e3228f9 map locally to b573a59/5fa6105; performance aa64dc7/9838865
maps locally to 4c10d8b/18590f4. These are source-to-integration identities, not
invented remote commit IDs; the next publication map records the actual remote IDs.

## Executed validation

At exact source 50a1b2f, Rust 1.90.0, locked published Runtime 3aa4762, two Cargo jobs,
no incremental/debug info and an exclusive integration target:

```sh
CARGO_NET_OFFLINE=true ./scripts/check.sh
cargo test --workspace --all-targets --locked -- --list
```

Both exited 0. Inventory lists 937 unique cases across 124 nonempty programs; four
doctests and 20 Python checks also pass. Strict all-target Clippy, strict rustdoc,
fmt and committed whitespace pass. Real app credential reload/output/history,
process cleanup, typed/stdio/WSS and saved-result/compaction tests ran. Counts
exclude nested storage child-process duplicate output. See the committed gzip logs in
`evidence/` (exact original bytes retained, both hashes recorded) and [manifest](../../docs/checkpoint-50a1b2f-validation.json).

The first compaction composition run passed 23 core cases but failed compiling two
new multiline Option-field fixture accesses. After mechanical adaptation, all 13
selected transport cases passed and the fresh complete gate above passed. The
failed and corrected logs are retained. An earlier CI 39b115e failed at the App
process fixture's Option access; 0cc9e61 and the actual 13-process suite fixed and
verified it. Neither fixture failure is presented as a production failure or a
successful prior run.

Task090's 8 parser checks and raw evidence verifier passed after integration,
separate from the 20 checks at the fixed code gate. Its checksum manifest is
historical: 28 unchanged paths pass in the integrated tree; its 29th entry is
the original f36761a Cargo.lock, verified against that fixed Git blob, not the
new Runtime pin. A direct whole-manifest check in this newer tree correctly
reports that one lock mismatch. No evidence hash was rewritten to hide it.
Its original f36761a production identity and measured/source exclusions remain
unchanged. Raw performance data use only explicit fixture values, not real
credentials or a live model account.

## Remaining boundary

The exact-head hosted run 42749273803 passed; preceding ccadc run 42748451681
also passed. Later 053 force/preparation/internal-message/scheduling
slices, their review, final composed validation and external review remain. No
TUI or live external-provider acceptance, capacity limit, final product approval
or main merge is claimed. Requested worker configuration remains Astra/max;
effective runtime configuration was not independently verified.
