# MiniCore Host storage

Object schema **1** and history schema **2**, implementing the persistence boundaries of SPEC v4.93.
The adapter supplies filesystem facts, not Host/Agent/Session business ownership.

## Supported platform and root ownership

The first implementation explicitly supports Linux. It needs a filesystem that
honors advisory inode locks, atomic same-directory rename and file/directory
`fsync`. Unsupported locking or synchronization fails rather than weakening the
contract. No claim of power-loss testing, network-filesystem validation or other
platform support is made.

`DataRoot::acquire(path)` resolves the actual root and takes a nonblocking OS
exclusive lock on the directory inode itself. Root symlink aliases resolve to the
same lock; no deletable lock-file marker or PID timeout decides ownership. A
second handle opened independently, including in the same process, is rejected.
Child storage handles and Directory clones retain the same root guard. The
final actual owner explicitly unlocks before closing its descriptor, including
initialization failures, so an unrelated forked child's inherited descriptor
cannot prolong the lease. A caller's departure or save confirmation does not
release live owner handles. Dropping the final handle is permitted only after
the owner has joined all execution and necessary save work. Abnormal exit
relies on the OS closing all copies of the locked description; leftover marker
files do not establish occupancy.

Below the established root, all operations are relative to open directory file
descriptors. Components are fixed filenames or strictly validated protocol IDs.
`O_NOFOLLOW` rejects links; stored files must be regular, owned by the current
user, single-linked and not writable by other users. Directories must also be
owned and not writable by other users. New directories are mode 0700 and files
0600, subject only to a more restrictive umask. Existing permissions are not
silently changed. This is not an isolation boundary against a malicious process
running as the same operating-system user. Root/path configuration is trusted;
tool workspaces are not managed by this adapter.

## Layout and object envelopes

The supplied directory is one Host root. Display names never affect paths.

| Path relative to root | Envelope |
| --- | --- |
| `host.json` | `{schema_version,id,data:{config,config_revision}}` |
| `agents/<AgentId>/agent.json` | `{schema_version,id,host_id,data:AgentData}` |
| `agents/<AgentId>/sessions/<SessionId>/session.json` | `{schema_version,id,agent_id,data:SessionData}` |
| `agents/<AgentId>/sessions/<SessionId>/history.jsonl` | One `HistoryRecord` per line |

`AgentData` contains name, mode, native `AgentConfig`, configuration revision and
nullable default Session ID. A saved default is checked against this Agent's
open persistent Session metadata and expected history file. Execution-ready
history validation remains with that Session. The business owner represents a
runtime memory default as null. `SessionData` contains mode, source Agent configuration revision,
configuration revision, native effective `SessionConfig`, optional historical
parent reference and optional committed `SessionClosure {reason}`. A committed
close cannot be reopened or rewritten. No incarnation, Runner, active Turn,
queue, pause, credential handle, waiter or delegation state is recoverable.

Host configuration is a caller-supplied serde type. The Host owns its validation
and resource resolution; storage verifies schema, identity and lossless decoding.
Agent and Session configurations reuse and validate protocol types. File ID and
direct-owner fields must agree with the selected hierarchy. Unsupported schemas,
unknown/discarded fields, duplicate JSON keys, content normalization and ownership
mismatch fail without migration or rewriting. `AgentStore::load_record()` validates
only the Agent record; initialization then loads each Session independently.
`list_sessions()` uses that same direct-owner boundary. Both Session and Agent
listing enumerate canonical direct-child names without opening those entries;
each child's load independently checks directory safety, schema and ownership.
A bad child directory is a fault for that child, not a failed enumeration for its
healthy siblings. Shared collection directories still require safe no-follow
access, and invalid names without a canonical object ID remain structural errors.
A broken default does not
block healthy siblings or silently redirect the default. The stricter `load()`
convenience method also checks the default reference. Changing configuration requires a
new configuration revision; Session creation provenance remains immutable.

`save_host`, `AgentStore::save` and `SessionStore::save` write a new exclusive
temporary file in the same directory, `sync_all` it, rename over the original,
then `sync_all` the containing directory and its owned ancestry. Root acquisition
confirms the ancestry needed to locate the root, including retries where a prior
failed synchronization left new directories visible. Existing empty initial
histories are synchronized again before Session publication. The returned `ObjectSave` binds the original configuration
revision and reports persisted only after these operations succeed. A failure
after rename may leave the new version visible: the business owner must check
that original version and must not infer rollback from the error. Temporary-file
cleanup never deletes an original object. Initial Session creation confirms its
empty history before publishing the Session record; this is not a multi-file
transaction and partial creation remains a real fact.

Metadata snapshots and atomic publication coordinate only on the exact file
path within this `DataRoot`. A snapshot retains its file-local read gate through
no-follow open, read and final file/directory checks; publication takes that
path's write gate for target validation, rename, synchronization and confirmation.
The shared gate index never holds a lock over filesystem work. A raw read releases
its gate before typed decoding or following parent/default references, so there
are no cross-object gate chains and unrelated metadata files remain independent.
Same-instance replacement therefore cannot invalidate a legitimate open snapshot.
External unlink, replacement, extra hardlinks, unsafe permissions or detached
directories observed during the read still fail; no `nlink == 0` exception,
automatic retry or repair is introduced. Append-only history does not use these
metadata gates and retains all its existing identity/link checks. A valid
snapshot is readable data, not a durability confirmation for a failed save.

Memory Agent/Session saves return not_applicable before filesystem work. A memory
Session journal performs no directory lookup, object creation or JSONL write.
Memory Agent plus persistent Session is rejected; modes never silently change.

## JSONL schema

Every record has exactly these outer fields:

- `schema_version`: 2 for newly written records. Exact original version-1 branches remain readable; the compaction branch requires version 2. Unsupported versions fail without rewriting.
- `session_id`: this Session's stable ID.
- `position`: zero-based contiguous Session-local position, not a public cursor.
- `entry`: `{kind,data}` with one of the branches below.

| kind | data |
| --- | --- |
| `input` | `turn_id,message_id,source,mode,in_reply_to,user` |
| `runtime` | `turn_id,response_index,item` |
| `partial` | `turn_id,loop_id,response_index,partial` |
| `turn_finished` | `turn_id,outcome,reason` |
| `summary` | `summary` |
| `compaction` | `source_position,covered_items,summary` (version 2 only) |

Input `user` is the exact Runtime `UserHistory`; text exists only in its native
`input`, alongside the message's consumed provenance. Queued-but-unconsumed
messages are never recorded. Runtime `item` is the native `HistoryItem`, but User
items must use `input` so their provenance cannot be omitted. Model responses and
tool results retain native types, IDs and payloads. `response_index` is the
zero-based logical response within the Host Turn, across internal Loops.
`partial` is authoritative Runtime `ModelPartial`, not an assistant response and
not a projection reconstructed from observation events. `loop_id` preserves its
internal association. Finish facts preserve actual outcome independently of save
confirmation. A standalone native summary does not imply prefix replacement.
An explicit compaction marker records its actual log position and advancing
cumulative coverage in original native items. Its cut must preserve tool-exchange
associations. Original JSONL and raw native history remain append-only; the latest
valid marker supplies only the model-context projection of summary plus uncovered
suffix. Partial, finish and compaction records do not count as native items.
Session ownership installs a generated projection only after its required save
confirmation; the storage helper alone does not implement manual/automatic work.

JSON object key order and insignificant whitespace are not significant. Optional
fields use the native serializers' representations: a field omitted for None
must be absent, whereas a field serialized as null remains null. Loading may
supply native defaults but never discard a supplied field or normalize supplied
content. Numeric tokens are parsed exactly before native decoding; if a reused
native codec cannot represent a number losslessly, loading explicitly fails
rather than rounding it. This deliberately rejects ambiguous or unsupported
hand-written forms.

The reader validates the **entire** file before returning any context. It checks
UTF-8/JSON, duplicate keys at every nesting level, strict native payload decoding,
schema, Session ownership, contiguous positions, logical response order,
Loop/Turn associations, terminal Loop partials, duplicate ToolCalls/results and
matching result names and request associations. Batch validation uses a small
overlay containing only newly touched indexes, not a clone of historical indexes.
An old call lacking a result is valid history and does not
reconstruct executable work. A final complete record without a newline is valid;
a new append inserts only the required separator, preserving the original bytes.
Blank lines, broken tails, unsupported records and corrupt middle lines fail with
a one-based line and zero-based byte offset when available. No tail truncation,
skipping, format conversion or automatic repair exists.

## Save ownership and exact failure reconciliation

All disk methods are synchronous. Invoke them in an owned blocking worker,
outside Host, Agent and Session state locks. A caller stopping its wait must not
drop the owner's accepted worker. A dedicated journal has one OS-locked writer;
it is not a Host-wide request lock. Independent Sessions can write independently.
The writer releases its history lease only when its actual owning worker drops
it, before releasing the retained root owners. Temporary shared readers and
explicit-deletion leases release on return, including error paths. None of these
lease releases performs synchronization or confirms a pending save range.
Each direct owner must serialize accepted saves of its own object outside its
business-state lock. File-local metadata coordination is not a compare-and-swap
for competing same-object revision updates or a multi-object transaction.

1. `accept(entries)` or `accept_batch(exact_records)` validates and takes a new
   range `[start,end)`. The exclusive accepted position advances immediately.
2. `persist_pending()` writes only that incremental range and synchronizes the
   file and necessary directory metadata. Only success advances persisted.
3. On any write/sync/identity failure, the accepted range and original bytes stay
   owned by the journal. New acceptance is blocked until this range is resolved.
   Neither readable data nor a caller-supplied flag implies persisted.
4. `reconcile_pending()` verifies the original prefix digest and exact file
   identity. A complete, identical suffix is synchronized without a second
   append. A proven absent suffix with identical original prefix can continue
   the original write. Partial, changed, replaced, truncated or extra data stays
   blocked and untouched. This never invokes Runtime or re-executes a tool.

The SHA-256 prefix state is a same-run reconciliation witness only, not an
on-disk integrity proof or security boundary. Successful incremental saves update
it without copying or encoding the old history. Before confirmation, writers
recheck the current directory, filename identity and exact file length after I/O.
Loading reads native history
once; `take_loaded_history()` moves that initial context to its Session owner.
Reopening performs a fresh explicit baseline sync before initializing the new
writer's confirmed position, but does not recover an old save operation or assert
that an old operation had reported persisted. File errors must block the affected
Session in its business owner; storage does not alter the actual Turn outcome.

No TTL, retention budget, queue cap, record limit, body limit or application-level
capacity admission is introduced by storage. Native Runtime type validation
remains authoritative.

## Explicit deletion

No destructor, stop, unload or close deletes persistent data. `delete_closed`
removes only a validated closed Session's two known files and empty directory,
and fails while a journal writer holds the file lock. Unexpected files block
deletion. `delete_empty` removes only the Agent record after its Sessions directory
is empty; its caller must already have stopped that Agent. Neither operation
recurses into descendants, workspaces, other recipients' histories or artifacts.
Deletion is not transactional: if a necessary sync fails after unlink, the error
does not undo that unlink. An explicit retry can continue when the original
closed ownership record remains. If that record is already removed but directory
cleanup failed, this convenience API does not reconstruct deletion authority or
claim completion; the business owner must retain and resolve the accepted
operation's partial facts. The public owner-level deletion lifecycle and receipts
are implemented in `host-core`; these storage conveniences do not own that work.

## Verification

The tests use real temporary directories and files. They cover intra-process and
cross-process locks, aliases and abnormal process exit; schema/ownership/path
errors; exact native histories and partials; no-file memory modes; complete tails
without newline; strict corruption and duplicate detection; confirmed ranges and
reopening; and explicit deletion boundaries. Linux subprocesses with a test-only
file-size rlimit produce real `EFBIG` errors for atomic object and partial JSONL
writes. Separate, explicitly named test hooks inject failed sync/zero-write/partial
write outcomes around real files to verify precise same-run reconciliation.
