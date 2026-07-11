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

The byte sequence signed for a purpose is:

```text
"RGIT-SIGNATURE\0" || uvarint(purpose_length) || purpose_utf8 || object_id_bytes
```

Kinds and signature purposes are registry values, not user strings. Unsigned LEB128
is used for the shown varints and MUST use its shortest encoding. Hash computation is
streaming and MUST verify that the payload itself declares the same kind and version.

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
default maximum canonical metadata object is 16 MiB; chunks carry bulk content.
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
schema change alone increments that object kind's schema version.

Open implementation detail: select a CBOR crate only after it passes rejection and
round-trip tests for this profile. Library-provided “canonical mode” is not assumed to
be sufficient without byte-level test vectors.
