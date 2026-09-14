//! Versioned status output. Permission checks and diff calculation are shared with text mode.
use super::*;

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    command: &'static str,
    actor: &'a str,
    change: Option<ChangeInfo<'a>>,
    base_snapshot: SnapshotReference,
    materialized_snapshot: SnapshotReference,
    changes: Option<&'a FileDiff>,
}

#[derive(Serialize)]
struct ChangeInfo<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum SnapshotReference {
    Absent,
    Restricted,
    Visible { id: String },
}

pub(super) fn print(
    repo: &Repo,
    actor: &Actor,
    change: Option<&Change>,
    changes: Option<&FileDiff>,
) -> Result<()> {
    let base_snapshot = match change.and_then(Change::workspace_base_snapshot_id) {
        None => SnapshotReference::Absent,
        Some(id) => {
            let snapshot = read_snapshot(repo, id)?;
            if can_access(actor, &snapshot.policy) {
                SnapshotReference::Visible { id: snapshot.id }
            } else {
                SnapshotReference::Restricted
            }
        }
    };
    let materialized_snapshot = match read_workspace(repo)?.mode_snapshot {
        None => SnapshotReference::Absent,
        Some(id) => {
            let snapshot = read_snapshot(repo, &id)?;
            if can_access(actor, &snapshot.policy) {
                SnapshotReference::Visible { id }
            } else {
                SnapshotReference::Restricted
            }
        }
    };
    let report = Report {
        schema_version: 1,
        command: "status",
        actor: &actor.name,
        change: change.map(|change| ChangeInfo {
            id: &change.id,
            name: &change.name,
        }),
        base_snapshot,
        materialized_snapshot,
        changes,
    };
    output::record("status", serde_json::to_value(&report)?);
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}
