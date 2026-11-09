# Standard local tools

Workspace-bound `read`, `edit`, `write`, and Unix `bash` implementations of the
real MiniCore Runtime 0.10.0 `Tool` interface. No UI or Host object is required.
The immutable Workspace and a narrow, trusted Turn-bound `LocalToolSink` are
constructor capabilities. Tool JSON contains business arguments only.

This crate provides execution and registration declarations. Production Host
registry selection, Session persistence, public output/change projections, and
finalization wiring are separate integration work, not claimed here.

The Turn owner must retain `LocalTools` and await `finish()` after Runtime ends,
including cancellation and deadlines. Runtime 0.10.0 drops port futures when
stopped; these tools retain actual workers and cleanup independently. `finish`
closes admission and joins them. Dropping a finish waiter does not cancel cleanup.
Do not declare the Turn finished merely because Runtime joined or kill was sent.

The recording sink is a short, nonblocking owner callback, not an observer or an
external store call. Its owner supplies Session/Turn/Load identity, validates
that identity, and must persist records through its normal ownership boundary.
Sink failures are internal tool errors, not shell exit codes. `NoopLocalToolSink`
is explicitly for standalone execution without retained Session records.

Read requires an explicit positive line/entry limit and has no hidden line or
byte quota. Edit matches literal text without newline normalization. Filesystem
business failures are structured results; successful writes use same-directory
atomic replacement and preserve existing permissions. The snapshots are actual
before/after file captures, including failed edits. Unknown capture is not empty
content. Shell changes remain unknown because arbitrary scripts have no known
target, and Git dirty state cannot attribute changes to this execution.

Bash executes `/bin/bash --noprofile --norc -c` with child-local cwd and null
stdin. It inherits the process environment; it is not a filesystem or network
sandbox. Runtime policy supplies permission, never Role text. Unix process groups
are stopped on cancellation/deadline and remaining group descendants are stopped
after leader exit. The direct child is always actually waited. Pipes are read to
EOF, or explicitly marked incomplete if an escaped descendant holds them after
one second of cleanup. Deliberately escaped process groups are outside this
local-host scope; no general process supervisor or Windows job system is claimed.
Full output is retained; non-UTF-8 output uses exact byte encoding. There is no
automatic retry of commands or file writes.

Path checks reject traversal, writes through symlinks, nonregular targets, and
`.git` metadata edits. In-workspace symlinks are supported for reads. Same-binding
writes are serialized; external hostile path/permission races and cross-process
locking are outside this local-host foundation. Parent-directory creation is not
an atomic multi-directory transaction. `apply_patch` and other tools are absent.
