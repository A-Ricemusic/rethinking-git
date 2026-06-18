# Rethinking Git

This project starts from a simple premise:

Git solved important problems, but its mental model is not the one most developers would design today.

If we were building source control from scratch for modern teams, cloud-hosted collaboration, preview environments, code review, and AI-assisted development, we would probably not begin with:

- detached content-addressed snapshots as the primary user abstraction
- branches as movable pointers to commit chains
- a split between local and remote truth
- push and pull as explicit synchronization rituals
- a staging area that is both powerful and routinely confusing

This repository is for redesigning source control from first principles.

## Goal

Design a system that keeps the strengths of Git:

- offline-friendly history
- cheap branching
- immutable records
- reproducibility
- distributed safety

while replacing the parts that feel accidental, overloaded, or hostile to normal product work.

## Starting Point

The first design draft lives here:

- [docs/foundation.md](/Users/pelicannurse/Documents/Apps/rethinking-git/docs/foundation.md)
- [docs/permissions-and-sync.md](/Users/pelicannurse/Documents/Apps/rethinking-git/docs/permissions-and-sync.md)
- [docs/source-control-landscape.md](/Users/pelicannurse/Documents/Apps/rethinking-git/docs/source-control-landscape.md)

It defines:

- the problems we want to solve
- the primitives that replace `worktree`, `commit`, `branch`, `remote`, and `push`
- a candidate collaboration model for a GitHub-like hosting service
- a proposed MVP
- a first pass at permission-aware sync and secrets handling
- research on existing Git alternatives and workflow experiments

## Working Thesis

Git mixed three concerns into one system:

1. content storage
2. collaboration protocol
3. user workflow

That gave us a powerful engine, but also a hard-to-learn product.

This project treats those as separate layers and redesigns the developer-facing workflow first, then fits storage and networking underneath it.
