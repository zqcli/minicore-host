# Task 121: Accept Responses metadata and advance Host to 0.12.9

## Scope and authorization

- Baseline: Host `dev` at `2a88d4771bf7a12c14f6290dbe0b6ce265092886`, version 0.12.8.
- The owner requests the diagnosed provider repair, a one-patch Host version
  increment and a normal push to `origin/dev`, explicitly without local tests.
- No TUI revision change, installed-binary replacement, release tag, main merge,
  failure-pause/resume change, storage migration or external dependency update.

## Reproduction and cause

Two isolated live requests through the installed TUI 0.12.8, its production Host
and the configured Responses gateway returned HTTP 200. Both streams ended with
`function_call_arguments.done`, `output_item.done`, `response.metadata` and
`response.completed`. The unknown-event branch rejected metadata before the
valid terminal could be consumed, preserving only a partial tool call. No tool
executed. Both captures pass the original adapter when only the metadata frame
is removed, without rewriting any other bytes or sequence numbers.

Raw streams, prompt bodies and private session evidence are excluded from Git.
The preceding investigation's local report is under
`../minicore-remote-build/provider-0128-repro/REPORT.md`.

## Implementation

- Recognize only `response.metadata` after existing error/sequence checks.
- Require a started, nonterminal response with a matching typed `response_id`.
- Skip advisory payload without emitting output, usage, tool ends or completion.
- Preserve error-envelope priority and duplicate-known-field rejection.
- Keep all other unknown-event, snapshot and incomplete/failed-stream checks.
- Add seven synthetic loopback regression functions for success positions,
  identities, sequences, incomplete termination, explicit errors and conflicts.
- Advance fourteen internal lock records, shared requirements and the storage
  crate's exact protocol requirement to 0.12.9; update current documentation.

## Verification boundary

The earlier diagnostic prototype accepted both unchanged captured responses and
passed 104 provider-related cases, including seven metadata regression functions.
That result predates this delivery and is not a claim that the final 0.12.9 tree
was tested. The independent diagnostic review was requested with `gpt-6-astra`
and `max` reasoning; requested settings are not runtime attestation.

For this delivery, local test/build/lint/doc execution is omitted at the owner's
explicit request. Only changed-file formatting, static source/diff review and
manifest/lock consistency checks are performed. Static comparison confirms all
225 external lock records are unchanged, fourteen internal versions are 0.12.9,
and both production files match the previously examined candidate. Independent
read-only delivery review, requested with `gpt-6-astra` / `max`, found no blocking
issue and ran no tests or Cargo commands. Any subsequent remote CI result belongs
to its exact pushed commit and is not asserted here.