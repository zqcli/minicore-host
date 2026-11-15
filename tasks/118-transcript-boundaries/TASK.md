# Task 118: Reasoning boundaries and visible transcript spacing

## Goal and authorization

On 2026-11-15 the owner supplied a screenshot showing concatenated thinking
headings and occasional two-row gaps between tool cards. The owner requested
the repairs, a 0.0.1 patch increment and Astra/max developers, and identified
earlier minicore-agent work as the reference. The current project workflow
updates the specification before production code, reviews independently and
publishes verified work to dev without rewriting history.

Status: complete. Implementation, independent reviews, complete local gates,
native desktop comparison and every required platform CI/package job pass at
version 0.12.5. Both exact source revisions are published on dev.

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

## Implementation and local Host verification

The specification-only Host commit is
`9c14da393b47fd7c65e136f3c6c60695736dfa43`; the TUI design checkpoint is
`da8bb62fe1b947618a446562595424de856a3df9`. Both precede Rust implementation.

The Responses adapter adds two constant-size state fields and twenty production
lines at its existing validated append boundary. Four additional provider
regressions compare observed, canonical, serialized and replayed reasoning,
including raw snapshot failures and preserved ordinary text. The provider's
95 tests and strict package Clippy pass. Independent Astra/max source review
found no correctness issue requiring a change.

The full Host `./scripts/check.sh` exits 0 with Rust 1.90.0, the committed lock
and offline dependency resolution. It passes formatting, strict workspace
Clippy, all workspace targets, seven doctests, strict rustdoc and 28 Python
checks. `cargo test --workspace --all-targets --locked --offline -- --list`
enumerates 1,259 unique Rust cases; storage subprocess output is not counted
again. All fourteen local packages move to 0.12.5; the 225 external lock
records remain identical to the 0.12.4 baseline. Storage schema and public
events are unchanged.

The TUI cache now separates only visible bodies, keeps zero-height identities
and reuses unchanged body chunks. Narrow empty Assistants follow the same rule;
partial markers and nonempty narrow rendering retain their existing behavior.
Six additional layout/render regressions cover visibility, cache reuse, native
history, folds, geometry, selection and internal whitespace. The existing real
embedded-Host test was extended and renamed to observe reasoning through
OutputBuffer and reopen durable history; it passes against the canonical Host
pin and is not an additional test-count increment. The focused results were 52 passed
and one existing ignored transcript case, plus 212 passed and two existing
ignored UI cases. Independent source reviews found no actionable layout defect.

## Canonical publication and complete TUI verification

The local source commits were published with exactly equal Git trees and a
checked expected dev head. All published commits are authored by `zqcli`.

| Checkpoint | Local source | Published source | Equal tree |
| --- | --- | --- | --- |
| Host specification | `9c14da393b47fd7c65e136f3c6c60695736dfa43` | `d76e25d467254e4af82573a6ebbd45ec88586e6e` | `ce3e2448beb934691977fce488bafe34f41852bb` |
| Host implementation | `36bfb9250a6e7fa89eafc90aa733f53e137afb0c` | `355d4b8bac4a3a634cd83a49fbf33fea7b67a262` | `cde4d491b3f9e25b9432635b41b89a5bf7f55120` |
| TUI design | `da8bb62fe1b947618a446562595424de856a3df9` | `1ed827641274ea3c05adedf7d41dc33965fb01fa` | `d124ffe55f06a8529bcc1d5fa86457031ebed450` |
| TUI implementation | `700e3c5531b020460cdec73c084a76c92b883abe` | `ecccdd9c2ea0ce99a29f121746e76629bbccc7c8` | `72803d8cdf8d10794249fa7063fef8773d713737` |

TUI's eleven manifest declarations, thirteen Host lock records and thirteen
resolved metadata packages pin Host `355d4b8bac4a3a634cd83a49fbf33fea7b67a262`.
All 272 external lock records, including dependency edges, sources and checksums,
match the baseline. The independent integration audit passes all seventeen
source/pin/version checks; its original pending-test notes remain a historical
snapshot, superseded by the completed evidence below.

The complete canonical-pin TUI gate exits 0 under Rust 1.90.0. It runs locked,
offline metadata resolution, `cargo fmt --all -- --check`, strict all-target
Clippy, `cargo test --locked --offline --all-targets`, doctests, strict rustdoc,
the crossterm dependency tree and an independent test listing. It passes 515
cases, with four existing ignored cases and zero failures, from 519 unique
enumerated cases. The 54 snapshot cases are included in that total. The real
embedded Responses observation/persistence/reopen regression passes. The first
gate exposed four snapshots containing the old version string; only those
version strings were updated, and the entire gate was rerun successfully.

The release build exits 0. The protected native verification executable is
31,911,472 bytes and reports `minicore-tui 0.12.5`; its SHA-256 is
`e8fbb8599d60ff207fba86acaed47ce437695f2e0f7337e99feae4b26fe7feb8`.

## Native desktop comparison

The coordinator operated the Linux terminal using native keyboard, mouse and
clipboard actions. The launcher supplied no simulated input or native history.
Separate data roots retained protected 0.12.4 and 0.12.5 binaries. A declared
local Responses fixture supplied the same four SSE bodies to each version;
their bytes match. Each run accepted one explicit prompt, completed one Turn,
made exactly four provider requests and executed real read, Bash and missing-read
tools. The last read deliberately returns a structured filesystem error shown
as a red tool card. Normal first-run and reopened-session exits all return 0.

| Scenario | 0.12.4 observation | 0.12.5 observation |
| --- | --- | --- |
| Live reasoning summary parts/items | Headings concatenate | Separate parts/items render on separate lines |
| Adjacent tool cards | Two blank separator rows | One blank separator row |
| Completed canonical transcript | Both defects remain | Both repairs remain |
| Reopened same Session | Both defects remain | Reasoning and spacing remain correct |
| Thinking hidden/restored and tool folding | Baseline observations retained | Correct visible spacing and functioning controls |
| Mouse expansion and Find | Baseline expansion retained | Correct Bash hit and sole final-answer match |
| Narrow and restored-wide window | Not a baseline claim | Correct reasoning and tool geometry |
| Bash internal whitespace | Preserved | Preserved, including both internal blank lines |

The canonical and reopened verifiers each pass sixteen checks for both versions.
No second prompt is sent after reopening. Native metadata/history and provider
evidence are unchanged across each restart; the baseline history SHA-256 is
`ba35b08801221a25afeda73f4ec6376028da867482ad73ef61518db5edfbf7e3`, and the fixed
history SHA-256 is
`7e13661b1b77d98ca10e0b2f49d9565c8457ccff35a57d7f62e599515cd68c94`.
All 48 unmodified screenshots and 51 logical actions are retained, including
desktop setup and the initial fixture preflight. Both loopback endpoints were
stopped and verified closed.

The initial fixture preflight rejected the Host's native input-provenance
envelope with HTTP 409. The rejected run and original fixture source remain
preserved; only the fixture validator was corrected before clean baseline/fixed
runs. The first baseline verifier incorrectly expected the missing-read outer
ToolOutcome to be failed; the native contract returns outer success containing
structured `status: failed`. The original failed result remains, and the corrected
verifier requires that exact structured filesystem failure. Neither fixture
correction is reported as a production regression or a successful live-model run.

## Platform CI and retained evidence

Both exact published code checkpoints passed their complete CI workflows on
attempt 1. The coordinator checked the completed workflow conclusions and every
job on 2026-11-15:

- Host source: `355d4b8bac4a3a634cd83a49fbf33fea7b67a262`,
  [run 43726944284](https://github.com/zqcli/minicore-host/actions/runs/43726944284):
  all nine jobs succeeded. These are Linux quality, native Clippy/tests on
  Linux, Windows, macOS Intel and macOS ARM, and packages for those four targets.
- TUI source: `ecccdd9c2ea0ce99a29f121746e76629bbccc7c8`,
  [run 43727578961](https://github.com/zqcli/minicore-tui/actions/runs/43727578961):
  all eight jobs succeeded. These are quality, Linux/macOS/Windows Clippy/tests,
  and four native platform packages.

This documentation-only closeout changes `PROJECT.md` and this task with
`[skip ci]`. The implementation, tests, versions, SPEC, locks and TUI Host pin
remain exactly those of the fully validated code checkpoints above.

The separate `minicore-fixes-0125-qa-20261115.zip` evidence package contains the
report, original test logs, source/pin/hash audits, independent reviews, explicit
fixture sources, HTTP/SSE records, native histories, terminal captures and
unmodified screenshots. Repository source and executables are identified by
exact Git revisions and digests instead of duplicated in that package. Eight
earlier archives/reports remain byte-for-byte unchanged.

## Limits

The fixture simulates only model responses; native Host execution, tool results,
history and UI operations are real. It establishes these two display repairs,
not live-model quality, long-run reliability or performance. GUI observations
are Linux-only; other native platforms are covered by the separate CI jobs.
Old reasoning already persisted as joined text is not rewritten or inferred
from Markdown/capitalization. New reasoning keeps its boundaries through reopen;
the visible-block spacing repair also applies when rendering existing history.
Other deferred product findings remain outside this task. No tag, main merge,
history rewrite or force update is performed.
