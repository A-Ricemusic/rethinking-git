# Rethinking Git

**Status: experimental; not yet a production Git replacement.** See the [production-readiness audit](docs/production-readiness.md) for verified fixes, remaining blockers, and acceptance criteria.

This repository is now centered on a `jj`-inspired model.

The first prototype starts from these ideas:

- a human works on a stable `change`
- the filesystem is continuously captured as immutable `snapshots`
- a `workspace` is the local editable view of a change
- every source-control action is recorded in an `operation log`

Later versions can add protected `lines`, typed `markers`, permissioned materialization, and sync.

Start here:

- [docs/jj-primitives.md](docs/jj-primitives.md)
- [docs/access-control.md](docs/access-control.md)
- [docs/prototype.md](docs/prototype.md)

## License

Rethinking Git is dual-licensed under the [Apache License, Version 2.0](LICENSE-APACHE)
or the [MIT License](LICENSE-MIT), at your option.
