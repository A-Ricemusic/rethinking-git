# ADR 0009: Supported operating systems and filesystem profile

- Status: Accepted
- Date: 2026-07-11
- Owners: workspace, storage, and release engineering

## Context

Version control crosses path encoding, case sensitivity, Unicode normalization,
symlinks, executable bits, atomic rename, locking, filesystem watchers, and durability.
Claiming generic “cross-platform” support without exact tiers makes corruption and
non-materializable snapshots likely.

## Decision

Tier 1 production targets are:

- x86_64/aarch64 Linux on maintained distributions, with local ext4, XFS, or Btrfs;
- x86_64/aarch64 macOS versions receiving Apple security updates, with local APFS;
- x86_64 Windows 11 and supported Windows Server, with local NTFS.

Every release runs unit/integration and repository crash/recovery coverage on Linux,
macOS, and Windows. Architectures without CI artifacts are Tier 2 best-effort.

Repositories default to the portable path profile in the object specification: UTF-8
NFC segments, no traversal/separators/NUL, case-fold collision rejection, Windows
reserved-name/trailing-dot-space rejection, and bounded component/path lengths.
Canonical objects track only regular files, executable intent, safe symlinks,
directories, subprojects, and secret references. Windows stores executable intent as
metadata. Symlink materialization on Windows requires capability/support or produces
a typed non-materialized entry; it never silently becomes an ordinary file.

Repositories and SQLite databases must be on one local filesystem for atomic publish.
NFS/SMB, FUSE providers, cloud-synchronized folders, FAT/exFAT, removable media, and
case-sensitive APFS/NTFS variants are unsupported for production in 1.0 unless added
by a tested filesystem capability profile. Startup probes required atomic rename,
locking, case behavior, symlink behavior, and durability assumptions and fails safely.

## Consequences

- Most repositories materialize consistently across developer platforms at the cost
  of rejecting names accepted by an individual host.
- Watchers (inotify/FSEvents/ReadDirectoryChangesW) are hints only; every command
  reconciles filesystem state because events can be missed or reordered.
- Durability implementations remain platform-specific: file flush, directory flush,
  rename semantics, and antivirus/indexer interference require tests and diagnostics.
- File ownership, ACLs, xattrs, forks, devices, sockets, and arbitrary mode bits are
  not portable schema 0 data.

## Rejected alternatives

- Preserve arbitrary host-native paths: creates inaccessible/colliding snapshots.
- Lowest-common-denominator bytes only: poor Windows/macOS UX and Unicode ambiguity.
- Network filesystems by default: locking/rename/durability semantics vary too widely.

## Verification and open work

Build path corpus and materialization round-trip tests on every Tier 1 target; test
case/normalization collisions, long paths, safe/escaping symlinks, permission failures,
watcher loss, concurrent processes, crash publication, and disk-full recovery. Set
precise supported OS version windows and path length limits in the release policy.
