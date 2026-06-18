# JJ-Inspired Primitives

## Starting Point

Jujutsu's most important idea is not "better Git commands."

It is the split between:

- a stable logical change
- the exact immutable snapshots that represent that change over time

That distinction is the right foundation for this project.

In Git, the commit is both the logical unit and the storage unit.

In a `jj`-style system:

- the `change` is the logical unit
- the `snapshot` is the storage unit
- the `operation` records how repository state changed

That is a cleaner base.

## Core Primitives

### Change

A `change` is the thing a human or agent is trying to accomplish.

Examples:

- `fix-login-timeout`
- `add-billing-webhook`
- `agent/refactor-settings-page`

A change has:

- stable identity
- title
- description
- author or agent owner
- current snapshot
- parent changes
- review state
- target line

The change survives edits. The snapshot changes as files change.

This is the primitive users should think about most often.

### Snapshot

A `snapshot` is an immutable record of file state.

It is closer to a Git commit object than a Git commit workflow.

A snapshot has:

- content hash
- manifest of paths
- blob references
- parent snapshot references
- timestamp
- authoring context

Snapshots are cheap and frequent.

The system can create them automatically:

- before each command
- after file changes
- before an agent command
- before tests
- before publishing a change

The user does not need to decide whether their work is "commit-worthy" before it is durable.

### Workspace

A `workspace` is the editable view of a change.

It contains:

- current change
- filesystem root
- materialization rules
- environment profile
- permission scope

Unlike Git, a workspace is not a branch.

A workspace is where work happens.

### Workspace Instance

A `workspace instance` is one local materialization of a workspace.

One change can have multiple instances:

- laptop directory
- desktop directory
- remote agent sandbox
- CI preview checkout

This solves a major Git worktree problem: the task identity and the local folder are separate.

### Line

A `line` is protected shared history.

Examples:

- `main`
- `release/2.4`
- `production`

A line is not where users do normal work.

Users work on changes. The system integrates changes into lines.

This replaces the overloaded Git branch concept.

### Marker

A `marker` is a named pointer to a snapshot or change.

Examples:

- `v1.0.0`
- `staging-2026-06-18`
- `customer/acme-approved`
- `security-reviewed`

Markers cover what Git tags and some branch names try to do.

They should have explicit types:

- release marker
- deployment marker
- review marker
- policy marker
- bookmark marker

This avoids treating every name as the same kind of pointer.

### Operation

An `operation` records a repository action.

Examples:

- created change
- updated workspace snapshot
- split change
- rebased change
- integrated change into line
- granted access to protected material
- materialized workspace instance

Operations power undo, redo, audit, and collaboration recovery.

This is one of `jj`'s best ideas and should be first-class from the start.

## How This Replaces Git

| Git Concept | New Concept |
| --- | --- |
| commit | snapshot plus change metadata |
| branch | line, workspace, or marker depending on intent |
| tag | typed marker |
| worktree | workspace instance |
| stash | old snapshots in the change evolution log |
| reflog | operation log |
| merge | integrate changes into a line |
| rebase | recompute change parents and snapshots |
| pull request | published change with review state |

## Recommended Mental Model

The user-facing flow should be:

```text
create change
work in workspace
snapshots happen automatically
publish change
review change
integrate change into line
mark release or deployment
```

The internal flow should be:

```text
operation -> change update -> snapshot -> workspace materialization
```

## What To Steal From JJ

### 1. Working copy as a real object

There should be no special dirty state.

The files on disk are the current materialized snapshot of a change.

### 2. Automatic snapshots

Source control should protect work before users think to protect it.

### 3. Stable change IDs

The user needs a stable thing to talk about even as the exact file contents evolve.

### 4. Operation log

Undo should apply to source-control actions, not just editor text changes.

### 5. History editing without fear

Rewriting local work should be normal because the operation log can recover prior states.

## Where We Should Go Beyond JJ

### 1. Permissioned materialization

JJ does not solve repo-wide file permissions.

Our system should allow a workspace instance to materialize only the files and protected material that identity can access.

### 2. Environment profiles

JJ does not solve the "new worktree is not runnable" problem by itself.

Our system should attach environment profiles to workspaces.

### 3. Agent history

JJ tracks source-control operations.

We should also track agent prompts, commands, test runs, generated diffs, and review comments as structured operations.

### 4. Typed markers

Git tags and branches are too generic.

Markers should encode purpose so the product understands the difference between a release, deployment, review checkpoint, and bookmark.

## Minimal Rust MVP

The next prototype should implement only:

- `init`
- `change new`
- `snapshot`
- `status`
- `operation log`
- `workspace info`
- `line list`
- `marker set`

Do not start with branches.

Do not start with push.

Do not start with merge.

The first milestone should prove:

```text
stable change identity + evolving snapshots + operation log
```

That is the foundation.
