# Rethinking Git

An experimental source-control tool built around stable **changes**, saved
**snapshots**, a working **workspace**, shared **lines**, and an **operation log**.
Snapshots are captured explicitly with `rgit snapshot`; there is no background watcher.

The CLI now supports a local edit/snapshot/integrate workflow, workspace restore and
switch, native linked worktrees, conflict resolution, verified backups, and selected-branch Git import,
export, clone, fetch, pull and push. Git interoperability and transport require installed Git.
[Git collaboration](docs/git-collaboration.md) supports saved remotes/upstreams,
`rgit push` / `rgit pull`, and bounded previews with `push --dry-run`.
Agents can inspect local integration state with `status --workflow` and guard
publication against concurrent line changes with `push --expect-snapshot`.

**This remains experimental and is not qualified as a production Git replacement.**
Local actor filtering is not authentication or encryption. The executable uses its
format-2 compatibility store; the newer canonical object/storage libraries are separate.
See the [readiness audit](docs/production-readiness.md) for remaining release blockers.

## Try it

With Rust 1.85 or newer, from this checkout:

```sh
cargo install --path . --locked
rgit --help
```

The [working CLI guide](docs/getting-started.md) walks through saving a change,
restoring a backup, and collaborating through Git. Use a disposable repository for
evaluation and keep the original Git repository when trying an import.

Commands support `--output json` for [automation](docs/automation.md).
Use [native worktrees](docs/worktrees.md) to give concurrent tasks separate working
directories with shared history.

## Repository map

| Location | Responsibility |
| --- | --- |
| `src/` | CLI, JSON compatibility records, command journal, working files, Git bridge and transport |
| `crates/rgit-objects/` | Canonical object encoding, identifiers, schemas and reference roles |
| `crates/rgit-graph/` | Pure graph traversal, manifest diffs and merge planning |
| `crates/rgit-store/` | Verified object storage, atomic reference publication and SQLite metadata |
| `tests/` | Executable workflows, refusal/corruption cases and Git round trips |
| `spec/`, `docs/adr/` | Durable format specifications and architecture decisions |

The [prototype reference](docs/prototype.md) describes current behavior and limits.
[Primitives](docs/jj-primitives.md) and [access-control design](docs/access-control.md)
explain the model; design documents may describe capabilities beyond the current CLI.

Run `cargo test --workspace --locked` for the test suite. CI also checks formatting,
strict Clippy, the minimum Rust version and dependency policy on the supported matrix.

## License

Dual-licensed under the [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT License](LICENSE-MIT), at your option.
