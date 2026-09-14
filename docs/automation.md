# Automation interface

`rgit status --json [--as ACTOR]` writes one JSON document followed by a newline.
Check the exit status before parsing stdout. Successful inspection returns `0`;
refusal/repository errors return `1` with diagnostics on stderr, and invalid CLI
syntax returns `2`. Errors do not produce a success-shaped JSON document. For other commands, use the all-command JSON outcome described below.

A status document has `schema_version: 1` and `command: "status"`:

```json
{
  "schema_version": 1,
  "command": "status",
  "actor": "public",
  "change": {"id": "chg_0123456789abcdef0123456789abcdef", "name": "example"},
  "base_snapshot": {"state": "restricted"},
  "materialized_snapshot": {"state": "restricted"},
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

## Checkout baseline

Status version 1 also includes `materialized_snapshot`, using the same
`absent`/`restricted`/`visible` reference representation as `base_snapshot`.
`base_snapshot` retains its original comparison meaning. `materialized_snapshot`
identifies the last capture or checkout used to decide ownership of working paths;
it can differ after an explicit restore or metadata-only `change new`. An absent
materialization reference also covers legacy workspaces that did not record one;
it is not proof that the directory is empty. Neither field exposes a restricted ID.
Use `workspace start NAME --target LINE` for an atomic fresh-change checkout.

## All-command JSON outcomes

Use `rgit --output json COMMAND ...` (or append `--output json`) for a single
versioned outcome on stdout. Text remains the default. `status --json` retains the
status-only document above; with `--output json`, that document is a `status`
record inside the outcome.

```json
{
  "schema_version": 1,
  "command": "workspace start",
  "ok": true,
  "exit_code": 0,
  "records": [
    {"kind": "change_created", "data": {"id": "chg_example", "name": "task", "target_line": "main"}},
    {"kind": "workspace", "data": {"change_id": "chg_example", "materialized_snapshot": null}}
  ],
  "text": "human-readable command output\n",
  "text_truncated": false,
  "error": null
}
```

Read `records` by `kind`, not by array position. Creation commands return IDs in
`change_created`, `snapshot_created`, and `conflict` records, so automation does
not need to read internal metadata files. Inspection records include `repository`,
`actor`, `path_policy`, `identity`, `change`, `snapshot`, `file`, `line`, `workspace`,
`status`, `diff`, `merge_preview`, `operation`, and `public_operation`. Mutation
outcomes include `integration`, `conflict_resolved`, `line_created`, `line_reset`,
`change_retargeted`, `verification`, `backup`, `git_import`, `git_export`,
`git_clone`, `git_pull`, and `git_push`. Empty results may have no records.
Records follow the same actor filtering as the corresponding text view; this is
still a local permissioned view, not an authentication boundary.

A command refusal returns exit code 1 and `ok: false`, with an `error` object
containing `kind` and `message`. Kinds are `unavailable`, `command_failed`, or
`conflicts`. Ordinary failures clear buffered records and text, so an ID printed
before a failed transaction is not advertised as success. A conflicted integration
commits conflict records and returns `conflicts` with the visible records intact;
resolve those IDs and retry integration. A publication error may leave a committed
recovery journal: inspect the error, recover with another command, and inspect
state before retrying a mutation. Errors also retain diagnostics on stderr.

CLI syntax errors, help, and version output are handled before command execution
and do not use this envelope. Syntax errors exit 2. `rgit --version` reports the
package version. Fatal process interruption or a broken stdout pipe cannot promise
an envelope; check the process result and parse defensively.

`text` is optional human context for clients and must not be parsed as an API. It
is capped at 1 MiB on a UTF-8 boundary; `text_truncated` indicates omitted text.
This applies to unified patches too: never apply a truncated patch. Typed records
are retained independently of this cap. Clients must check `schema_version`,
ignore unknown additive fields/record kinds, and handle nonzero exits before
using success records.

Filesystem destination fields are exact UTF-8 strings, or `null` when an operating
system path cannot be represented as UTF-8. Companion `destination_display` (or
`path_display` for worktrees) is for display only and may replace invalid bytes.
Do not use a display value as a filesystem identifier. Native linked registrations
require UTF-8 roots and refuse unsupported paths before creating the destination.
