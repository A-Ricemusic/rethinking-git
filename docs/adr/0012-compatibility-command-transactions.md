# ADR 0012: Recoverable command publication for the compatibility CLI

Status: accepted for the format-2 prototype; not canonical-store integration.

The CLI currently writes format-2 JSON records independently. A failed snapshot
or integration can therefore publish a pointer without its operation record, and
concurrent commands can overwrite each other's state.

All CLI commands now acquire a SQLite write lock before repository reads. JSON
updates are staged in memory. Successful commands commit their complete update
set to a separate SQLite journal with synchronous FULL, then publish individual
records through flushed temporary files and atomic rename. The lock remains held
through journal publication and clearing. The next command replays an interrupted
committed update before reading repository state. Ordinary errors discard staged
updates; an integration conflict intentionally commits conflict records while
returning its existing nonzero status. Immutable blobs are flushed before metadata
publication. Failed commands may leave unreferenced blobs.

The two SQLite files have a distinct application identifier and schema version;
neither is the frozen canonical store database. Existing format-2 repositories
acquire this journal automatically. Keep both files with the repository, and use
only transaction-aware CLI versions: older binaries and direct JSON editors do
not participate in locking. A live filesystem copy is not a consistent backup.

This is a compatibility bridge, not authenticated storage. It does not fabricate
signatures or alter the signed canonical object format. Canonical-store migration,
identity bootstrap, filesystem checkout transactions, and snapshot permissions
remain separate work. Atomicity applies to cooperating CLI commands; external
readers can see intermediate JSON publication. Windows process-interruption
recovery is covered by the same journal design, but power-loss directory durability
is not qualified there. On Unix, parent directories are flushed after rename.
Initialization before the first committed journal can leave an incomplete control
directory. A command can also print output before final publication fails; callers
must check its exit status. Same-principal adversarial filesystem races and network
filesystem locking are not supported guarantees.

Validation covers existing CLI behavior, rollback without publication, staged
read/list visibility, replay after partial publication, invalid recovery paths,
and concurrent commands observing their committed predecessor.

## Working-file publication extension (journal schema 2)

A second journal table stores each working path's expected previous bytes and
intended replacement (NULL denotes absence). The command validates every blob,
path and current file before committing both metadata and working updates. Recovery
accepts only the expected old or intended new bytes, applies working changes first,
then publishes metadata. Clearing both tables is one SQLite transaction. An edit
made after interruption stops recovery without removing the committed journal.
Symlink traversal, special files, untracked collisions, and unsupported path shapes
are refused. Snapshot file modes and symlinks require a later format extension.
Subprocess tests exercise exit after the first replacement and preservation of edits
made before recovery. Readers still need to participate in repository locking.
