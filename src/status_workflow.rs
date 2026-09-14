//! Opt-in, offline workflow assessment. Unknown/restricted history never implies readiness.
use super::*;

fn history(
    repo: &Repo,
    actor: &Actor,
    roots: impl Iterator<Item = String>,
) -> Result<Option<BTreeMap<String, Snapshot>>> {
    let mut pending: Vec<_> = roots.collect();
    let mut graph = BTreeMap::new();
    while let Some(id) = pending.pop() {
        if graph.contains_key(&id) {
            continue;
        }
        let snapshot = read_snapshot(repo, &id)?;
        let change = read_change(repo, &snapshot.change_id)?;
        if !can_access(actor, &snapshot.policy) || !can_access(actor, &change.policy) {
            return Ok(None);
        }
        pending.extend(ancestry::parents(&snapshot).cloned());
        graph.insert(id, snapshot);
    }
    ancestry::validate_graph(&graph)?;
    Ok(Some(graph))
}

fn contains(graph: &BTreeMap<String, Snapshot>, root: &str, needle: &str) -> bool {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(id) = pending.pop() {
        if id == needle {
            return true;
        }
        if seen.insert(id) {
            pending.extend(ancestry::parents(&graph[id]).map(String::as_str));
        }
    }
    false
}

pub(super) fn report(
    repo: &Repo,
    actor: &Actor,
    change: Option<&Change>,
    diff: Option<&FileDiff>,
) -> Result<serde_json::Value> {
    let dirty = diff.map(|diff| {
        !diff.added.is_empty()
            || !diff.modified.is_empty()
            || !diff.deleted.is_empty()
            || diff.hidden > 0
    });
    let Some(change) = change else {
        return Ok(
            serde_json::json!({"saved_work":"no_change", "workspace_has_changes":dirty, "next_actions":["start_change"]}),
        );
    };
    let line = read_line(repo, &change.target_line)?;
    if !can_access(actor, &change.policy) || !can_access(actor, &line.policy) {
        return Ok(
            serde_json::json!({"saved_work":"restricted", "workspace_has_changes":dirty, "next_actions":["inspect_permissions"]}),
        );
    }
    let roots = change
        .current_snapshot
        .iter()
        .chain(line.head_snapshot.iter())
        .cloned();
    let graph = history(repo, actor, roots)?;
    let saved_work = match (&graph, &change.current_snapshot, &line.head_snapshot) {
        (None, _, _) => "restricted",
        (_, None, _) => "none",
        (_, Some(_), None) => "unintegrated",
        (Some(graph), Some(saved), Some(head)) => {
            if contains(graph, head, saved) {
                "integrated"
            } else if contains(graph, saved, head) {
                "unintegrated"
            } else {
                "diverged"
            }
        }
    };
    // Compare the captured/materialized tree, not IDs: an integration can create
    // another snapshot ID with identical file contents. Live edits are separate.
    let materialized = read_workspace(repo)?.mode_snapshot;
    let matches_line = match (&materialized, &line.head_snapshot) {
        (Some(materialized), Some(head)) if graph.is_some() => {
            let left = read_snapshot(repo, materialized)?;
            let right = read_snapshot(repo, head)?;
            if can_access(actor, &left.policy)
                && can_access(actor, &read_change(repo, &left.change_id)?.policy)
                && can_access(actor, &right.policy)
                && left
                    .files
                    .iter()
                    .chain(&right.files)
                    .all(|file| can_access(actor, &file.policy))
            {
                let by_path = |files: Vec<FileEntry>| {
                    files
                        .into_iter()
                        .map(|file| (file.path.clone(), file))
                        .collect::<BTreeMap<_, _>>()
                };
                Some(by_path(left.files) == by_path(right.files))
            } else {
                None
            }
        }
        _ => None,
    };
    let mut actions = Vec::new();
    if dirty == Some(true) {
        actions.push("snapshot");
    }
    match saved_work {
        "restricted" => actions.push("inspect_permissions"),
        "unintegrated" => actions.push("integrate"),
        "diverged" => actions.push("review_integration"),
        _ => {}
    }
    if matches_line == Some(false) {
        actions.push("inspect_line");
    }
    if actions.is_empty() {
        actions.push(if saved_work == "integrated" {
            "preview_push"
        } else {
            "edit"
        });
    }
    Ok(serde_json::json!({
        "target_line":line.name, "saved_work":saved_work,
        "workspace_has_changes":dirty,
        "materialized_matches_line":matches_line,
        "next_actions":actions,
    }))
}
