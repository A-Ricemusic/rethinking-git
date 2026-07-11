# ADR 0008: Rust 1.85 MSRV and edition 2024

- Status: Accepted
- Date: 2026-07-11
- Owners: release engineering

## Context

An explicit minimum supported Rust version (MSRV) makes builds reproducible and gives
dependency updates a compatibility boundary. The current prototype uses edition 2021,
but the planned workspace is a new production baseline.

## Decision

The production workspace targets Rust 1.85.0 or newer and edition 2024. `rust-version`
is declared in workspace package metadata. CI tests exactly 1.85.0 plus current stable;
nightly is used only for optional lint/fuzz jobs and is never required for release.

MSRV may increase no more than once per minor release, requires an ADR/changelog entry,
and follows a policy of supporting at least the latest stable compiler available in
the oldest supported major OS/toolchain environment. Dependencies are selected and
pinned with MSRV compatibility; resolver version 3 is used.

## Consequences

- Edition 2024 and a modern stable standard library are available to the refactor.
- Existing `Cargo.toml` changes occur in Milestone 1, not as part of this decision-only
  milestone.
- Downstream builders receive a clear error instead of accidental compiler breakage.

## Rejected alternatives

- “Current stable only”: unnecessarily disruptive and not reproducible.
- Very old MSRV: constrains security dependencies and multiplatform maintenance before
  users exist.
- Nightly: unacceptable stability and supply-chain variance for production releases.

## Verification and open work

Add an MSRV CI lane with the committed lockfile, stable CI lane with minimal and locked
dependency resolution, and a release check that reads manifest metadata. Reassess the
support window after real distributor and enterprise feedback.
