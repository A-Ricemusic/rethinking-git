# Behavioral benchmark fixtures

These small, checked-in fixtures exercise permission and merge semantics independently of the generated performance corpus. All credentials are intentionally fake.

- `public-only`: ordinary public source files.
- `mixed-access`: public source plus a path intended for the `security` domain.
- `security-fix`: an embargoed patch description with public advisory metadata.
- `secret-reference`: a production configuration that contains only a secret-manager reference, never secret material.
- `concurrent-conflict`: base, incoming, and line-side versions of one file for merge planning.

The performance fixtures are generated instead of checked in. Run `scripts/generate-benchmark-fixture.sh --list` to inspect profiles.
