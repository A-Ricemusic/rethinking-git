use super::*;

pub(super) fn parents(snapshot: &Snapshot) -> impl Iterator<Item = &String> {
    snapshot
        .parent_snapshot
        .iter()
        .chain(snapshot.merge_parents.iter())
}

pub(super) fn validate_graph(snapshots: &BTreeMap<String, Snapshot>) -> Result<()> {
    let mut colors = BTreeMap::new();
    for root in snapshots.keys() {
        let mut stack = vec![(root.as_str(), false)];
        while let Some((id, leaving)) = stack.pop() {
            if leaving {
                colors.insert(id, 2);
                continue;
            }
            match colors.get(id) {
                Some(2) => continue,
                Some(1) => bail!("snapshot ancestry contains a cycle"),
                _ => {}
            }
            let snapshot = snapshots.get(id).context("missing snapshot parent")?;
            let unique: BTreeSet<_> = parents(snapshot).collect();
            if unique.len() != parents(snapshot).count() {
                bail!("snapshot has duplicate parents");
            }
            colors.insert(id, 1);
            stack.push((id, true));
            for parent in parents(snapshot) {
                stack.push((parent, false));
            }
        }
    }
    Ok(())
}

fn ancestors(root: &str, graph: &BTreeMap<String, Snapshot>) -> BTreeSet<String> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![root.to_string()];
    while let Some(id) = pending.pop() {
        if visited.insert(id.clone()) {
            pending.extend(parents(&graph[&id]).cloned());
        }
    }
    visited
}

pub(super) fn merge_base(
    repo: &Repo,
    line: Option<&str>,
    incoming: &Snapshot,
    legacy_base: Option<&str>,
) -> Result<Option<Snapshot>> {
    let Some(line) = line else {
        return Ok(None);
    };
    let mut graph = BTreeMap::new();
    let mut pending = vec![line.to_string(), incoming.id.clone()];
    while let Some(id) = pending.pop() {
        if graph.contains_key(&id) {
            continue;
        }
        let snapshot = read_snapshot(repo, &id)?;
        pending.extend(parents(&snapshot).cloned());
        graph.insert(id, snapshot);
    }
    validate_graph(&graph)?;
    let left = ancestors(line, &graph);
    let right = ancestors(&incoming.id, &graph);
    let common: BTreeSet<_> = left.intersection(&right).cloned().collect();
    let mut best = common.clone();
    for id in &common {
        for parent in parents(&graph[id]) {
            best.remove(parent);
        }
    }
    if best.len() > 1 {
        bail!("multiple merge bases require recursive merging; integration was not published");
    }
    if let Some(id) = best.first() {
        return Ok(Some(graph[id].clone()));
    }
    // Older format-2 snapshots omitted the first snapshot's base edge.
    read_optional_snapshot(repo, legacy_base)
}
