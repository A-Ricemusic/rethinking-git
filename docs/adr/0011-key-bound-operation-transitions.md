# ADR 0011: Key-bound mutable-reference transitions

- Status: Accepted
- Date: 2026-07-11
- Owners: objects and storage

## Context

Operation schema 0 signs typed before/after object references but does not name the
mutable reference being changed. Two marker or change references with the same target
kind can therefore be substituted without changing that action. Inferring intent from
publication order or a database row would make mutable state authoritative.

## Decision

Keep schema-0 Operation byte-for-byte frozen and add Operation schema 1. Its new
bound-transition action signs a canonical closed `ReferenceKey` containing both the
namespace kind and stable ID, plus the typed before and required after object. The
existing line-advance action already binds its LineId and remains unchanged.
Operation-head updates remain bound by the Operation ID and parent relation.

Schema 1 rejects schema-0 generic transitions, duplicate exact keys,
line/operation-head use through the generic action, and object kinds inconsistent
with the key. Schema support is dispatched by `(ObjectKind, schema_version)`.

Schema-0 objects remain readable and retain their IDs, signatures, and parent edges,
but production publication requires schema 1. Repository activation and storage
migration are separate from this canonical object decision.

## Consequences

- A signed Operation determines which marker, change, or release reference it
  authorizes without consulting mutable state.
- Schema-1 signatures and IDs differ because schema is domain-separated.
- Operation DAGs may contain both schema versions; no immutable object is rewritten.
- Canonical objects and storage share one stable reference-key registry.

## Rejected alternatives

- Mutate schema 0: this invalidates frozen vectors, IDs, and signatures.
- Infer the key from kind or update order: same-kind references remain substitutable.
- Add stable IDs to every target object: this does not bind the signed command to the
  selected mutable namespace and causes unrelated schema changes.

## Verification

Freeze signed/unsigned schema-1 vectors and both hash IDs. Negative vectors cover
legacy actions, malformed keys, wrong target kinds, duplicate keys, unknown schemas,
and marker-key substitution. Schema-0 vectors remain byte-identical.
