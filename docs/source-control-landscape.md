# Source Control Landscape

Research date: 2026-06-18

This document surveys systems that are rethinking source control, Git workflow, code review, or repository hosting.

The key distinction:

- some tools replace the storage model
- some tools keep Git but replace the workflow
- some tools keep Git and add agent/worktree orchestration
- some tools solve adjacent problems like large binary assets or versioned data

That distinction matters because a polished Git wrapper cannot solve problems that require a different object model, permission model, or sync protocol.

## High-Level Takeaways

### 1. Rust is the dominant language for new source-control engines

Jujutsu, GitButler, Radicle, Zed, and large parts of Sapling are Rust-heavy.

That is not accidental.

Source control needs:

- filesystem traversal
- local databases
- crypto
- network protocols
- cross-platform binaries
- safe concurrency

Rust is a strong fit for that mix.

### 2. Most modern tools are not replacing Git storage yet

Many products are Git-compatible or Git-backed:

- Jujutsu currently stores commits and files in Git, while keeping higher-level metadata outside Git.
- GitButler is explicitly Git-backed.
- Radicle is a peer-to-peer forge built on Git.
- Cursor and Zed worktree features use Git worktrees.

The practical lesson:

Git compatibility is useful for adoption, but it can trap the product inside Git's permission, branch, and commit assumptions.

### 3. The most interesting ideas are above commits

The real innovation is happening around:

- working-copy commits
- automatic save/undo
- stacked changes
- virtual branches
- operation-level history
- agent conversation history
- partial materialization
- virtual filesystems

This supports our direction: make `save`, `change`, `workspace`, and `line` separate concepts.

### 4. Nobody mainstream has solved permissioned repository contents cleanly

Large-file tools, enterprise VCS products, and hosted services have access controls, but most source-control workflows still treat repository read access as broad read access.

Our `protected material` idea remains differentiated if it becomes a real storage and sync primitive instead of just UI filtering.

## Comparison Table

| System | What It Rethinks | Git Relationship | Main Language/Stack | Relevant Ideas |
| --- | --- | --- | --- | --- |
| Jujutsu `jj` | Working copy, history editing, backend abstraction | Git-compatible, often uses Git as storage | Rust | Working copy is a commit, easy undo, change IDs, storage separated from UX |
| Sapling | Monorepo scale, stacked commits, smartlog UX | Git-compatible client; evolved from Mercurial ideas | Rust, Python, C++, TypeScript | Scales to huge repos, virtual filesystem, stack-first workflow |
| GitButler | Branch UX, parallel work, agent workflows | Git-backed | Rust, TypeScript, Svelte, Tauri | Virtual/parallel branches, operation log, drag-and-drop commit surgery |
| Zed Git/worktrees | Editor-native worktree and agent isolation | Uses Git worktrees | Rust editor | Worktree lifecycle hooks, thread history tied to worktrees |
| Zed DeltaDB | Operation-level versioning for agent work | Git remains for checks and external world | Not fully public; Zed is Rust-heavy | Fine-grained deltas, conversation plus code history, CRDT worktrees |
| Cursor agents | Agent execution isolation | Uses Git/worktrees and GitHub integration | Proprietary; VS Code/Electron lineage | Cloud/background agents, isolated worktrees, PR handoff |
| Pijul | Patch theory and merge correctness | Independent VCS | Rust | Commutative patches, partial clones, conflict resolutions as changes |
| Darcs | Patch-first source control | Independent VCS | Haskell | Changes over snapshots, interactive patch selection |
| Fossil | All-in-one project system | Independent DVCS with Git mirroring | C, SQLite, Tcl/TH1 | Repo contains code, tickets, wiki, forum, web UI, autosync |
| Mercurial | Simpler distributed VCS | Independent DVCS, Git interop extensions | Python, C, Rust | Simpler commands, extension model, whole-history clones |
| Radicle | Decentralized forge, identity, collaboration | Built on Git | Rust | Peer-to-peer replication, signed social artifacts, no central host |
| Unity Version Control / Plastic SCM | Game/large asset workflows | Independent VCS with Git interop options | Proprietary; historically .NET/Mono ecosystem | File locking plus branching, centralized/distributed modes, large binaries |
| Diversion | Game/large repo cloud source control | Alternative to Perforce/Git LFS | Proprietary | Fast large-file workflows, real-time sync, browser/IDE/CLI surfaces |
| Graphite | Stacked review workflow | Built on Git/GitHub | Product stack not central to VCS engine | Stacked PRs, stack sync, review units smaller than branches |
| Dolt | Version control for SQL data | Git-like, not code VCS | Go | Branch/merge/diff semantics applied to structured tables |

## Cursor

Cursor is not currently replacing Git as the source-control engine.

What it is doing:

- using agents as parallel workers
- using Git worktrees for isolation
- integrating with GitHub for handoff
- supporting cloud/background agents

Cursor's lesson for this project:

Agent-native coding makes the worktree problem worse. Multiple agents need isolated filesystems, runnable environments, and a way to merge or discard their output. Git worktrees are a pragmatic substrate, but they inherit Git's environment bootstrap problems.

Useful idea to steal:

- treat an agent's work context as a first-class workspace
- preserve the conversation, plan, commands, diffs, and test results as part of the change
- make the handoff from agent work to review a native operation

Constraint:

Cursor is proprietary, so implementation details and exact source-control internals are not publicly verifiable.

## Zed

Zed has two relevant efforts.

### Current Git worktree integration

Zed supports creating and switching Git worktrees from the editor. It creates new worktrees in detached HEAD state, then expects the user to create or check out a branch inside that worktree.

It also has worktree lifecycle hooks, which directly addresses one of your pain points: after a worktree is created, run setup steps to make it usable.

Useful idea to steal:

- workspace creation should have lifecycle hooks
- a workspace should know how to rehydrate local setup
- agents should be scoped to isolated workspaces

### DeltaDB

Zed's DeltaDB is more radical.

Their claim is that software is increasingly made between commits, especially through conversations with agents. DeltaDB records fine-grained deltas and links messages to the edits they produced. They describe conflict-free replicated worktrees where multiple people and agents can edit the same files across machines, with the worktree mountable to disk.

Useful idea to steal:

- operation-level history matters
- conversation history should be linked to code history
- collaboration should happen before a commit or pull request exists
- line comments tied only to line numbers are too fragile

Risk:

Operation-level history can become noisy unless there is a higher-level object such as `change` to summarize intent.

## Jujutsu

Jujutsu is one of the most important systems to study.

Core ideas:

- the working copy is represented as a real commit
- commands automatically amend the working-copy commit
- branches are de-emphasized in favor of changes and bookmarks
- history editing is normal and safer than in Git
- storage is abstracted from the user-facing model
- Git is currently used as the storage layer for commits and files

Useful idea to steal:

- make in-progress work durable automatically
- remove the staging-area ceremony
- give changes stable IDs independent of commit hashes
- separate high-level workflow metadata from low-level storage

Design warning:

Jujutsu is still close to a commit graph. It improves Git's workflow substantially, but it does not solve permissioned file materialization or environment profiles.

## Sapling

Sapling comes from Meta and is shaped by monorepo scale.

Core ideas:

- usability and scalability are explicit goals
- smartlog gives a clearer view of stacked work
- stacks of commits are first-class
- the ecosystem includes Sapling client, Mononoke server, and EdenFS virtual filesystem
- operations are intended to scale with files used by a developer, not total repo size

Useful idea to steal:

- the local filesystem should not need to materialize the whole repository
- a virtual filesystem can make huge repos feel small
- stacks should be a product concept, not a fragile pile of manually rebased branches

Design warning:

Sapling still lives in a commit/stack world. It helps with scale and UX, but it is not primarily a permissioned-source-control model.

## GitButler

GitButler is Git-backed but aggressively redesigns the branch UI.

Core ideas:

- virtual/parallel branches let users work on multiple branches simultaneously
- commit management is visual and forgiving
- operations are logged so users can undo
- it targets modern agentic workflows

Useful idea to steal:

- multiple active branches/workstreams in one working directory can be useful
- undo history should apply to source-control operations, not only file edits
- users should manipulate changes visually and structurally, not by memorizing rebase commands

Design warning:

Because it is Git-backed, it cannot fully escape Git's file permission and checkout model.

## Pijul And Darcs

Pijul and Darcs are the most important patch-theory systems.

Darcs is older and Haskell-based. It focuses on changes rather than snapshots.

Pijul is newer and Rust-based. It advertises mathematically grounded patches, commutation, merge correctness, first-class conflicts, and partial clones.

Useful idea to steal:

- patches can be the semantic object, not snapshots
- conflict resolutions can themselves be durable changes
- partial clones can be principled rather than an afterthought

Design warning:

Patch theory is intellectually attractive, but product adoption depends on performance, tooling, and being understandable by normal teams.

## Fossil

Fossil is worth studying because it rejects the "VCS plus separate website" model.

Core ideas:

- single self-contained executable
- repository stored in SQLite
- built-in web UI
- bug tracker, wiki, forum, alerts, chat, and technotes live in the same system
- autosync reduces needless fork/merge churn

Useful idea to steal:

- the hosting site and source-control engine should share an object model
- project memory should include discussions, decisions, and operational metadata
- SQLite is a credible local repository database

Design warning:

Fossil is intentionally compact and self-contained. That is elegant, but a modern hosted collaboration product may need a stronger cloud and permissions architecture.

## Radicle

Radicle is not a new VCS engine so much as a decentralized Git forge.

Core ideas:

- no single central host
- repositories replicate peer-to-peer
- social artifacts are stored in Git
- artifacts are signed with public-key cryptography
- local-first ownership is central

Useful idea to steal:

- identity and signing should be native
- collaboration metadata should be portable, not trapped in a SaaS database
- a hosted service should not be the only possible source of truth

Design warning:

Our current direction is host-first with strong local capability. Radicle is closer to peer-first. We should consciously choose where we sit on that spectrum.

## Unity Version Control, Plastic SCM, Diversion, And Perforce-Like Systems

These are shaped by games, film, and asset-heavy teams.

Core ideas:

- huge binary files matter
- artists need a different UX than programmers
- file locking is still useful
- centralized workflows can outperform fully distributed ones at large asset scale
- cloud and on-prem options matter for enterprise teams

Useful idea to steal:

- file locking should exist for non-mergeable files
- source control should distinguish code, generated assets, binary assets, secrets, and environment material
- permissions and partial materialization matter more in very large projects

Design warning:

Centralized locking systems solve real problems, but they can feel slow or heavy for normal code workflows.

## Graphite And Stacked Diff Tools

Graphite, ghstack, spr, and related tools mostly keep Git and GitHub but change review workflow.

Core ideas:

- large pull requests are bad review units
- stacked changes are easier to review
- Git can represent stacks, but managing them manually is painful
- tooling can automate rebasing, syncing, and PR creation

Useful idea to steal:

- `change` should support dependencies directly
- review should happen on small logical units
- integration should understand stacks without requiring users to manually rebase every branch

Design warning:

Stacked PR tools are workflow improvements, not source-control replacements. They do not solve storage, permissions, or environment materialization.

## Dolt

Dolt is not a code source-control replacement, but it is useful for design thinking.

Core ideas:

- Git-like branch, merge, diff, push, and pull semantics can apply to SQL tables
- structured data gets domain-specific diffing and merging
- version control can be embedded inside another domain instead of treating files as the only unit

Useful idea to steal:

- not all repository material should be treated as bytes in files
- environment configuration, secrets, schemas, and generated metadata may need structured versioning

## Language Notes

Observed public implementation stacks:

| Project | Language Notes |
| --- | --- |
| Git | Core primarily C, historically with significant shell scripts around it |
| Jujutsu | Rust |
| Zed | Rust-heavy editor |
| GitButler | Rust, TypeScript, Svelte, Tauri |
| Sapling | Rust-heavy, with Python, C++, TypeScript, and other components |
| Pijul | Rust |
| Darcs | Haskell |
| Fossil | C, SQLite, Tcl/TH1, some JavaScript |
| Mercurial | Python with C and Rust components |
| Radicle | Rust |
| Dolt | Go |
| Cursor | Proprietary; public information points to a VS Code/Electron lineage, but source-control internals are not public |
| Unity Version Control / Plastic SCM | Proprietary; historically associated with .NET/Mono ecosystem |
| Diversion | Proprietary |

## Ideas Most Relevant To Our Design

### First-class workspace instances

Borrow from Cursor, Zed, and Git worktrees, but fix the missing environment problem.

Our version:

- `workspace` is the task context
- `workspace instance` is a local materialization
- each instance can attach an `environment profile`
- protected material materializes by permission

### Automatic durable saves

Borrow from Jujutsu and GitButler.

Our version:

- every workspace has automatic saves
- saves are not necessarily user-facing commits
- undo is built into the operation log

### Change-based review

Borrow from Graphite, Sapling, and Jujutsu.

Our version:

- `change` is the review unit
- changes can stack
- changes can contain saves, operations, messages, tests, and protected material metadata

### Operation/conversation history

Borrow from Zed DeltaDB.

Our version:

- agent prompts, plans, commands, file edits, and tests should be linked to the resulting change
- low-level operations should be queryable
- the user-facing history should still summarize intent

### Permission-aware materialization

This is the area where we can be most differentiated.

Our version:

- repository state can include public files, restricted files, and secret material
- sync materializes only what the identity is allowed to see
- encrypted placeholders preserve structure
- audit logs record access and grants

### Virtual filesystem and partial clone

Borrow from Sapling/EdenFS and Pijul partial clones.

Our version:

- large repos should materialize by workspace need
- permission boundaries and scale boundaries should share one materialization engine
- local checkout size should not grow without limit just because the project is large

## Sources

- Cursor Worktrees: https://cursor.com/docs/configuration/worktrees
- Cursor Cloud Agents: https://cursor.com/docs/cloud-agent
- Zed Git docs: https://zed.dev/docs/git
- Zed Parallel Agents: https://zed.dev/docs/ai/parallel-agents
- Zed DeltaDB blog: https://zed.dev/blog/introducing-deltadb
- Jujutsu GitHub: https://github.com/jj-vcs/jj
- Jujutsu docs: https://docs.jj-vcs.dev/latest/
- Sapling docs: https://sapling-scm.com/docs/introduction/
- Sapling GitHub: https://github.com/facebook/sapling
- GitButler GitHub: https://github.com/gitbutlerapp/gitbutler
- GitButler virtual branches: https://blog.gitbutler.com/building-virtual-branches
- Pijul: https://pijul.org/
- Darcs: https://darcs.net/
- Fossil: https://fossil-scm.org/
- Mercurial: https://www.mercurial-scm.org/
- Radicle: https://radicle.dev/
- Unity Version Control: https://unity.com/features/version-control
- Diversion: https://www.diversion.dev/
- Graphite stacked diffs: https://graphite.com/guides/stacked-diffs
- Stacking workflow: https://www.stacking.dev/
- Dolt: https://github.com/dolthub/dolt

## Current Recommendation

For our project, the strongest path is not "Git with a nicer UI."

It is:

- Rust local engine
- hosted service with first-class workspaces, changes, lines, and protected material
- Git import/export for adoption
- operation log for undo and agent traceability
- environment profiles for runnable workspace creation
- permission-aware sync and materialization from the beginning

That direction borrows the best ideas from current systems without inheriting Git's main limitations as permanent product constraints.
