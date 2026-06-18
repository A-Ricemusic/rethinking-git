# Prototype

This is the first Rust prototype for the `jj`-inspired model.

It intentionally implements only four primitives:

- `change`: the stable logical unit of work
- `snapshot`: an immutable capture of the repository files
- `workspace`: the current editable context
- `operation`: an append-only record of source-control actions

There are no branches, tags, remotes, permissions, or hosting yet.

## Install Rust

If `cargo` or `rustc` are not available, install Rust.

Install Rust with:

```sh
brew install rustup
PATH="/opt/homebrew/opt/rustup/bin:$PATH" rustup toolchain install stable
```

Then check:

```sh
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo --version
PATH="/opt/homebrew/opt/rustup/bin:$PATH" rustc --version
```

If you want `cargo` and `rustc` available in every new shell:

```sh
echo 'export PATH="/opt/homebrew/opt/rustup/bin:$PATH"' >> ~/.zshrc
```

After restarting the shell, these should work:

```sh
cargo --version
rustc --version
```

## Build

```sh
cargo build
```

## Flow

Initialize the repo:

```sh
cargo run -- init
```

Create a logical change:

```sh
cargo run -- change new add-dark-mode-settings
```

Check what has changed since the latest snapshot:

```sh
cargo run -- status
```

Create a snapshot:

```sh
cargo run -- snapshot --message "add settings toggle"
```

Inspect the workspace:

```sh
cargo run -- workspace info
```

List changes:

```sh
cargo run -- change list
```

Show the operation log:

```sh
cargo run -- op log
```

## Storage

The prototype stores state in `.rgit/`.

```text
.rgit/
  repo.json
  workspace.json
  blobs/
  changes/
  operations/
  snapshots/
```

Snapshots reference blobs by SHA-256 hash.

Changes point at their current snapshot.

The workspace points at the current change.

Operations record how state changed over time.
