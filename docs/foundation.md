# Foundation

## Core Position

Do not start by asking, "How can we make Git nicer?"

Start by asking:

"What should source control feel like if it were invented for teams shipping software through a hosted collaboration platform in 2026?"

That framing changes the defaults.

Source control should feel like:

- a durable workspace system
- a reviewable stream of changes
- a shared project memory
- a reliable deployment handoff

It should not feel like:

- manual pointer manipulation
- shipping opaque commit graphs between machines
- constant awareness of branch state
- fear of history rewriting

## Problems To Solve

Git's internals are elegant. Git's user model is not.

### 1. The working model is too indirect

Most developers think in terms of:

- "what am I changing?"
- "why am I changing it?"
- "is it ready for review?"
- "is it safe to ship?"

Git asks them to think in terms of:

- index state
- branch position
- rebase strategy
- local versus remote divergence

Those are implementation details presented as product concepts.

### 2. Local and remote truth are split too hard

In Git, a local repository is one truth and the host is another. That is technically flexible, but operationally messy.

Modern teams usually expect:

- a canonical hosted project state
- clear collaboration history
- automatic previews and checks
- auditability tied to identities and permissions

The product should treat hosted collaboration as a first-class design target instead of a thin layer on top.

### 3. Branches carry too much meaning

A branch is simultaneously:

- a line of development
- a collaboration boundary
- a review unit
- a deployment candidate
- a personal scratch space

Those should be separate concepts.

### 4. Commits are a poor user-facing primitive

Commits are good storage objects and decent audit units. They are not ideal as the main thing humans organize around.

People want to reason about:

- a task
- a proposal
- a stack of dependent changes
- a release decision

Commit history is often forced to impersonate all four.

### 5. Push and pull are too manual

If a project has a trusted hosted home, syncing should usually be continuous, intentional, and visible.

Developers should not need to remember when their work exists only on one machine.

## First-Principles Design

Replace Git's user-facing primitives with a smaller, more explicit set.

## Proposed Primitives

### 1. Workspace

A `workspace` replaces the user-facing meaning of a worktree and much of the meaning of a branch.

A workspace is:

- a named environment for making changes
- tied to a goal or topic
- syncable across devices
- resumable
- allowed to contain unpublished draft work

Examples:

- `fix-login-timeout`
- `design-new-cache-layer`
- `release-2-4-stabilization`

Key idea:

A workspace is not history. It is a live place to work.

### 2. Change

A `change` replaces the user-facing meaning of a commit or pull request.

A change is:

- an intentional unit of work
- described by title, rationale, and scope
- reviewable
- testable
- publishable

A change can internally contain many saved states. Users do not need every save to become a first-class historical object.

Key idea:

Developers think in changes, not commit hashes.

### 3. Save

A `save` replaces the operational role of a commit for durability, but not necessarily for primary UX.

A save is:

- immutable
- content-addressed
- cheap
- frequent
- usually auto-created

Saves are the machine-level safety layer. They should be available, inspectable, and diffable, but not the main cognitive burden.

Key idea:

Commits become infrastructure.

### 4. Line

A `line` replaces the idea of a branch as shared project history.

A line is:

- a stable shared stream such as `main`, `release/2.4`, or `experiment/runtime-v2`
- protected by policy
- the thing environments deploy from

Unlike workspaces, lines are shared public history, not personal activity surfaces.

Key idea:

Separate private work context from shared integration history.

### 5. Publish

`publish` replaces `push`.

Publishing is not "send my whole local graph to a remote."

Publishing means:

- send selected changes or saves to the host
- attach them to a workspace or line
- trigger review, checks, previews, and policy

This is closer to "submit work to the system" than "synchronize refs."

### 6. Sync

`sync` replaces much of `pull`, fetch, and ad hoc backup behavior.

Sync is continuous and usually automatic.

It keeps:

- workspace draft state
- saves
- metadata
- collaborators' updates

aligned with the host and other trusted devices.

Key idea:

Backup and collaboration should be defaults, not rituals.

## Replacement Map

Git concept to new concept:

- `worktree` -> `workspace`
- `commit` -> `save` internally, `change` externally
- `branch` -> split into `workspace` and `line`
- `push` -> `publish`
- `pull/fetch` -> `sync`
- `pull request` -> review state on a `change`
- `stash` -> draft saves inside a workspace
- `rebase/cherry-pick` -> change composition operations

## Example User Workflow

### Start work

Instead of:

- create branch
- switch branch
- maybe open a draft PR later

The user does:

- create workspace `fix-login-timeout`
- make edits
- saves happen automatically or on demand

### Prepare for review

Instead of:

- clean up commit history
- push branch
- open pull request

The user does:

- create change from workspace
- optionally split workspace output into multiple changes
- publish change

The host then:

- runs checks
- creates preview environments
- attaches reviewers
- shows discussion against the change, not against a raw branch

### Integrate

Instead of:

- merge or rebase branch into `main`

The system does:

- validate the change against the target line
- compose it into the line using the repository's integration policy
- preserve authored saves for audit without forcing the line history to mirror local drafting history

## Architecture Direction

The design should be layered.

### Layer 1: Local workspace engine

Responsible for:

- filesystem tracking
- fast local diffs
- offline saves
- restore and undo

### Layer 2: Repository history engine

Responsible for:

- immutable saved states
- structural diffs
- rename and move tracking
- merge and conflict calculation

### Layer 3: Collaboration host

Responsible for:

- identity
- permissions
- review
- policies
- previews
- automation
- canonical project visibility

### Layer 4: User workflow

Responsible for:

- workspace management
- creating and publishing changes
- integrating reviewed work
- release lines and promotion

Git blurred these layers. A redesign should keep them distinct.

## What The Host Should Be

If the website is "not GitHub but similar," it should not just host a bare repository with nicer pages.

It should be a source control operating system.

The host should natively understand:

- workspaces
- changes
- lines
- review state
- deployment state
- environment state

That means the canonical object model for the website is not:

- repo
- branch
- commit
- PR

It is:

- project
- line
- workspace
- change
- save
- environment

## Opinionated Defaults

If this system is to be meaningfully better than Git, it should be more opinionated.

Suggested defaults:

- auto-save local work
- auto-sync drafts to the host when permitted
- every published change has a human-readable purpose
- review attaches to changes, not branches
- shared lines are protected and policy-driven
- deployment metadata is part of project history
- conflict detection begins before integration time

## What To Keep From Git

Do not throw away the good parts just because the UX is flawed.

Keep:

- immutable content-addressed storage
- local-first operation
- cheap branching internally
- strong diff and merge machinery
- offline safety
- cryptographic integrity

The redesign target is the experience model, not a rejection of durable snapshot storage.

## Hard Questions

These need explicit answers before implementation.

### 1. What is the canonical history?

Options:

- every save is permanent history
- only published saves are canonical
- changes have stable identities and save history is partially hidden

My current bias:

Published saves and integrated change states become canonical project history. Private draft saves remain durable but are not part of the main shared story by default.

### 2. How should merges work?

Options:

- traditional three-way merge over snapshots
- operation-based merge
- semantic merge for structured languages

My current bias:

Start with snapshot-based merge plus richer metadata. Leave semantic merge as a later layer.

### 3. Is this truly distributed?

If the product assumes a trusted host, it is not Git-style fully distributed in practice.

That is acceptable if the tradeoff is explicit.

My current bias:

Build a host-first collaborative system with strong local offline capabilities, rather than pretending all peers are equal when most teams use a canonical hosted service anyway.

### 4. How much history editing should exist?

Git permits extensive history surgery. That is powerful and dangerous.

My current bias:

Allow private workspace cleanup before publish. Once a change is published and reviewed, history should become much harder to rewrite.

## MVP Proposal

The first version does not need to replace all of Git.

It needs to prove the model.

### MVP scope

Build:

- a local CLI
- a local repository format
- workspaces
- saves
- changes
- lines
- publish to a single hosted service
- basic review metadata

Defer:

- full Git interoperability
- advanced merge UX
- monorepo scale optimizations
- semantic code understanding
- deployment orchestration

### MVP CLI sketch

Possible commands:

- `src init`
- `src workspace new fix-login-timeout`
- `src save`
- `src change create`
- `src publish`
- `src line list`
- `src line integrate <change>`
- `src sync`

The naming can change. The important part is that commands reflect user intent, not storage internals.

## Recommended Next Step

Do not jump into implementation yet.

Write the next document:

`docs/object-model.md`

It should define:

- exact entities
- identities
- state transitions
- local versus hosted ownership
- minimal storage format

Without that, implementation will drift back toward "Git with different command names."
