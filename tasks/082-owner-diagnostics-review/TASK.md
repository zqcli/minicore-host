# Independent Host and Agent diagnostics review

## Initial lifecycle scope

APPROVE at `7781a1d8f58967dfb76d52fbef6a224f813911d6` for the initial
Host/Agent lifecycle slice. The original three probes, 71 regression cases,
eight supplementary checks and strict lint/fmt/whitespace passed independently.
Span destruction no longer invokes callbacks under admission; repeated and
store-release-window shutdown keep immutable actual Host mode. The public
300-round race did not reproduce the narrow mode window, so no dynamic red is
claimed for that static invariant correction.

Evidence archive SHA-256:
`57a7fd8b3802284b8ac52042bce618bfb7e6a9c6cee6d816b39f61cdecde1a88`.

## Metadata scope

APPROVE only the incremental source
`19e7bbba78f70be61732452d34fbfc717e564e15`, based on7781a1d. It covers Agent
metadata save acceptance/result/reconcile, resource refusal and original
operation forwarding through close/Session delete/Agent cascade. The reviewer
did not change production or claim the later root composition as independently
executed.

Fresh independent results: 317 existing core cases, five added probes repeated
20 times, strict core lib/tests Clippy, strict rustdoc, fmt and diff checks.
Actual persistent failures, unchanged committed revisions, original immutable
receipts, independent invocations sharing one operation ID, caller disconnect,
all tracing callback kinds re-entering direct owners, and no/closed logging
passed. The direct Agent logs the original primary ERROR once; intermediate
forwarding neither relabels it nor duplicates it. A pending-save cascade
refusal was initially mis-modelled by the reviewer fixture; that failed log and
the corrected existing-contract assertion are retained, not a production fix.

Evidence archive: coordinator `task082-metadata-review-19e7bbba.tar.gz`, SHA-256
`d58cc104075f22ec8a9018571d2e08c93836eed3f750c79b851c05f8e024fac9`.
Its report, source/probe, real JSONL, complete logs, hashes and replay script
identify the exact reviewed source. These workspace artifacts are not public
download links. Later root587318b aggregate908/4/20 and remote2615 CI success
are separate integration evidence, not added to this independent test count.

## Reclamation scope

A separate scoped APPROVE covers only
`28a09dc128ad60d3ea53491054666e2c6ef15ef7` (integrated as26fc72a):
45 targeted regression cases and four new independent probes, repeated20 times,
all pass, with strict focused Clippy, rustdoc, fmt and diff checks.

Actual Runtime-driven reclamation releases model/resource/Host weak references
while observer and logger guards remain. Persistent cascade success, directory
deletion failure and accepted partial memory creation retain original mode,
operation and receipt facts; failure does not fabricate Retired, another
acceptance, or an unpublished Session ID. Subscriber callbacks re-enter all
three owner query boundaries without blocking. Existing invalid empty-cascade
retry was first mis-modelled by the test, then correctly asserted as a refusal;
no production fallback was added. Join/panic branches received source review
only, not claimed dynamic coverage. The evidence package is being finalized.

No approval above extends to the later combined root or Session-operation patch.
No main merge, push, TUI/live-provider or complete product approval occurred.
The requested gpt-6-astra/max runtime configuration is not independently attested.
