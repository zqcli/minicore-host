# Host tools

The statically compiled `agent` tool reuses Runtime `ToolSpec`, `Tool`, and
`ToolSet`. Its model inputs expose only the six actions in SPEC v4.93 §7.
Identity, operation/message ID generation, source capabilities, and configuration
revision selection are backend responsibilities, never model-supplied authority.

`TurnPort` is a narrow, Turn-bound owner integration boundary. Implementations
must use a trusted internal capability minted by Session before input claiming,
not a copied client `CallerContext`. Delegation permits and result sinks have
independent lifetimes, matching the registered scope and direct parent. The
adapter must receive necessary actual results before a memory child releases.

## Current stage

Strict action contracts, Runtime tool declaration, and snapshot configuration
mapping are implemented. The owner ports describe the pending trusted core
integration and are not themselves a production Host implementation. Contract
tests do not claim a live provider, real Host delegation, or TUI integration.

No remote push is authorized. No TUI files or SPEC are modified.
