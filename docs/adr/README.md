# Architecture Decision Records

ADRs capture decisions whose reversal would affect formats, security, portability, or
several crates. `Accepted` means the project uses the decision as its implementation
baseline; it does not imply code exists or external review is complete.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-canonical-object-encoding.md) | Deterministic CBOR canonical objects | Accepted |
| [0002](0002-hash-algorithm-agility.md) | BLAKE3-256 default with versioned algorithm IDs | Accepted |
| [0003](0003-embedded-metadata-database.md) | SQLite metadata/index database | Accepted |
| [0004](0004-immutable-packs-and-chunks.md) | Immutable packs and content-defined chunks | Accepted |
| [0005](0005-cryptographic-libraries.md) | Audited Rust libraries and fixed suite registry | Accepted |
| [0006](0006-async-runtime.md) | Tokio for network/service async only | Accepted |
| [0007](0007-rpc-and-transport.md) | gRPC over HTTP/2 with rustls | Accepted |
| [0008](0008-minimum-supported-rust-version.md) | Rust 1.85 MSRV and edition 2024 | Accepted |
| [0009](0009-supported-platforms-and-filesystems.md) | Linux/macOS/Windows portable filesystem profile | Accepted |
| [0010](0010-isolated-overlay-workspace-sessions.md) | Isolated per-session overlay roots with portable managed fallback | Proposed |
| [0011](0011-key-bound-operation-transitions.md) | Key-bound mutable-reference transitions | Accepted |

New ADRs use the next four-digit number. Do not rewrite an accepted decision's history;
supersede it and link both records. Every record states consequences, verification,
and unresolved follow-up work.
