# Prototype

This is the first Rust prototype for the `jj`-inspired model.

It implements the first source-control primitives:

- `change`: the stable logical unit of work
- `snapshot`: an immutable capture of the repository files
- `workspace`: the current editable context
- `operation`: an append-only record of source-control actions
- `line`: a shared integration target such as `main`
- `actor`: a person or tool with domain grants
- `path policy`: file-level access control

There are no branches, tags, remotes, real encryption, or hosting yet.

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

Show the workspace diff:

```sh
cargo run -- diff workspace
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

Show a change:

```sh
cargo run -- change show chg_example
```

List snapshots:

```sh
cargo run -- snapshot-info list
```

Show a snapshot:

```sh
cargo run -- snapshot-info show snap_example
```

Show the operation log:

```sh
cargo run -- op log
```

## Permissioned Flow

Initialize the repo:

```sh
cargo run -- init
```

Create actors:

```sh
cargo run -- actor set alice --domain public
cargo run -- actor set bob --domain public --domain team/security
cargo run -- actor set admin --domain public --domain admin
```

Restrict sensitive paths:

```sh
cargo run -- access path .env --domain admin
cargo run -- access path security --domain team/security
```

Create a private security change:

```sh
cargo run -- change new fix-token-replay --domain team/security
```

Create files:

```sh
mkdir -p src security
printf 'patched auth\n' > src/auth.txt
printf 'SECRET=value\n' > .env
printf 'exploit repro\n' > security/repro.test
```

Snapshot and integrate into `main`:

```sh
cargo run -- snapshot --message "fix token replay"
cargo run -- line integrate main
```

Alice can see the shared line but not restricted files:

```sh
cargo run -- line view main --as alice
cargo run -- change list --as alice
cargo run -- op log --as alice
```

Bob can see the security material:

```sh
cargo run -- line view main --as bob
cargo run -- change list --as bob
cargo run -- snapshot-info list --as bob
cargo run -- op log --as bob
```

Admin can see everything, including `.env`:

```sh
cargo run -- line view main --as admin
```

Show line history with actor-specific redaction:

```sh
cargo run -- line history main --as alice
cargo run -- line history main --as bob
cargo run -- line history main --as admin
```

Show actor-filtered diffs:

```sh
cargo run -- diff workspace --as alice
cargo run -- diff snapshot snap_old snap_new --as bob
cargo run -- diff line main --as admin
```

Diffs currently show file-level added, modified, deleted, and hidden restricted counts. They do not show line-level text patches yet.

## Storage

The prototype stores state in `.rgit/`.

```text
.rgit/
  repo.json
  workspace.json
  path-policies.json
  actors/
  blobs/
  changes/
  lines/
  operations/
  snapshots/
```

Snapshots reference blobs by SHA-256 hash.

Changes point at their current snapshot.

The workspace points at the current change.

Operations record how state changed over time.

Actors and path policies decide which objects are visible in commands that accept `--as`.

This is not cryptographic security yet. It is the local policy and view model that real encrypted sync would enforce later.
