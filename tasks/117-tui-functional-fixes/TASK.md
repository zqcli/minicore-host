# Task 117: TUI query paste, forward delete and Session identity

## Goal and authorization

On 2026-11-15 the owner approved exactly three repairs, required a specification
update before implementation, requested `gpt-6-astra` developers with `max`
reasoning and no Codex CLI, and requested a 0.0.1 version increment. The two
earlier UI-01/UI-03 findings and the current discussion's items 2/1 are the same
two scopes. The newly confirmed long-title Session-picker ambiguity is the
third scope. Task 117 was unused when this work began.

Status: specification prepared; implementation, integration, independent review
and validation pending. No production fix or new native-terminal pass is
claimed by this initial task record.

## Starting checkpoints

- Host: `aeb09650d01a32c98a7266e8e19f92efebbb5be4`, version 0.12.3,
  following Task 116's completed Windows fixture and documentation follow-up.
- TUI: `0b7d4dc799fbe8c124661713bf8300fc3686ca78`, version 0.12.3,
  embedding Host `dbaaa8175780759b84d26e0519aa4b27ced5f8f4`.
- Previous SPEC: 4.11.2,
  SHA-256 `19fab2cf90270200430ca1b229a4839bfc17ea2862ea60a22f8e87ae3cdfbd56`.
- Target: Host/TUI 0.12.4 and SPEC 4.11.3. Native object and history formats
  remain schema 3; external dependencies require no change for these fixes.
- New SPEC SHA-256:
  `21108befba6187fcc1355f9b11caae597c3d3f4b6611b5cea7e712c1f6e8b735`.
  Specification and implementation commit IDs are pending.

## Evidence and minimal contracts

The companion TUI repository's `qa/session-names/PENDING_FIXES.md` and
`qa/session-names/RESULTS.md` retain the baseline evidence and historical
authorization state. The later authorization above permits this repair batch;
it does not turn those earlier observations into successful fix validation.
The frozen long-run and functional reports remain unchanged.

1. **Overlay query paste.** Source inspection of
   `src/app/keys/composer.rs::App::on_paste` confirms that Find, Sessions,
   Models and Agents currently reject terminal paste. Route only these active
   query/filter fields to their existing query refresh logic, append to the
   query and preserve the draft. Reset result selection as typing does; changed
   Session filters clear pending delete confirmation. Normalize CRLF, lone CR
   and LF to one space per line break, TAB to four spaces, then apply existing
   terminal sanitization for ANSI/OSC/BOM removal and visible other-control and
   bidi-control escapes. Preserve ordinary Unicode; normalized-empty paste is a
   no-op. Paste never accepts, submits or triggers a command. Existing editor,
   interaction and read-only boundaries remain.
2. **Forward-delete history.** Source inspection of
   `src/app/composer.rs::Composer::delete` confirms that unselected deletion
   has no pre-edit snapshot, and both physical Delete and nonempty Ctrl+D use
   this method. Each actual deletion is a separate undo step and clears stale
   redo; no-op deletion preserves valid redo. Restore text, graphemes, complete
   paste chips, cursor and selection atomically. Keep selected deletion as one
   step and preserve empty Ctrl+D's quit meaning.
3. **Session-picker identity.** Source inspection of `src/ui/sheets.rs`
   confirms that short IDs appear only for equal full labels, before
   `src/ui/panel.rs` clips the title. Always display the existing six-character
   native Session-ID prefix and reserve it before title and optional-detail
   clipping. Remove the need for full-label duplicate detection; do not add a
   collision registry or change stored titles. Actions keep full native
   `(agent_id, session_id)` targets. Stable IDs remain visible for idle/running,
   unique/duplicate/automatic/fallback names and after filtering or renaming.

All three changes belong to TUI presentation and local input state. Host ->
Agent -> SessionController, Runtime boundaries and native business ownership
remain unchanged. Host receives the specification and patch-version update,
with no new production behavior or data fields. The independent Web client,
themes, F4 help mismatch, clipboard readback, provider history limitations,
Session rename editing and the other deferred QA findings are excluded.

## Ownership and implementation order

Workers were requested with model `gpt-6-astra` and reasoning effort `max`;
these requested settings are recorded without claiming independent attestation
of the effective runtime. No Codex CLI or coding-model subprocess is used.

The specification worker owns `SPEC.md`, its `AGENTS.md` hash, this task's
initial record, the new `PROJECT.md` entry and TUI `docs/design-host.md`.
Implementation workers wait for the specification checkpoint and then own
disjoint TUI editor/routing and picker-rendering paths. The coordinator owns
version manifests/lockfiles, exact Host dependency pin, release documentation,
final integration and validation records. Shared files are coordinated before
editing. This specification checkpoint contains no Rust production changes.

## Validation plan and current outcome

- Focused editor and app-key regressions cover both forward-delete keys,
  pasted and typed drafts, earlier undo history, preloaded text, abandoned redo,
  no-op deletion, selections, CJK, emoji, combining graphemes and paste chips.
- Query-routing regressions cover all four surfaces, appending, Unicode,
  CR/LF/TAB/controls, match and selection refresh, pending deletion and draft
  preservation. Existing specialized paste routes and read-only guards remain
  covered without broadening the repair.
- Actual-width frame tests cover different long titles clipping alike, idle
  and running rows under one Agent, exact duplicate names, narrow layouts,
  renaming and stable full target identities. A string before clipping is not
  sufficient evidence that the ID is visible.
- Repeat relevant native-terminal sequences when available and record each
  observed result separately from reducer/editor/rendering tests. Agents paste
  had only source-inferred baseline coverage and needs its own observed result;
  neither that scope nor other unrun checks are reported as native passes.
- Run the existing locked formatting, strict Clippy, Rust tests, doctests,
  strict rustdoc, repository checks and unique test enumeration, then the
  established native-platform and packaging gates for the exact checkpoints.
  Record blocked or unrun gates explicitly.

Initial preparation inspected both repositories' `AGENTS.md`, the two named
QA records, the relevant input/editor/rendering sources and existing task IDs.
Only specification and planning documents have changed at this checkpoint.
The coordinator will record exact commits, commands, outcomes, limits and next
steps after implementation and verification; no test totals are prefilled.
