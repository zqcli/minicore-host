# Task 125: Responses WebSocket and hosted search

- Comparison baseline: `d91157e737b8889f918948f476e099c3e4382df8`.
- Feature checkpoint: `79443c989416ffdacf0af21bf8ed2361272c45d5` (same source tree as tested local commit `5342f65488b5c33e2e1ddcd41635ef89750a376a`).
- SPEC: 4.11.8. Package version unchanged. No companion repository changes.

## Design

The model entry's existing Responses endpoint selects HTTP/SSE or WS. WS sends one
authenticated `response.create` per connection without provider conversation
state, socket pooling or fallback. Both transports feed the same decoder.
The configured connection timeout and Runtime deadline remain in effect. Local
plain WS is limited to literal loopback, and the existing inbound JSON-RPC is
unchanged.

The optional per-model `web_search` setting selects `live` or `cached`; omission
leaves it disabled. The provider performs search within the model response. The
adapter stores native search results and URL citations in provider replay and
replays same-provider-model search items; it does not replay citation positions
against normalized display text. No local tool, standalone search route or new
client view is added.

References: Pi Rail UI `8e55c31441002ea5684f4c4550fab55e548ef8c0`,
`commands/rail-oai-search.ts` and `openai/responses-websocket/transport.ts`.
The design uses Pi's hosted tool, not its provider-side continuation cache.

## Validation and handoff

- Rust 1.90.0, locked dependencies. Focused Provider gate: 106 unit/integration
  tests and 2 doctests passed. Real loopback HTTP and WS and real Runtime with
  scripted upstream events, not a live provider.
- WS coverage: handshake path/authentication, Ping/Pong, shared streamed text,
  full-context replay into a second connection, missing terminal with partial
  text preserved on the final attempt. Search coverage: live/cached payloads,
  search lifecycle, retained URL sources/citations, same-model replay and no
  local tool invocation.
- Full Linux workspace gate: `./scripts/check.sh` with a writable isolated
  HOME, temporary build directory and pinned toolchain. Includes the three
  Python suites, rustfmt, strict Clippy, workspace tests/doctests and strict docs.
- No live WSS endpoint, provider credentials or TUI/Web acceptance exercised.
  Independent review pending.
