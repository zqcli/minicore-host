# Host 0.11.0 independent review

The design baseline was committed before implementation as SPEC 4.10.2 in
`32ea194ff2269aa8fb2bebff79355b600d404b45`. Review used the actual source and
the approved Host-only scope. The requested worker configuration was
`gpt-6-astra` with `max` reasoning; effective runtime settings were not
independently attested. Review findings do not substitute for test results.

## Coverage and ownership

| Change | Implementation owner | Independent source review |
| --- | --- | --- |
| Responses provider | Responses worker | Root: shared HTTP safety, typed events/error priority, terminal ordering, native replay and canonical ranges |
| Single-file patch | Patch worker | Root: reference parser adaptation, existing mutation/cleanup owner, exact captures and policy integration |
| Configuration/resource/release integration | Root | Patch worker: explicit API selection, safe discovery, frozen bindings, policies, templates, package inclusion and integration fixtures |
| History and activity ownership | History worker | Patch worker and root: native positions, compaction batches, checkpoint coalescing, load/retirement fences and cursor windows |
| History storage/public projections | History worker | Responses worker: schema gates, mandatory fields, terminal statistics, timestamps and RPC/tool projection agreement |

## Findings and dispositions

### Automatic compaction overtook an already-bound input

The first implementation exposed an input at native position N before saving
it. Automatic compaction could then accept its marker at the unchanged journal
position N. The retained and persisted views would disagree on both the input
position and its timestamp.

The fix includes the already-bound unsaved input prefix before the marker in
the original compaction append batch. Input positions and `recorded_at` values
are reused, marker `source_position` follows the prefix, and `save_cursor` and
the accepted history position advance once. The existing journal owns the same
batch through failure/reconciliation. The marker is published once on confirmed
compaction, and later Turn finalization cannot append those inputs again.
Independent targeted source review found this path resolved; focused regression
and integrated test results are recorded in the task validation checkpoint.

### Recovered activity could miss the retirement fence

Explicit and driver loading recovered a newer activity time from valid saved
history, but could publish quiescence before a later wake registered the metadata
checkpoint. Concurrent retirement could then exclude that save.

Both load completion paths now register the checkpoint under their existing
owner lock. A newer activity time racing an older metadata save is coalesced;
the successful settlement registers the next candidate before exposing
quiescence. Failed saves remain retained and are not automatically retried.
The patch reviewer rechecked both load paths and the coalescing/failure boundary.

A final review of the control-action tests identified the same registration
requirement at history/partial/terminal acceptance. Those boundaries now register
the activity save under their existing lock before publishing accepted history;
settlement does not defer registration to a later lock acquisition. The direct
gated-append regression confirms metadata is owned and can settle while the
history append remains gated. The independent Responses reviewer rechecked this
fix and root's bounded owner waits in `cf4bd05`.

### Activity-only metadata saves cleared resource faults

The existing configuration-save completion also cleared resource/configuration
faults. Reusing it for activity-only checkpoints initially caused an unintended
new resource-preparation attempt. Fault clearing is now conditional on a real
configuration-revision change; activity-only publication retains the original
fault and execution boundary.

### Public projections dropped original metadata

The agent tool's message-inspection projection omitted `received_at` and
`history_position` after the core query gained them. Retained Turn state/results
and completion events omitted original terminal timestamps even though cold
saved results exposed them. The fix projects existing accepted values into
those surfaces; it does not generate a time during a query or introduce another
record owner. Cold `saved_terminal` and retained execution state keep their
different authority and persistence meanings.

### Durable terminal and Session metadata needed semantic validation

The initial schema extension made timestamp/counter fields mandatory but did
not validate contradictions such as a saved response index 0 followed by a
terminal claiming zero requests. Session object validation also admitted an
activity time before its creation time, contrary to equal initialization and
max updates.

The storage changes validate guaranteed relations against accepted facts,
including request/attempt shape and actual known response/tool-result bounds.
The request admission boundary reserves a zero-attempt slot so a local failure
before an observed attempt remains truthful. Validation does not fabricate
unrecorded attempts or require wall-clock ordering between history records.
Fixtures must carry coherent counters, and invalid load/save cases retain the
original bytes. Targeted source re-review found no remaining issue; the final
integrated gate records executed test coverage.

### Integration fixture correction

The initial binary read-only policy test supplied malformed headerless patch
syntax. It now supplies a valid Codex Update File envelope for the actual
existing file, so the denied call would otherwise be executable. The actual
Host binary's policy matrix passed after the correction.

### Streaming hash implementation

Root review requested that normalized Responses text be hashed in unchanged
UTF-8 runs rather than one character per hash update. The implementation now
retains correct control/CRLF handling and original range mapping while ordinary
deltas use a single update. No second cumulative text or argument buffer was
added. Provider tests include multibyte and split-CRLF replay.

## Verification boundary

The first full integration runs and native CI exposed stale fixture assumptions:
terminal rows and explicit compaction markers were omitted, independent
executions were compared with identical TurnIds/times, and configuration/unload
actions raced already-owned activity saves. The follow-up tests preserve exact
envelopes across event, retained and cold views of the same execution. Only
inherently different accepted identities/times are normalized when comparing
two distinct executions. Bounded observation waits settle the original owner
before one control action; they do not retry mutations.

Windows required renaming an embedded-NUL-content fixture from the reserved
device name `nul` to `embedded-nul.bin`; all rejection and unchanged-byte checks
remain. Its long-Turn fixture now separately bounds actual request admission
and final completion while retaining all four-request, three-tool-round and
native-history assertions. The previous generic aggregate timeout did not
identify a slow phase and is not evidence of a production root cause.

The final integrated Linux gate passes 1,178 unique Rust cases, seven doctests
and 28 Python checks plus formatting, strict Clippy and strict rustdoc.
[Validation evidence](validation.json) identifies the exact matching local and
published source trees and distinguishes native CI from local execution.

An independent evidence review checked all 15 local-to-remote source commit
mappings against the fetched Git objects, including trees, parents, authors and
messages. It recomputed the local test counts, checked the SPEC hash, workspace
versions and unchanged external dependencies, and opened the actual local Linux
archive to verify its file count and archive/executable hashes. The earlier
package documentation checkpoint remains distinct from the final source CI.

The component notes record passing provider and patch gates. Root's resource
tests exercise real Host, Runtime, provider, workspace and edit owners with
explicit loopback model replies. The Windows native job and five standard-user
cases passed; their execution evidence is recorded separately from Linux
results in the [CI record](ci-source.json). No live external model or TUI
validation is claimed. The task log records the integrated source checkpoint,
full gate, CI and publication status.
