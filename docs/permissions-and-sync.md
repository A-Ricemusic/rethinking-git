# Permissions And Sync

## Why This Matters

One of Git's sharpest limitations is that repository access is usually all-or-nothing.

If a file is committed, anyone with repository access can usually read it.

That breaks down for:

- environment variables
- private signing keys
- customer-specific configuration
- regulated data
- internal-only operational scripts

Developers work around this by keeping secrets outside source control, usually with `.env` files, secret managers, onboarding scripts, and local setup glue.

That workaround is common because Git is missing a native permission model for repository contents.

## Product Goal

The new system should support this behavior:

- a developer can sync a working project and get a complete runnable environment
- different users can receive different files or different file contents based on policy
- sensitive material is still durable, versioned, and auditable
- the host can enforce access without forcing every secret to live in a separate product

In short:

`sync` should materialize the repository the user is allowed to see, not a single identical filesystem for everyone.

## Core Tension

This is not just a UI feature.

Permission-scoped sync changes:

- storage
- encryption
- diffing
- review
- caching
- local clones
- automation

If the host stores restricted files in plaintext and merely hides them in the UI, the model is weak.

If the client receives encrypted blobs but lacks keys, the host can safely store broader repository state without every user being able to read it.

That means the permission system must be designed into the object model, not layered on later.

## Proposed Primitive: Protected Material

Add a first-class repository object type:

`protected material`

Protected material is any tracked content whose visibility is governed by policy.

Examples:

- `.env`
- `.env.production`
- `ios/signing/Distribution.mobileprovision`
- `ops/prod-access.tfvars`
- `customers/acme/private-config.json`

Protected material should support:

- version history
- scoped visibility
- encrypted storage
- audit logs
- policy inheritance

## Three Visibility Classes

Not every file needs the same handling.

### 1. Public project material

Normal code and assets.

- visible to all project members with repo read access
- diffable and reviewable by default
- cached and mirrored freely inside the project's trust boundary

### 2. Restricted project material

Sensitive files needed by some, but not all, collaborators.

- visible only to members with matching roles or grants
- stored encrypted at rest
- filenames may be visible even if contents are not, depending on policy
- change metadata is always auditable

### 3. Secret material

High-risk content such as credentials, private keys, or customer secrets.

- contents always encrypted end-to-end
- retrieval allowed only for users, services, or environments with explicit grants
- review may show metadata and policy status without exposing raw content
- plaintext should avoid long-lived storage on the host where possible

This distinction matters because not every sensitive file should be treated like a password.

## Two Delivery Models

There are two honest ways to implement "different users pull different files."

### Model A: Selective materialization

All repository objects may exist on the host, but a user sync only materializes the files they are allowed to read.

Pros:

- natural user experience
- fast workspace bootstrap
- avoids local clutter

Cons:

- absent files can affect builds in confusing ways
- path visibility itself can leak information
- harder to reason about consistent diffs across roles

### Model B: Universal structure, selective decryption

All users sync the same project structure, but restricted content is encrypted and only users with keys can materialize plaintext.

Pros:

- repository topology stays consistent
- easier merges and diffs at the structure layer
- stronger storage model

Cons:

- client and key management are more complex
- UX must handle encrypted placeholders cleanly
- reviews need role-aware rendering

My current bias:

Use a hybrid of A and B.

- normal files sync normally
- restricted paths may sync as encrypted placeholders or access-denied stubs
- authorized users transparently decrypt during workspace materialization

That preserves structure without pretending everyone should see everything.

## Replace `.gitignore`-Style Secret Handling

Today, teams often rely on:

- `.env.example`
- setup docs
- secret managers
- ignored local files
- scripts that fetch runtime config

The redesign should absorb part of that workflow natively.

Instead of "do not commit secrets," the system can support:

- `track this file as secret material`
- `grant access to backend team and production CI`
- `materialize this into my workspace on sync`
- `rotate this value and record who received it`

That is a much stronger product model.

## New Primitive: Environment Profile

Your worktree complaint is really about missing environment state.

The system should therefore track not just source files, but a second object:

`environment profile`

An environment profile defines the non-source material required to make a workspace runnable.

It can include:

- protected files
- package manager state
- local service definitions
- toolchain versions
- generated config inputs

Examples:

- `web-dev`
- `ios-release-signing`
- `customer-acme-support`

Workspace creation should be able to say:

- create workspace `feature/a`
- attach environment profile `web-dev`
- materialize all permitted protected material

That is better than copying whatever happened to exist in another directory.

## Important Constraint

Do not confuse convenience cloning with secure source control.

If a user should not be allowed to see production secrets, a "hard clone everything" model is wrong.

The better design is:

- clone the whole workspace structure
- materialize all non-sensitive project content
- materialize only the protected material that policy allows
- make missing protected material explicit, not silent

That keeps the bootstrap experience good without collapsing security boundaries.

## What Replaces Branches And Worktrees

Your complaints about branches and worktrees are valid because Git conflates:

- a line of history
- a working directory
- a task context
- a local checkout location

Those should be separate.

### Proposal

Use:

- `line` for shared integration history
- `workspace` for the task context
- `workspace instance` for a local materialization on a device

That means one workspace can have:

- a laptop instance
- a desktop instance
- a temporary review instance

Each instance can live in a different directory without stealing the meaning of `main` from another path.

This directly addresses:

- multiple simultaneous tasks on one machine
- multiple local directories for the same work context
- deterministic environment rehydration

## Collaboration Model

The system should stop assuming that everyone constantly rebases private history against everyone else.

Instead:

- users create saves inside workspaces
- workspaces sync incrementally to the host
- changes are published from workspace state
- integration into a line is a host-mediated operation

That shifts the hard reconciliation point from "everyone manually merges branch histories" to "the system composes reviewed changes into a protected line."

Conflicts still exist, but they become integration events, not constant local branch maintenance.

## Review And Audit

Permissioned material creates review challenges.

The system should support:

- metadata-only review for unauthorized reviewers
- full-content review for authorized reviewers
- policy audit trails for every read, sync, and grant
- proof that a change touched secret or restricted material

That means a change can say:

- modified 3 public files
- modified 1 restricted file
- reviewed by 2 authorized reviewers

without leaking the restricted file contents to everyone.

## Key Management Direction

This design is only credible with explicit key handling.

Minimum viable direction:

- each organization has a root trust domain
- each user and automation actor has an identity key
- protected material is encrypted with per-material or per-scope keys
- access grants are represented by encrypted key-wrapping metadata

This is harder than Git, but it is the cost of real content permissions.

## MVP Boundary

Do not try to build the full permission system first.

Start with:

- normal tracked files
- protected files with organization-managed encryption
- role-based materialization at sync time
- environment profiles attached to workspaces
- audit logs for reads and syncs

Defer:

- per-lineage cryptographic proofs
- customer-by-customer shard optimization
- semantic diffs for encrypted structured data
- offline multi-device key recovery

## Implications For Implementation Language

The core engine should be written in a systems language.

Requirements:

- fast filesystem traversal
- efficient local database access
- good concurrency
- strong binary distribution story
- safe handling of cryptography and network protocols

### Best options

#### Rust

Best overall fit for a new implementation.

Why:

- memory safety matters for a long-running sync engine
- strong ecosystem for CLI, storage, and crypto
- good cross-platform static binaries
- easier to maintain safely than C

Tradeoff:

- higher complexity than Go for some teams

#### Go

Good fit for the host services and acceptable for the CLI.

Why:

- simple deployment
- good networking and concurrency
- fast team onboarding

Tradeoff:

- less precise control over low-level storage and memory behavior
- crypto and local engine ergonomics are fine, but not as compelling as Rust for a source-control core

#### C++

Technically viable, but I would not choose it for a greenfield redesign unless there is a very specific performance reason.

## What Git Was Written In

Git was originally written primarily in C.

Historically it also used shell scripts heavily, and parts of the broader tooling around Git have used Perl, Tcl/Tk, and other languages. But the core Git implementation is C.

## Current Recommendation

If we are serious about building this:

- write the local engine and sync client in Rust
- write the hosted control plane in Rust or Go
- keep cryptography and storage formats in the same language as the core engine

The wrong move would be building the core in a scripting language and discovering later that filesystem scale, diff speed, local caches, and secure sync all need a rewrite.

## Next Document

The next useful spec is:

`docs/object-model.md`

It should define:

- `workspace`
- `workspace instance`
- `save`
- `change`
- `line`
- `protected material`
- `environment profile`
- access grants
- sync and materialization rules
