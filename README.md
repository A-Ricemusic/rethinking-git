# Rethinking Git

This repository is now centered on a `jj`-inspired model.

The first prototype starts from these ideas:

- a human works on a stable `change`
- the filesystem is continuously captured as immutable `snapshots`
- a `workspace` is the local editable view of a change
- every source-control action is recorded in an `operation log`

Later versions can add protected `lines`, typed `markers`, permissioned materialization, and sync.

Start here:

- [docs/jj-primitives.md](/Users/pelicannurse/Documents/Apps/rethinking-git/docs/jj-primitives.md)
- [docs/prototype.md](/Users/pelicannurse/Documents/Apps/rethinking-git/docs/prototype.md)
