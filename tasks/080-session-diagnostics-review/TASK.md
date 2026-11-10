# Independent Session and WSS diagnostic reviews

These are scoped internal approvals, not complete first-version or external
review approval. Production was unchanged by the reviewer. Requested
gpt-6-astra/max was not independently attested.

## Tool and process summaries

APPROVE at fixed source `101abcb25a416a81f8350ca1acdbe6078b0de0bc`, tree
`16cb1a24ea981d1fb4bd43c341ffede2567634c8`, for producer `0f6c36f` integrated
as `0e7b891`. Two new independent compound tests and the unchanged producer
test passed in a fresh exclusive target, with strict scoped Clippy, fmt and
diff checks. No full aggregate was claimed by this review.

Actual production Host/Runtime/local tools and loopback HTTP exercised four
Turns, five tool completions and four original process facts, including exit
7/9, SIGTERM and cancelled SIGKILL. The same ToolCallId across concurrent
Sessions/Turns retained the actual full chain. Callback re-entry, cleanup join
barriers, unconfirmed/panicked cleanup, original cancellation, closed logging,
unknown-field omission and body/credential exclusion passed. Ordinary read
business failure did not become infrastructure ERROR. Separate cleanup fixtures
are identified as fixtures, not production provider validation.

The fixed Runtime executes calls serially within an ordinary tool batch. The
probe therefore uses genuinely concurrent Sessions; it does not claim same-batch
parallel tools. Earlier incorrect probe assumptions and the corrected test-only
Tokio blocking rendezvous are retained in the evidence rather than reported as
production defects.

Evidence: coordinator `task080-tool-diagnostics-review/REVIEW.md`, SHA-256
`23a7c21e27360d503f70ea1b846e914d2de23ae495ee9b55a685bb038f337c71`.
The adjacent EVIDENCE.json identifies replayable probe patches, actual JSONL
and every raw check log; workspace locations are not public download links.

## WSS initialization fixture

APPROVE only the changed deadline test at
`f931bcc83e38371f11f7ea3a8a5ff01d81bc6da1`, integrated as `587318b`.
The file is byte-identical at both checkpoints, SHA-256
`7e847a867c140943a2ef0279b7c376da347821a3476c79864c046838e1755ddd`.
The exact case and five extra repeats passed with a fresh exclusive target.

Real TCP/TLS/WebSocket/authentication, an original 200 ms deadline, Pending at
199 ms and the exact WaitTimeout at 200 ms are preserved. The 201 ms outer test
guard rejects an incorrectly restarted budget; it does not extend the client
budget. The socket stays open so EOF cannot replace the asserted timeout.
The controlled red proves the CI failure mechanism, not an unchanged local
reproduction of CI scheduling. No production, timeout-policy or error changes.
Report: coordinator `task080-wss-deadline-review/REVIEW.md`.

## Continuing scope

History/Session metadata original-operation review targets the newer fixed
composition `d9ce4e9727204e0727943dc83d366dba9142604d`; it has not yet completed.
Static inspection of model attempt/usage summaries found no new confirmed issue,
but actual Runtime wait timing and its Host integration require their own final
verification. Do not expand either approval above to those later changes.
