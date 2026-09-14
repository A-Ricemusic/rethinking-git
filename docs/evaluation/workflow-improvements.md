# Follow-up to the React VCS trial

The September 12 [trial](react-vcs-trial.md) remains a historical report. Its two
file/directory refusals and automation friction now have regression coverage and
implemented fixes; this follow-up describes the resulting behavior.

| Trial concern | Implemented improvement | Evidence |
| --- | --- | --- |
| Logical and on-disk baselines diverged | Checkout uses the materialized snapshot to establish path ownership; status reports both references. `workspace start` atomically creates and checks out a change. | `tests/workspace_checkout.rs`: repeated file/directory restores and metadata-only change creation need no intervening snapshots; dirty starts remain atomic refusals. |
| IDs required terminal parsing/internal reads | `--output json` emits versioned outcomes and typed records, including IDs and committed conflicts. | `tests/json_api.rs`: complete workflow obtains IDs only from the CLI, tests private metadata redaction, and resolves conflicts after a nonzero outcome. |
| Parallel agents lacked independent native directories | `worktree add`, `list`, and `detach` share history with independent change/materialization pointers. | `tests/native_worktrees.rs`: concurrent snapshots, integration from two directories, active-change exclusivity, interruption recovery, and backup independence. |
| Guides understated implemented behavior | Working, automation, prototype, and worktree guides describe the current commands and their limits. | Examples distinguish metadata-only `change new`, atomic `workspace start`, and linked `worktree add`. |

## Engineering assessment

These changes make agent-driven use substantially easier to orchestrate: IDs come
from records, failed commands cannot accidentally advertise buffered success, and
agents can edit separate directories while sharing saved history. The baseline fix
removes the two unnecessary snapshot workarounds while retaining dirty-file and
untracked-file protection. This is a qualitative workflow improvement, not a
measured claim that commands outperform Git.

There are still costs. Agents must distinguish saved change ancestry from the
materialized files and remember that integrating a line does not check it out.
Native worktrees share one command lock, so writes serialize. They require the
primary control store and fixed paths; relocation is not implemented. Explicit pruning now retires detached or missing
registrations and enables path reuse without deleting history. Text inside JSON is bounded, so clients must use typed
records and respect truncation when handling patches. JSON syntax errors and
process interruption require ordinary process-level error handling.

For controlled evaluation, the native workflow is now more practical for multiple
agents. For professional repositories, Git remains the qualified default until
this project demonstrates broader durability, scaling, interoperability, and
release qualification. Local actor selection still is not authenticated access
control. These changes address the specific trial concerns; they do not close the
[production readiness audit](../production-readiness.md).
