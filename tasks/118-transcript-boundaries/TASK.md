# Task 118: Reasoning boundaries and visible transcript spacing

## Goal and authorization

On 2026-11-15 the owner supplied a screenshot showing concatenated thinking
headings and occasional two-row gaps between tool cards. The owner requested
the repairs, a 0.0.1 patch increment and Astra/max developers, and identified
earlier minicore-agent work as the reference. The current project workflow
updates the specification before production code, reviews independently and
publishes verified work to dev without rewriting history.

Status: specification checkpoint prepared; implementation and validation are
pending. This record makes no repaired-build or native-terminal pass claim.

## Baseline and target

- Host: `3b7fee78a2749a0e561ce83b8f2d76b4cd4c35d4`, version 0.12.4.
- TUI: `be80bfe570358a6d9f408f321ffe817a9acec733`, version 0.12.4, embedding
  Host `667f0923b827fea9b5fdb60f75a21f5369e44193`.
- Previous SPEC: 4.11.3, SHA-256
  `21108befba6187fcc1355f9b11caae597c3d3f4b6611b5cea7e712c1f6e8b735`.
- Target: Host/TUI 0.12.5 and SPEC 4.11.4. Keep schema-3 storage and the locked
  external dependency graph unchanged.
- New SPEC SHA-256: `e5d61422fa661b6730bb5d189e35259a99853ed2f9612af7c476ad1cf302c3e1`.

## Confirmed causes and reference behavior

The Responses adapter validates item and part identities but emits all reasoning
fragments without a boundary separator. Runtime correctly accumulates those
fragments into one canonical reasoning string. The TUI already preserves LF as
Markdown hard breaks, so the missing boundary is lost before rendering.

Agent commit `d7346e6c4dfde49bd82047e6db89b807298491e3` introduced the relevant
reasoning behavior; the current reference keeps it in
`src/models/openai/reasoning.rs`. Separate actual nonempty reasoning parts/items
by one LF only when neither side supplies CR/LF; do not infer boundaries from
capitalization, Markdown or transport chunks. Current Host identities remain
strictly validated; legacy missing-identity compatibility is unnecessary.

The current native history projection keeps an empty Assistant for a tool-only
response. Its renderer produces zero rows, but Layout inserts a separator and
forces at least one row for every logical block. A tool card, empty Assistant
and next tool card therefore produce two visible separator rows. Historical
TUI commit `f1f8cf3abcf0068b7f4e68c653eb48543d173582` fixed this same symptom by
filtering the old projections after live-overlay handling. Current native
canonical-response detection uses retained Assistant identity; reuse the old
visible result while making zero-row layout entries consume no rows or gaps.

## Ownership and minimal contracts

- Provider owns reasoning boundary insertion and its emitted-text checksum.
  Preserve original part fingerprints, completed native items, encrypted data,
  strict lifecycle checks and existing ordinary-output text behavior. Runtime,
  public event types and durable formats need no new abstraction or field.
- TUI Layout owns separation between actually visible blocks. Keep native
  history identity and positions, late-output suppression, meaningful internal
  tool/code/prose whitespace, cached body reuse and correct visible geometry.
- Existing Tasks 115–117 configuration, errors, query paste, deletion history
  and Session identity contracts remain. The screenshot's model-generated tool
  arguments do not expand this task into tool-schema or Agent behavior changes.

Workers are requested with `gpt-6-astra` and `max` reasoning; requested settings
are recorded without independent attestation of effective runtime internals.
No Codex CLI or coding-model subprocess is used. Contributors wait for the
specification checkpoint, then own disjoint provider/integration and frontend
layout/rendering files. The coordinator owns shared version/lock files, release
documents, final integration, publication and validation records.

## Validation plan

Provider regressions cover same-part fragments, multiple summary parts and
reasoning items, empty parts/deltas, existing CR/LF and split CRLF, done-only
content, Unicode, duplicate completion snapshots and partial failures. Compare
observer reasoning, canonical response, serialization and native replay; retain
ordinary-text and Chat Completions coverage. Extend the real embedded Host
Responses fixture through persistence/reopen when useful.

Frontend regressions use native history with tool-only responses and zero-row
Assistant projections. Check exact visible spacing during streaming, canonical
installation and reload; leading/trailing/multiple empty blocks, folds,
reasoning visibility, resizing, cache reuse, anchors, hits, search and selection
must remain consistent. Internal blank lines retain their meaning. Add focused
native-terminal verification with a declared local simulated provider and keep
its observations separate from source and fixture tests.

Run existing locked formatting, Clippy, tests, doctests, strict rustdoc, unique
test enumeration and platform CI/package gates. Record exact commands, commits,
counts and unavailable checks after execution. Earlier QA archives remain
historical evidence and are not rewritten or relabelled as new passes.
