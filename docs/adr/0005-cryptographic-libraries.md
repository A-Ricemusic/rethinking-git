# ADR 0005: Audited Rust cryptographic libraries and fixed suites

- Status: Accepted
- Date: 2026-07-11
- Owners: crypto and security

## Context

RGit needs signatures, authenticated encryption, device encryption/key wrapping,
password-based recovery, TLS, secure randomness, and secret-memory hygiene. Inventing
cryptography or permitting arbitrary algorithm negotiation is unacceptable.

## Decision

Use maintained, audited Rust libraries with stable constant-time implementations:

- `blake3` and RustCrypto `sha2` for registered hashes;
- `ed25519-dalek` for actor/device/object signatures;
- `x25519-dalek` plus a reviewed HPKE implementation conforming to RFC 9180 for
  recipient key envelopes;
- RustCrypto `chacha20poly1305` with XChaCha20-Poly1305 for object storage AEAD;
- RustCrypto `aes-gcm` only for an explicitly enabled AES-256-GCM compliance suite;
- RustCrypto `argon2` using Argon2id for passphrase-protected recovery material;
- `rustls` with an approved provider for transport; `getrandom` for OS entropy;
- `zeroize` and `secrecy` for best-effort secret-memory handling.

Suite identifiers, key purposes, nonce construction, authenticated context, and
minimum parameters are centrally registered. No CLI or peer chooses individual
primitives. Signing, encryption, release, service, and recovery keys are distinct.
Private keys live in OS/hardware key stores where supported, not repository objects.

## Consequences

- Safe APIs and limited suites reduce misuse, while several dependencies remain
  security-critical and require pinning, audit, and update ownership.
- XChaCha20's large nonce supports random per-envelope nonces; uniqueness is still
  tested and authenticated context binds repository/object/policy/epoch/suite.
- FIPS claims require a separately validated provider/profile and are not implied by
  AES/SHA selection.
- Zeroization cannot guarantee removal from allocator copies, swap, editors, or disks.

## Rejected alternatives

- OpenSSL as universal primitive API: platform variance and broader unsafe surface.
- Ring-only design: narrower algorithm/key-management flexibility.
- Custom crypto or user-configured cipher lists: excessive misuse/downgrade risk.
- Encrypting secrets directly by default: a dedicated secret manager remains safer.

## Verification and open work

Require known-answer/cross-implementation tests, nonce/context mutation tests, key
rotation/recovery exercises, dependency review, and external audit. The concrete HPKE
crate and OS key-store abstraction need focused evaluation before implementation.
Post-quantum migration requires a later ADR; the suite registry preserves a path.
