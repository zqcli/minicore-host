# Task 124: Agent action defaults and delegation grants

Baseline: Host `dev` `fc971505eb36efe46762aabe0ca5ff8349634c46` (0.12.16).
Package and SPEC version labels remain unchanged. Workers are requested with
`gpt-6-astra` / `max`, not claimed as independently verified effective runtime
settings. No Codex is used.

## Scope and decisions

- `create` without `resource` requires nonblank `text` and uses the existing
  Session creation path: same-Agent memory by default, current settings inherited,
  explicit Session overrides retained, and asynchronous acceptance/results.
- `inspect` without `target` reads the current Session. Configuration and output
  remain opt-in; explicit targets retain their required fields and validation.
- New delegations through `create` with text or first `send(delegate:true)` are
  allowed to the root and to explicitly authorized active members. Instead of a
  hard-coded depth gate, only a root can give the new recipient
  `allow_delegation:true` and optional positive `max_children` (default 2).
  The grant belongs to that active delegation, not to Session history or config;
  a member cannot pass the grant on or add it to an existing relation.
- Both start paths use the same concurrent limit for an authorized member's
  direct children, including pending starts. Release or failed startup frees an
  occupied slot. Ordinary `send` remains steer and defaults to `delegate:false`;
  appending to a current relation neither grants rights nor consumes a slot.
- `wait`, `update`, `control`, target kinds, closure and lifetime behavior stay
  unchanged. No new parent alias, team API, resource scheduler or client change.
- The model-facing tool description explains the short forms, current grant
  policy and distinction between sending messages and starting new delegation.

The contract and examples are in `SPEC.md`'s agent tool sections. Implementation
is in `crates/host-tools/src/{action,schema,scope,tool,core_adapter}.rs`. Creating
with text reserves a slot before owner creation I/O to avoid a quota rejection
after creating a persistent object. Both dispatch paths charge the same quota.
Registration transfers the reservation to the child relationship; unregistration
releases it. There is no lock held across creation I/O and no new durable state.

## Verification and handoff

Fixed Rust checkpoints: `ba8fc93c6f930de013e916bf59b276b2c2dd6067` for action
defaults; `bff1e2e5049ee2a4c0c05a8e66786820cd72009a` for grants, shared quota
and prompt. Compare the latter against the baseline above for code review.

- Rust 1.90: `cargo fmt --all -- --check`, `cargo test -p minicore-host-tools
  --all-targets --locked` (79 passed), `cargo clippy -p minicore-host-tools
  --all-targets --locked -- -D warnings`, and `git diff --check` passed.
- Host/Runtime integration uses scripted model streams and real owners/tool
  dispatch. It covers shorthand defaults, permission rejection on both paths,
  shared cap and reuse after handoff, no grant inheritance or reuse, rejection
  of attempted regrant on append, and unchanged ordinary messages.
- A gated owner-port fixture checks pending quota across concurrent creation
  and send, rejection before another owner creation call, and reuse after
  failed preparation. The actual ToolSpec JSON Schema passed 19 positive and
  48 negative cases with a Draft 2020-12 validator.
- No live provider, TUI/Web interaction, remote CI or independent review is
  claimed here.
