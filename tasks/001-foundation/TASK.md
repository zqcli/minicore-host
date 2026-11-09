# Build Host Foundation

Status: active.

## Goal

Implement typed ownership contracts, durable object/history storage, connection operation rights, Session double queues, and a real pinned Runtime vertical slice. Preserve the full v4.93 scope and record subsequent work explicitly.

## Source and environment checkpoint

- Full spec copied from the user's exported v4.93 source; SHA-256 verified as `f044519fd361c9842f41696d6309d56c04598f5d481c66e94737079ba9491bd3`.
- `git clone https://github.com/zqcli/minicore-host.git` completed with the expected empty-repository warning.
- Linux x86_64 with Git/C compiler/curl available; Rust/Cargo initially absent.
- Official Rust installer downloaded from `https://sh.rustup.rs`; install to workspace-local Rust/Cargo homes is in progress.
- No remote writes, TUI changes, provider calls, or passed integration checks yet.

## Acceptance priorities

1. Strict IDs, text-only messages, typed receipts and direct ownership.
2. Real Session acceptance; independent next-step/next-turn FIFO; no host-wide execution lock.
3. One active Turn; normal finish races with steer at one owner boundary.
4. Complete Runtime records handed off at response/tool-batch boundaries; persisted only after required sync.
5. Cancellation joins and receives authoritative partial output; no reconstruction from observer events.
6. Stable persistent identities, fresh incarnation, exclusive root, strict corruption detection; no execution recovery.
7. First valid connection controls; read-only connections never silently upgrade; stale callers cannot accept new writes.

## Remaining full-version scope

All unimplemented methods and lifecycle branches, DSH delegation, complete provider/tools, stdio/WSS, event/read synchronization, independent review, and real TUI integration. These are not asserted complete by foundation tests.
