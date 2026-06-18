# Access Control

## Core Idea

Git has one repository history.

If you can read the repository, you can usually read every committed file, every old version, and every commit message.

This project should use a different model:

`history is a permissioned view over repository objects`

That means two people can sync the same project and see different files, changes, snapshots, workspaces, and operations.

The repository is not one universal filesystem.

It is a graph of objects, and each actor gets a materialized view of the objects they are allowed to access.

## Required Shift

This cannot be solved with UI hiding.

If a client receives a blob, that client can read it.

If a client receives an old vulnerable file version, that client can inspect it forever.

If a client receives full operation metadata, that metadata can leak sensitive intent.

So access control has to exist at the storage and sync layer.

The server should only send objects the actor can access, and restricted objects should be encrypted so storage access is not the same as read access.

## Access-Controlled Objects

Every major primitive needs visibility rules.

### File Blob

A file blob is raw content.

Examples:

- source file
- `.env`
- signing key
- exploit proof-of-concept
- customer config

Blob access decides whether the actor can read file bytes.

### Snapshot

A snapshot is a manifest of files.

In an access-controlled system, a snapshot is not just:

```text
path -> blob
```

It is:

```text
path -> object reference -> visibility policy -> materialization behavior
```

For unauthorized actors, a snapshot may show:

- the file normally
- a redacted placeholder
- an omitted path
- metadata only

### Change

A change is logical work.

Some changes should be public to the project.

Some changes should be visible only to:

- security team
- release managers
- a customer support group
- a specific automation tool
- the author and reviewer

Unauthorized users may see no change at all, or they may see a redacted change like:

```text
security update applied
```

without the files, discussion, exploit details, or intermediate snapshots.

### Workspace

A workspace is a working context.

Workspace access controls who can:

- see that the workspace exists
- materialize it locally
- read its files
- create snapshots inside it
- run tools against it

Agent workspaces should be scoped tightly because agents may have tool access that humans do not.

### Operation

Operations are sensitive because they reveal what happened.

Examples:

- `created_security_fix`
- `rotated_prod_secret`
- `granted_agent_access`
- `integrated_private_change`

An operation should support split visibility:

- public shell: something happened
- private payload: exact details
- audit payload: visible only to authorized auditors

This allows a public project history to remain coherent without leaking dangerous details.

## Policy Domains

Instead of putting a raw user list on every object, use policy domains.

Examples:

- `public`
- `team/backend`
- `team/security`
- `env/production`
- `customer/acme`
- `tool/ci-release`
- `tool/security-scanner`

Each object belongs to one or more domains.

Actors receive capabilities for domains.

If an actor has the needed capability, the object can be materialized.

## Materialized Views

The key product primitive is the `view`.

A view is the repository as seen by one actor.

Examples:

- frontend developer view
- security engineer view
- production CI view
- external contractor view
- customer support view
- agent sandbox view

The same snapshot can produce different views.

For a security engineer:

```text
src/auth/login.ts
src/auth/rate_limit.ts
security/exploit-repro.test.ts
.env.security
```

For a normal app developer:

```text
src/auth/login.ts
src/auth/rate_limit.ts
security/exploit-repro.test.ts [redacted]
.env.security [not materialized]
```

For an external contractor:

```text
src/auth/login.ts
src/auth/rate_limit.ts
```

## Security Fix Example

A security engineer creates:

```text
change: fix-token-replay-vulnerability
domain: team/security
```

The change contains:

- vulnerable old behavior
- exploit reproduction
- patched code
- private review discussion
- private snapshots

When the change is integrated, the public line can move forward without exposing the private change history.

Public project history might show:

```text
operation: integrated security update
visible files: patched public source files
hidden files: exploit reproduction, private notes, private snapshots
```

Security team history shows:

```text
operation: integrated fix-token-replay-vulnerability
change: fix-token-replay-vulnerability
snapshots: all private snapshots
files: exploit reproduction, tests, patch
discussion: full review
```

The public view gets the fixed state.

The private view gets the full explanation and audit trail.

This does not make old released software impossible to analyze, but it prevents the repository from handing every collaborator a map of the vulnerability.

## Redacted History

This system needs first-class redaction.

Redaction does not mean deleting history.

It means showing a different representation to actors without access.

Possible redacted forms:

- object omitted entirely
- path visible but content hidden
- change title replaced with generic text
- operation payload hidden
- snapshot edge hidden
- diff collapsed into "restricted changes"

Authorized auditors should still be able to reconstruct the full chain.

## Encryption Model

At minimum:

- blobs are encrypted by policy domain
- private snapshot manifests are encrypted
- private change metadata is encrypted
- private operation payloads are encrypted
- access grants wrap decryption keys for allowed actors

The server can enforce policy, but encryption should limit damage if server storage is copied or misconfigured.

## Prototype Direction

The next prototype should not implement real cryptography yet.

First, add policy metadata and filtered views.

Suggested first fields:

```text
visibility: public | restricted
domains: [team/security, tool/ci-release]
redaction: omit | placeholder | metadata_only
```

Suggested commands:

```text
rgit actor set alice --domain public
rgit actor set sec-eng --domain public --domain team/security
rgit snapshot --domain public
rgit snapshot --domain team/security
rgit status --as alice
rgit status --as sec-eng
rgit op log --as alice
rgit op log --as sec-eng
```

That would prove the product idea:

same repository, different materialized history.

## Hard Rule

If an actor is not allowed to read an object, that object should not be sent to their client in readable form.

Everything else is just decoration.
