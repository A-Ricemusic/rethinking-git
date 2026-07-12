# RGit schema-0 registry

This crate is an additive implementation of `spec/objects.md` and
`spec/canonical-encoding.md`. It does not read or rewrite the prototype CLI's JSON
repository format.

The schema-0 registry below is frozen. Incompatible changes require a new schema
version and migration; numeric values MUST NOT be renumbered or reused.

Operation schema 1 is the sole schema-1 assignment. Every other logical kind remains
at schema 0. Implementations dispatch support by `(kind, schema)` rather than assuming
that all kinds share one current schema.

Object-kind registry: chunk 1, blob 2, secret reference 3, manifest 4, subproject 5,
snapshot 6, change revision 7, line state 8, conflict 9, operation 10, marker 11,
release 12, policy 13, repository root 14, identity 15, group membership 16,
key-envelope set 17, change relation 18, conflict resolution 19, review evidence 20,
approval evidence 21, CI evidence 22, policy-decision evidence 23, projection rules 24,
projection proof 25, build provenance 26, artifact 27, operation payload 28, view 29,
migration 30, and ruleset 31. `StorageEnvelope` is physical storage metadata and
MUST NOT be assigned a logical kind. Each schema uses common field 0
for kind, 1 for schema version, and 2 for the exact policy reference. Remaining
numeric assignments are the field numbers emitted in `src/object.rs` and accepted
by the closed decoder in `src/decode.rs`.

Schema 0 has two frozen resource profiles. Metadata is limited to a 1 MiB encoded
object, a 256 KiB byte string, a 64 KiB text string, 65,536 items per array/map, and
64 nested container levels. Chunk and Blob are limited to a 16 MiB encoded object, a
4 MiB byte string, a 64 KiB text string, 1,000,000 items per array/map, and 64 nested
container levels. Limits are inclusive. Storage may set stricter limits but may not
relax these schema ceilings. The rationale and compatibility rules are normative in
`spec/canonical-encoding.md`.

A chunk payload and a chunk profile's declared maximum are limited to 4 MiB. A chunk
is policy-bound, preventing cross-policy equality oracles.
Chunked blobs record algorithm, version, minimum, target, and maximum sizes. Schema
0 freezes algorithm 0 as FastCDC and its parameter profile version as 0. Unknown
algorithm or profile values are rejected rather than treated as opaque extensions.
Profile 0 is exactly 256 KiB minimum, 1 MiB target, and 4 MiB maximum, with
normalization level 1, gear seed `0x7267697466636463`, early mask `(1<<21)-1`, and
late mask `(1<<19)-1`. The SplitMix64 gear derivation and byte-exact boundary state
machine are normative in ADR 0004 and pinned by `tests/vectors/fastcdc-v0.json`.
Chunk references have lengths in `1..=4 MiB`, stay in stream order, and must sum
exactly to the Blob length. Empty files are inline empty Blobs, never zero-length
ChunkRefs. Bytes of length 0 through 65,536 MUST be inline; length 65,537 and above
MUST be chunked. Representation and all boundaries are independent
of caller read-buffer segmentation.

Line-state generation zero is the only genesis representation and must omit a
previous state. All later generations require one. A line-advance operation embeds
the complete intended state declaration but never the new line-state ID; the signed
line state subsequently points to the finalized operation. Generic transitions may
refer to an old line state as `before` but may not install one as `after`.

## Operation schema 1

Operation schema 1 preserves schema-0 top-level fields 0 through 13, changes common
field 1 to `1`, and replaces unbound generic action 0 with bound-transition action 2.
Schema-1 decoders reject action 0. Line-advance action 1 is unchanged.

A bound transition is `{0: 2, 1: reference_key, 2?: before, 3: after}`. `before` and
`after` are typed object references; `after` is required. The reference key is
`{0: key_kind, 1?: stable_id}` using the closed registry line 1, change 2, operation
head 3, release 4, and marker 5. Stable ID is exactly 16 bytes and required for kinds
1, 2, 4, and 5; it is omitted for operation head. Bound-transition accepts only
change, release, and marker keys. Line updates use line-advance, whose declaration
already binds the exact LineId, and operation-head updates use the Operation's own ID
and parents. Before and after kinds must equal the key's required target kind. One
Operation may contain at most one action for any exact reference key.

Schema-0 Operation bytes, signatures, IDs, and semantics remain frozen and readable.
Schema 1 uses the same metadata ceiling and signature profile, but its schema number
is present in both the canonical map and signature/object-ID domain preimages.
Operation DAG parent edges may cross schema versions.

Schema objects carry fixed profile-0 signature records. Algorithm 0 is Ed25519; key
IDs are exactly 32 bytes, signature bytes exactly 64 bytes, and purposes are numeric
registry values 0 (line state), 1 (operation), 2 (marker), 3 (release), and 4
(policy). Multi-signature arrays are nonempty, canonically sorted, and unique. The
unsigned projection omits only field 13, 12, 8, 16, or 15 respectively. The exact
`RGIT-SIGNATURE\0` preimage grammar is frozen in `spec/canonical-encoding.md`.
Signature verification belongs to the crypto layer; this crate deliberately has no
private-key signing API and accepts no placeholder or unsigned production object.
Genesis additionally requires an external trust anchor. The linked bootstrap graph
is Identity/KeyEnvelopeSet -> root Policy -> genesis Operation -> genesis LineState
-> RepositoryRoot; bootstrap omissions are allowed only at the explicitly documented
zero versions, so no logical object ID cycle is required.

The registry slice 14--31 is frozen. Fields after the common header are:

| Kind | Fields (numeric key = meaning) |
| --- | --- |
| 14 RepositoryRoot | 3 repository ID; 4 root Policy; 5 sorted Identity IDs; 6 bootstrap KeyEnvelopeSet; 7 genesis Operation; 8 sorted initial LineStates; 9 filesystem profile; 10 signatures |
| 15 Identity | 3 subject kind; 4 subject ID; 5 version; 6 previous; 7 signing keys; 8 encryption keys; 9 issuer; 10 status; 11 activation Operation; 12 not-after Operation; 13 signatures |
| 16 GroupMembership | 3 membership ID; 4 group ID; 5 version; 6 previous; 7 principal; 8 state; 9 issuer; 10 activation Operation; 11 not-after Operation; 12 signatures |
| 17 KeyEnvelopeSet | 3 epoch; 4 suite; 5 sorted recipient envelopes |
| 18 ChangeRelation | 3 relation kind; 4 sorted sources; 5 sorted results; 6 sorted provenance; 7 creating Operation |
| 19 ConflictResolution | 3 Conflict; 4 typed result; 5 resolver; 6 device; 7 resolution kind; 8 sorted typed provenance; 9 wall time; 10 signatures |
| 20--23 evidence | 3 typed target; 4 Snapshot; 5 Ruleset; 6 issuer; 7 device; 8 outcome; 9 constraints; 10 sorted related refs; 11 wall time; 12 signatures. CI adds 13 check name, 14 runner Identity, optional 15 BuildProvenance. |
| 24 ProjectionRules | 3 version; 4 previous; 5 ordered registered rules; 6 default-fail |
| 25 ProjectionProof | 3 algorithm; 4 source Snapshot; 5 ProjectionRules; 6 audience Policy; 7 projected Manifest; 8 proof |
| 26 BuildProvenance | 3 Snapshot; 4 Ruleset; 5 builder Identity; 6 sorted typed inputs; 7 sorted Artifact outputs; 8 reproducibility metadata; 9 wall time; 10 signatures |
| 27 Artifact | 3 kind; 4 digest algorithm; 5 digest; 6 byte length; optional 7 locator; optional 8 Blob |
| 28 OperationPayload | 3 payload kind; 4 sorted typed refs; 5 payload schema; 6 canonical payload |
| 29 View | 3 actor; 4 device; 5 sorted Policies; 6 sorted line/generation/state records; 7 projected Manifest; 8 validity constraints; 9 signatures |
| 30 Migration | 3 source format; 4 target format; 5 sorted one-to-one mappings; 6 tool Identity; 7 wall time; 8 signatures |
| 31 Ruleset | 3 ruleset kind; 4 version; 5 previous; 6 constraints; 7 sorted required evidence kinds |

Closed subregistries are: filesystem profile portable 0/native 1; identity subject
actor 0/device 1; identity status active 0/suspended 1/revoked 2; membership active
0/removed 1; change relation split 0/combine 1/supersede 2; resolution left 0/right
1/base 2/manual 3/driver 4; evidence pass 0/fail 1/abstain 2/error 3; operation payload
inverse 0/recovery 1/public-redaction 2/private-audit 3; and ruleset review 0/approval
1/CI 2/landing 3/release 4. Public-key algorithms are Ed25519 0 and X25519 1;
signing and encryption key arrays respectively accept only those algorithms, and key
IDs are unique across both arrays. Ed25519 and X25519 public keys are exactly 32
bytes; nonzero key IDs are independently exactly 32 bytes. Key-envelope suite 0 is
X25519-HKDF-SHA256-AES256-GCM. Projection-rule kinds are include 0/exclude 1/redact
2; projection-proof algorithm 0 is Merkle-v0. Artifact kinds are binary 0/source
archive 1/SBOM 2/attestation 3, and artifact digest algorithms are SHA-256 0/BLAKE3-256
1. Operation-payload schema 0 is a canonical CBOR map. Migration ID formats are v0
0 and reserved migration target v1 1. Existing closed namespaces are conflict content
0/add-add 1/modify-delete 2/type-change 3 and policy derivation no-derivation 0/
same-policy 1/explicit-evidence 2. Signature purposes append repository root 5, identity 6,
membership 7, conflict resolution 8, review 9, approval 10, CI 11, policy decision
12, build provenance 13, view 14, and migration 15. Unsigned projections omit only
the signature field named above.

Evidence constraints, projection-rule parameters, build reproducibility metadata,
operation payloads, view validity constraints, and ruleset constraints are registered
schema-0 canonical CBOR maps. Noncanonical bytes, non-map values, unknown numeric
registries, and unregistered opaque extension fields fail closed.

Portable manifests and paths reject Windows reserved names (including the documented
superscript-digit COM/LPT aliases), Windows-illegal ASCII characters, ASCII controls
U+0000--U+001F, DEL U+007F, and trailing spaces or dots. Device-name matching is
performed after the profile's pinned case fold. Rejecting DEL, although it is not a
Windows-illegal character, prevents an invisible terminal control character from
entering the cross-platform namespace. Sibling collision keys use Unicode Default
Case Folding, full and non-Turkic, followed by NFC normalization. Collision scope is
one manifest only, so equal segment names at different directory levels remain valid.
The implementation pins `unicode-casefold` 0.2.0 and its Unicode 9.0.0 dataset;
vectors include folds that differ from lowercase (for example `Straße`/`STRASSE` and
Greek final sigma). Changing the dataset is a format change and requires new
compatibility vectors and an accepted format decision.

Schema 0 freezes the portable component limit at 255 canonical UTF-8 bytes and the
slash-joined materialized relative-path limit at 1,023 UTF-8 bytes. Separators count;
a leading separator and trailing NUL do not. The empty segment array is the root and
has length zero. UTF-8 byte accounting makes acceptance platform-independent and is
conservative for NTFS/Win32 because valid Unicode has no more UTF-16 code units than
UTF-8 bytes. The total limit fits the smallest Tier-1 POSIX `PATH_MAX`; Windows
materializers must additionally use handle-relative traversal or extended-length
paths so the checkout-root prefix does not consume the repository-relative budget.
