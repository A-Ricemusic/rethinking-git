# Automation interface

`rgit status --json [--as ACTOR]` writes one JSON document followed by a newline.
Check the exit status before parsing stdout. Successful inspection returns `0`;
refusal/repository errors return `1` with diagnostics on stderr, and invalid CLI
syntax returns `2`. Errors do not produce a success-shaped JSON document. Other
commands currently retain text output.

A status document has `schema_version: 1` and `command: "status"`:

```json
{
  "schema_version": 1,
  "command": "status",
  "actor": "public",
  "change": {"id": "chg_0123456789abcdef0123456789abcdef", "name": "example"},
  "base_snapshot": {"state": "restricted"},
  "changes": {
    "added": ["new file.txt"],
    "modified": ["src/main.rs"],
    "deleted": [],
    "hidden_count": 1
  }
}
```

`base_snapshot` describes the snapshot used for comparison, including an inherited
line head when the change has no snapshots of its own. Its state is `absent`,
`restricted`, or `visible`; only `visible` includes an `id` field. Restricted
snapshot IDs/messages and hidden filenames are omitted. `hidden_count` counts
changed hidden paths, using the same policy-aware comparison as text status; it
is not a list of those paths or the total number of hidden files.

Without a current change, `change` and `changes` are `null` and `base_snapshot` is
`{"state":"absent"}`. This is a successful empty-workspace inspection, not a claim
that the directory has no files. For a readable current change, the three path
arrays and `hidden_count` are always present. A fully clean comparison has empty
arrays and a zero hidden count. Paths are JSON strings: use a JSON parser to retain
spaces, backslashes, quotes, Unicode and embedded newlines exactly.

Clients should reject unsupported schema versions and ignore unknown additive
fields within version 1. Breaking field or meaning changes require a new schema
version. The default text output is intended for people and is not a parsing API.
Selecting an actor remains a local view choice, not authenticated authorization.
