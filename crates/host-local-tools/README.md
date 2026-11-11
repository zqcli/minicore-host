# Standard local tools

Workspace-bound `read`, `edit`, `write`, and native `bash` implementations of the
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
error, while a worker panic, failed scope wait, unconfirmed process-scope end,
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
Windows file replacement flushes a read/write directory handle and requires a
local NTFS volume whose serial number matches the held directory. Directory
reparse points and mismatched file identities fail explicitly. Other Windows
filesystems are unsupported; sync failures after replacement retain
`write_outcome: applied` together with the actual before/after capture, never
reporting a successful durable write or pretending to roll it back.
Before writing replacement content, Windows copies the existing file's complete
effective DACL, its protected/unprotected inheritance setting, and the read-only
attribute to the empty temporary file. It reads the copied DACL back before body
writes; a changed ACE sequence, protection setting, different source owner, or
permission-copy failure rejects the operation before publication. Newly created
files retain the directory's normal inheritance rules. Unix retains the existing
permission-mode preservation.

Bash executes `/bin/bash` on Linux/macOS and an installed Git Bash on Windows,
with `--noprofile --norc -c`, child-local cwd and null stdin. Windows binds an
absolute MSYS Bash executable from absolute PATH entries or standard Git install
locations and adds its own utilities directory to the child PATH. It rejects the
Windows WSL launcher and fails explicitly when Git Bash is unavailable.
It inherits the process environment except names supplied by its
`CommandEnvironment` immediately before launch. Production Host bindings exclude
all configured model API-key and optional network access-key source names,
including formerly published names for that Host lifetime. Successful reloads
tighten this policy for later commands even in already-bound Turns, without
replacing their models, tools or workspace. Candidate preparation, rejection and
failed saves do not publish exclusions. No process-global environment or actual
tool output is changed. Already-started children are not retroactively altered;
this is not a same-user process, filesystem or network sandbox. Runtime policy
supplies permission, never Role text. Unix process groups and Windows Job Objects
are stopped on cancellation/deadline and remaining descendants are stopped after
leader exit, including when they redirected both output pipes. Windows attaches
the process to its job before it begins executing. The real
process wrapper wait is always joined without dropping it mid-flight, while both
pipes are drained. On Unix, a separate no-signal group probe must observe ESRCH
before cleanup is confirmed; scope observation allows ten 20 ms intervals before
reporting unconfirmed cleanup. On Windows, the uninterrupted wrapper wait must
query the job and observe `ActiveProcesses == 0`; a completion-port notification
only wakes that query. The wrapper is pinned to the upstream correction for
earlier versions that mistook any job notification for completion. A successful kill request,
direct-child exit, or pipe EOF alone is insufficient. Pipes
are read to EOF, or explicitly marked incomplete if an escaped descendant holds
them after one second of cleanup. Windows output uses owned asynchronous named
pipe readers, so an incomplete drain cannot leave a detached blocking reader.
Deliberately escaped Unix process groups are outside this local-host scope;
this is not a general process supervisor.
Full output is retained; non-UTF-8 output uses exact byte encoding. There is no
automatic retry of commands or file writes.

Path checks reject traversal, writes through symlinks or Windows reparse points,
nonregular targets, and `.git` metadata edits (including case aliases on Windows
and macOS). Windows rejects alternate streams and trailing-dot/space aliases.
In-workspace symlinks are supported for reads. Workspace
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
