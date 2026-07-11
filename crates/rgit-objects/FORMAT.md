# RGit schema-0 registry

This crate is an additive implementation of `spec/objects.md` and
`spec/canonical-encoding.md`. It does not read or rewrite the prototype CLI's JSON
repository format.

The schema-0 registry below is provisional until the registry ADR is accepted. Tests
exhaustively pin the currently implemented assignments, but this pre-1.0 document
does not yet claim that numeric values can never change.

Object-kind registry: chunk 1, blob 2, secret reference 3, manifest 4, subproject 5,
snapshot 6, change revision 7, line state 8, conflict 9, operation 10, marker 11,
release 12, and policy 13. Each schema uses common field 0
for kind, 1 for schema version, and 2 for the exact policy reference. Remaining
numeric assignments are the field numbers emitted in `src/object.rs` and accepted
by the closed decoder in `src/decode.rs`.

Schema 0 limits canonical metadata to 16 MiB, nesting to 64 levels, an individual
byte/text string to 16 MiB, and an array/map to 1,000,000 elements. Storage may set
stricter limits. A chunk is policy-bound, preventing cross-policy equality oracles.
Chunked blobs record algorithm, version, minimum, target, and maximum sizes; the
initial chunking algorithm registry remains unassigned until its ADR is accepted.

Line-state generation zero is the only genesis representation and must omit a
previous state. All later generations require one. A line-advance operation embeds
the complete intended state declaration but never the new line-state ID; the signed
line state subsequently points to the finalized operation. Generic transitions may
refer to an old line state as `before` but may not install one as `after`.

Schema objects carry fixed profile-0 signature records. Algorithm 0 is Ed25519; key
IDs are exactly 32 bytes, signature bytes exactly 64 bytes, and purposes are numeric
registry values 0 (line state), 1 (operation), 2 (marker), 3 (release), and 4
(policy). Multi-signature arrays are nonempty, canonically sorted, and unique. The
unsigned projection omits only field 13, 12, 8, 16, or 15 respectively. The exact
`RGIT-SIGNATURE\0` preimage grammar is frozen in `spec/canonical-encoding.md`.
Signature verification belongs to the crypto layer; this crate deliberately has no
private-key signing API and accepts no placeholder or unsigned production object.
Genesis additionally requires an external trust anchor until the Identity and
RepositoryRoot schemas land in the next registry slice.

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
