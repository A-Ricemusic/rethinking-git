# Milestone review and sign-off process

## Purpose

A milestone is complete when its claimed behavior is reproducible and accountable, not
when implementation merely stops. Each milestone gets a versioned review record created
from [the template](milestone-review-template.md). The record links evidence rather than
copying transient command output into the checklist.

## Review lifecycle

1. **Open the gate.** The milestone owner creates `reviews/milestone-N.md`, names the
   candidate commit, lists in-scope checklist criteria, and records applicable ADRs and
   threat-model changes. Its initial decision is `Pending`.
2. **Assemble evidence.** Every criterion links to tests, CI runs or durable artifacts,
   test-environment details, and any manual reproduction steps. Benchmarks name hardware
   and fixture versions. Security claims identify both positive and denial tests.
3. **Run implementation review.** A reviewer examines code organization, error and
   recovery behavior, tests, documentation, migrations, platform differences, and
   checklist-to-evidence traceability. Automated agents may contribute findings, but
   their output is supplemental evidence.
4. **Resolve findings.** Each finding is closed by a linked change and regression test,
   or recorded as accepted risk with severity, accountable owner, rationale, expiry or
   revisit trigger, and follow-up issue. Unresolved critical/high security findings block
   the gate. Any unmet mandatory acceptance criterion blocks the gate.
5. **Obtain approvals.** The accountable roles listed by the template sign using a
   stable identity and date. Security-sensitive gates also require the independent or
   external review specified in the [ownership model](ownership.md).
6. **Record the decision.** The project maintainer sets `Accepted` or `Rejected`, signs,
   and records the exact candidate commit. Later code changes do not inherit sign-off;
   material changes require an amended record or a new review candidate.
7. **Close tracking.** Only after acceptance are matching checklist items marked
   complete. The checklist links back to the review record or its evidence where useful.

## Required evidence

Every review record contains:

- candidate commit and repository format/protocol versions affected;
- checklist criteria mapped one-to-one to durable evidence;
- supported-platform CI results and explicitly skipped behavior;
- test commands, fixtures, and a real-directory end-to-end exercise when applicable;
- benchmark results and budgets when performance is an exit criterion;
- failure-injection, corruption, and recovery results when state can be lost;
- threat-model, authorization, metadata-leakage, and secret-handling impact;
- compatibility, migration, rollback, and data-recovery impact;
- implementation-review findings and their dispositions;
- required accountable-role and independent-review approvals;
- final decision, signer, date, and candidate commit.

Links to ephemeral CI logs are accompanied by a durable checked-in summary or artifact
digest. Secrets, restricted paths, private object identifiers, and credentials never
appear in review evidence.

## Decision states

- `Pending`: evidence or approvals are still being collected.
- `Blocked`: a named unmet criterion, finding, owner, or external dependency prevents
  acceptance.
- `Rejected`: the candidate was reviewed and will not be released as the milestone.
- `Accepted`: every mandatory criterion is evidenced and every required approval exists.
- `Superseded`: a later accepted record replaces this decision and links back to it.

Only `Accepted` satisfies “milestone review is signed off.” A role being unassigned is a
valid `Blocked` reason, never an implied approval.

## Milestone 0 gate

Milestone 0 specifically requires evidence that documented prototype flows run in CI,
golden tests protect existing semantics, benchmark baselines are reproducible, and the
threat model/trust boundaries have been reviewed. All required ADRs must be `Accepted`,
not merely drafted. The project maintainer, reliability owner, owners of affected ADRs,
and security owner sign the record. If one person holds multiple roles, each role is
listed separately and the lack of independent review is disclosed.

Because this project currently has no formal role assignees, these documents establish
the process but do not by themselves sign off Milestone 0.
