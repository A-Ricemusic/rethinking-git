# Native GitHub pull-request workflow

This evaluation builds and uses the project from a native checkout with no `.git`
directory. The source contains 166 Git commits and 147 tracked files at
`b2fcb724d8ac9b8c2433220e3f103162169b12f5`. The full workspace tests and release build
pass there, and `status --json` reports a clean workspace afterward.

The rebuilt `rgit` binary captures this documentation as a native snapshot,
integrates it, and pushes the saved line to a documentation branch in this GitHub
repository. Installed Git supplies HTTPS authentication and object transport.
Native operations use `.rgit` metadata and the command journal; this does not
exercise a separate native authentication service or encrypted policy backend.

The follow-up receipt records the first successful authenticated publication and
compares the remote Git commit with the identity saved by `rgit`. A second native
snapshot publishes that receipt. The PR and its CI checks provide the final
publication/review record. No Git index or `git commit` is used for these changes.

## Reproduction outline

1. Build the bootstrap binary at the recorded revision.
2. Run `rgit git clone REMOTE NEW_DIRECTORY --branch SOURCE_BRANCH --domain public`.
3. In that native directory, run `cargo test --workspace --locked` and
   `cargo build --release --locked`, then use `target/release/rgit` for later steps.
4. Check `status --json` and `repo verify --as admin`. Configure an explicit author
   with `identity set NAME EMAIL`.
5. Create a change, edit the documentation, snapshot, and integrate `main`.
6. Push with `git push REMOTE --line main --branch NEW_BRANCH --as admin` using the
   `rgit` executable. Compare the remote head with the saved Git identity.
7. Capture and push the receipt, create the GitHub PR, and run the required checks.

This is an observed development workflow, not a release certification, independent
security review, power-loss test, or proof of complete Git feature parity. The
[production audit](../production-readiness.md) retains the remaining limitations.

[Recorded receipt](native-pr-workflow-2026-09-12.json) includes the source and binary
digests, completed native phases, and the first published Git identity. Local logs
were retained separately; their digests are provenance, not an attestation.
