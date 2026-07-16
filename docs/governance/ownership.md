# Ownership model

## Principles

Every durable format, security boundary, and release gate has exactly one accountable
role. Accountability means making or escalating the decision, maintaining its evidence,
and owning remediation; it does not mean doing all implementation work. Contributors
may hold several roles, but a review that requires independence must name a different
person or organization.

This repository currently documents one maintainer and no consenting named assignees.
The current maintainer acts in all roles so routine work is not blocked, but all formal
assignments below are **Unassigned**. This is intentional: the project must not invent
people, imply consent, or represent an automated agent as a human approver.

## Accountable roles

| Role | Accountable scope | Formal assignee | Current operating state |
| --- | --- | --- | --- |
| Project maintainer | Product model, roadmap, milestone gate chair, releases | Unassigned | Current maintainer acting |
| Storage owner | Canonical objects, persistence, recovery, migrations | Unassigned | Current maintainer acting |
| Graph owner | Change/snapshot graph, merge, conflicts, landing | Unassigned | Current maintainer acting |
| Security owner | Threat model, identity, policy, cryptography, secrets | Unassigned | Current maintainer acting; cannot self-satisfy independent review |
| Workspace owner | Sessions, materialization, watchers, platform backends | Unassigned | Current maintainer acting |
| Sync owner | Protocol, remote service, authorization-before-discovery | Unassigned | Current maintainer acting |
| Git compatibility owner | Import, export, mirroring, projection safety | Unassigned | Current maintainer acting |
| CLI owner | Commands, structured output, diagnostics, accessibility | Unassigned | Current maintainer acting |
| Reliability owner | CI, test strategy, fault injection, performance, operations | Unassigned | Current maintainer acting |

The roles match the implementation plan's required storage, graph, security, workspace,
sync, Git compatibility, CLI, and reliability ownership. A team may add contributors or
delegates without creating new accountable roles.

## Assignment rules

An assignment is valid only when a repository change records the person's name or
stable organization identity, the role, effective date, and an explicit acknowledgement
from that assignee. The project maintainer approves routine assignments. The security
owner and project maintainer jointly approve changes to security accountability once
both roles are assigned.

Departing owners record a handoff or mark the role unassigned. An unassigned role blocks
only the gate that needs its approval; it does not block research, prototypes, tests, or
reversible implementation. A role may delegate reviews, but the accountable assignee
must still sign the milestone record.

## Separation and external review

Routine milestones may be signed by one person holding several accountable roles, as
long as the record discloses that fact. The following gates cannot be self-approved:

- object identity and cryptographic design before Milestone 4 closes;
- sync authorization and key distribution before an external remote beta;
- workspace isolation and secret materialization before an agent-session beta;
- full client/server security assessment before 1.0.

For those gates, the independent reviewer must not have authored the reviewed design or
implementation. An external review must identify the reviewer or organization, scope,
date, report location, unresolved findings, and accepted-risk authority. Automated
review agents can find defects and provide repeatable evidence, but do not satisfy this
human independence requirement.

## Decision responsibility

- The project maintainer accepts or rejects product-model ADRs and milestone scope.
- The relevant accountable owner accepts implementation ADRs in their scope.
- The security owner must co-approve decisions that change confidentiality,
  authorization, key handling, secret materialization, or metadata exposure.
- The reliability owner verifies test, performance, recovery, and CI evidence; they do
  not waive a failed acceptance criterion.
- Only the project maintainer declares a milestone accepted after all required owner
  approvals and independent reviews are present.

No role may approve an unmet criterion by editing the checklist. Exceptions are explicit
accepted risks in the milestone record, with an owner, expiry/revisit trigger, and a
linked follow-up issue.
