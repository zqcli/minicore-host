# Standard local tools

Workspace-bound `read`, `edit`, `write`, and Unix `bash` implementations of the
real MiniCore Runtime 0.10.0 `Tool` interface. No UI or Host object is required.
The immutable Workspace, a narrow, trusted Turn-bound `LocalToolSink` and a
name-only `CommandEnvironment` are constructor capabilities. Tool JSON contains
business arguments only. Standalone callers explicitly choose their policy;
`InheritCommandEnvironment` retains ordinary inheritance without exclusions.

This crate provides execution and registration declarations. Production Host
registry selection, Session persistence, public output/change projections, and
finalization wiring live in the core/tools/resources integration, not this crate.

The Turn owner must retain `LocalTools` and await `finish()` after Runtime ends,
including cancellation and deadlines. Runtime 0.10.0 drops port futures when
stopped; these tools retain actual workers and cleanup independently. `finish`
closes admission and joins them. Dropping a finish waiter does not cancel cleanup.
Do not declare the Turn finished merely because Runtime joined or kill was sent.
Each completion separates its tool error from `cleanup_confirmed`: a record sink
failure after actual child/worker join is confirmed cleanup with an internal
error, while a worker panic, failed group wait, unconfirmed process-group end,
or unfinished pipe cleanup is unconfirmed. The integrating owner must keep
unconfirmed work blocked in finalization rather than treating a returned
diagnostic as proof of exit.

The sink's per-call `complete` callback reports the same actual cleanup fact after
all I/O and observations, even when Runtime has dropped its waiting future. A
pre-worker rejection reports confirmed completion immediately. The production
Session adapter uses this fact to close the invocation's output lease and expose
its retained native result as finished. It does not wait for the whole Turn,
synthesize a ToolResult, or replace the existing once-per-Turn collector.

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
stdin. It inherits the process environment except names supplied by its
`CommandEnvironment` immediately before launch. Production Host bindings exclude
all configured model API-key and optional network access-key source names,
including formerly published names for that Host lifetime. Successful reloads
tighten this policy for later commands even in already-bound Turns, without
replacing their models, tools or workspace. Candidate preparation, rejection and
failed saves do not publish exclusions. No process-global environment or actual
tool output is changed. Already-started children are not retroactively altered;
this is not a same-user process, filesystem or network sandbox. Runtime policy
supplies permission, never Role text. Unix process groups
are stopped on cancellation/deadline and remaining group descendants are stopped
after leader exit, including when they redirected both output pipes. The real
process wrapper wait is always joined without dropping it mid-flight, while both
pipes are drained. A separate no-signal group probe must observe ESRCH before
cleanup is confirmed; a successful kill request, direct-child exit, or pipe EOF
alone is insufficient. Scope observation allows ten 20 ms intervals before
reporting unconfirmed cleanup, never an invented successful termination. Pipes
are read to EOF, or explicitly marked incomplete if an escaped descendant holds
them after one second of cleanup. Deliberately escaped process groups are outside
this local-host scope; no general process supervisor or Windows job system is claimed.
Full output is retained; non-UTF-8 output uses exact byte encoding. There is no
automatic retry of commands or file writes.

Path checks reject traversal, writes through symlinks, nonregular targets, and
`.git` metadata edits. In-workspace symlinks are supported for reads. Workspace
clones share a dedicated mutation operation guard across all their LocalTools
bindings, from source capture through replacement and authoritative after capture.
This guard belongs to the actual blocking worker, not its caller's wait. It is
not an owner-state lock. Separately opening the same path creates a different
binding and does not share this guard. The production resource builder reuses
actual Workspace bindings through its owner-held weak map across Turns and
resource publications, without retaining unused roots. Shell writes, external
path/permission races and cross-process locking are outside this boundary.
Parent-directory creation is not an atomic multi-directory transaction.
`apply_patch` and other tools are absent.
