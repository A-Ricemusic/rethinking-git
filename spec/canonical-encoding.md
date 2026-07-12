# Canonical Encoding and Object Identity, Version 0

Status: initial normative specification

Decision record: [ADR 0001](../docs/adr/0001-canonical-object-encoding.md)

## Encoding profile

RGit canonical objects use deterministic CBOR as defined by RFC 8949 section 4.2,
with the stricter profile below. Debug JSON is diagnostic only and MUST never be
hashed, signed, or accepted as a canonical object.

- Definite-length arrays, maps, byte strings, and text strings are required.
- Maps use unsigned integer field keys and deterministic bytewise key ordering.
- Duplicate map keys are invalid.
- Integers use the shortest form. Schema 0 supports only `u64` and `i64` ranges.
- Floating-point values, CBOR tags, simple values other than `false`, `true`, and
  `null`, and indefinite-length items are forbidden.
- Text is valid UTF-8 normalized to Unicode NFC before encoding. Security-sensitive
  opaque values use byte strings, not text.
- Optional absent values are omitted unless a schema explicitly distinguishes null
  from absence. Encoders MUST NOT emit unknown fields.
- Arrays preserve schema-defined order. Sets are encoded as arrays sorted by the
  canonical encoding of each element, with duplicates rejected.
- Decoders MUST reject a valid-but-non-canonical representation rather than silently
  normalizing it.

## Hash preimage

The byte sequence hashed for an object is:

```text
"RGIT-OBJECT\0" || uvarint(kind) || uvarint(schema_version) || canonical_cbor
```

The byte sequence signed under frozen signature profile 0 is exactly:

```text
"RGIT-SIGNATURE\0" ||
uvarint(signature_profile = 0) ||
uvarint(signature_algorithm) ||
uvarint(signature_purpose) ||
signer_actor_id[16] ||
uvarint(signing_key_id_length = 32) || signing_key_id[32] ||
uvarint(object_kind) || uvarint(schema_version) ||
uvarint(unsigned_cbor_length) || unsigned_cbor
```

`unsigned_cbor` is the kind-specific canonical object map with only its assigned
signature field omitted. Every other field, including the common header, remains.
For a multi-signature object, every signature covers the same unsigned projection;
the signature record itself supplies the algorithm, purpose, signer, and key ID bound
by its preimage. The final object ID covers the complete object including signatures.

Kinds, algorithms, and purposes are numeric closed registry values, not user strings.
Unsigned LEB128 is used for every shown varint and length and MUST use its shortest
encoding. The domain separator includes its trailing NUL byte. Implementations MAY
stream these bytes. Before constructing an ID or signing preimage from untrusted
canonical bytes, a decoder MUST verify that the payload declares the supplied kind
and version. Typed schema encoders establish that correspondence by construction.
Profile 0 assigns signing algorithm 0 to Ed25519 and purposes 0 through 4 to line
state, operation, marker, release, and policy respectively.

Profile 0 structurally treats an Ed25519 signing-key ID as an opaque, nonzero 32-byte
value. The crypto milestone will freeze its derivation from public-key material and
perform actual Ed25519 verification. Schema-0 vectors in this milestone use fixed
nonzero record bytes only to pin structure and preimage construction; they are not
proof that a signature or key binding has been cryptographically verified.

## Object ID binary and text form

The binary form is:

```text
uvarint(id_format = 0) || uvarint(hash_code) || uvarint(digest_length) || digest
```

Hash code `0x1e` means BLAKE3-256 and `0x12` means SHA-256. Schema 0 digests are 32
bytes. The text form is lowercase, unpadded base32hex prefixed with `rg0_`. Parsers
MAY accept uppercase input but MUST render lowercase. UI abbreviations are never
accepted in durable records or protocol messages and must be proven unambiguous in
the current authorized view.

## Verification rules

Decoders enforce nesting, allocation, and collection limits before allocation. The
following schema-0 admission profiles are frozen. A limit is inclusive; a value at
the listed maximum is accepted. “Text string” is measured in UTF-8 bytes, and
“collection items” applies independently to each array or map.

| Maximum | Metadata objects | Chunk and Blob objects |
| --- | ---: | ---: |
| Encoded canonical object | 1,048,576 bytes (1 MiB) | 16,777,216 bytes (16 MiB) |
| Individual byte string | 262,144 bytes (256 KiB) | 4,194,304 bytes (4 MiB) |
| Individual text string | 65,536 bytes (64 KiB) | 65,536 bytes (64 KiB) |
| Array or map items | 65,536 | 1,000,000 |
| Nested container depth | 64 | 64 |

Metadata uses the smaller profile because graph objects are routinely decoded while
walking attacker-influenced DAGs; the per-string and collection caps bound allocation
amplification within the 1 MiB envelope. Chunk and Blob use the bulk profile. A Chunk
payload and a chunk profile's declared maximum are each limited to 4 MiB, matching
the versioned content-defined chunking ceiling. The 16 MiB encoded envelope is mainly
for Blob chunk-reference arrays; it does not permit a larger individual payload.

Implementations MAY impose stricter operational limits, in which case objects above
those limits are unavailable in that deployment. They MUST NOT admit a schema-0
object above these ceilings. Changing a ceiling alters the schema-0 compatibility
contract and requires a new canonical profile or object schema decision; it must not
be silently changed in a patch release.

Implementations MUST decode, re-encode, compare bytes, validate schema invariants, and
then verify the digest before admitting untrusted data to durable storage.

Canonical test vectors will include every object kind, all integer-width boundaries,
map ordering, Unicode normalization, invalid duplicates, forbidden floats/tags,
truncation, excessive nesting, and both registered hash algorithms. Vectors contain
semantic fixture JSON, canonical CBOR hex, preimage hex, and final binary/text IDs.

## Registry governance and evolution

The project maintains registries for object kinds, field keys per kind, hash codes,
signature purposes, and critical extensions. Assigned values are never reused. A
canonical profile change increments the ID format and necessarily changes IDs. A
schema change alone increments that object kind's schema version. Supported schemas
are registered per object kind, not as one global current version. Operation schema 1
is the first additive assignment; all schema-0 encodings and IDs remain unchanged.

Open implementation detail: select a CBOR crate only after it passes rejection and
round-trip tests for this profile. Library-provided “canonical mode” is not assumed to
be sufficient without byte-level test vectors.
