# ADR 0003: SQLite for transactional metadata and indexes

- Status: Accepted
- Date: 2026-07-11
- Owners: storage and reliability

## Context

Mutable references, operation publication, reachability indexes, leases, and recovery
need atomic transactions and mature crash behavior on all supported operating systems.
Bulk immutable content belongs in files/packs rather than database blobs.

## Decision

Use SQLite through `rusqlite`, with a project-pinned bundled SQLite build for consistent
features and security updates. Store metadata, reference state, pack/chunk locations,
and rebuildable indexes in SQLite. Store canonical object/envelope bytes in immutable
loose files and packs.

Use WAL mode for local fixed-disk repositories, foreign keys, strict tables where
available, `synchronous=FULL` for reference publication, explicit busy timeouts, and
one bounded writer coordinator. Publish object files durably before committing rows
or references that expose them. Repository locking and fsync-directory behavior are
platform-specific tested code, not assumptions delegated to SQLite.

## Consequences

- SQLite brings proven transactions, inspection/recovery tooling, and broad platform
  support at the cost of C code in the supply chain.
- Indexes can be rebuilt; the signed immutable graph and reference checkpoints remain
  authoritative.
- Database files must remain local. Network/cloud-synced folders are unsupported.
- Schema migrations are transactional, monotonic, backed up, and tested from every
  supported repository format.

## Rejected alternatives

- redb/sled: appealing Rust implementations, but less operational history and tooling
  for this security- and crash-critical baseline.
- RocksDB: heavy build/runtime footprint and mismatch with relational transactions.
- Flat JSON/files only: insufficient atomicity, query performance, and migration safety.

## Verification and open work

Fault-inject every publication boundary, process kill, disk-full, corrupt index, WAL
recovery, concurrent writer, and migration. Benchmark WAL/checkpoint policy at Linux
and monorepo scale. Reconsider a pure-Rust database only through a superseding ADR
with equivalent recovery evidence.
