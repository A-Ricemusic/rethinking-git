use super::*;
use std::{
    io::Write,
    process::{Command as ProcessCommand, Stdio},
};

fn git(root: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new("git");
    command.current_dir(root);
    // Ambient Git repository variables must never redirect export writes.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command.env("GIT_CONFIG_NOSYSTEM", "1").env(
        "GIT_CONFIG_GLOBAL",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    command
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1");
    command
}

fn run(root: &Path, args: &[&str]) -> Result<()> {
    let output = git(root)
        .args(args)
        .output()
        .context("failed to run Git; install Git to use interoperability commands")?;
    if !output.status.success() {
        bail!(
            "Git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn ordered_history(repo: &Repo, head: &str) -> Result<Vec<Snapshot>> {
    let mut graph = BTreeMap::new();
    let mut pending = vec![head.to_string()];
    while let Some(id) = pending.pop() {
        if graph.contains_key(&id) {
            continue;
        }
        let snapshot = read_snapshot(repo, &id)?;
        pending.extend(ancestry::parents(&snapshot).cloned());
        graph.insert(id, snapshot);
    }
    ancestry::validate_graph(&graph)?;
    let mut order = Vec::new();
    let mut visited = BTreeSet::new();
    let mut stack = vec![(head.to_string(), false)];
    while let Some((id, leaving)) = stack.pop() {
        if leaving {
            order.push(graph[&id].clone());
            continue;
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        stack.push((id.clone(), true));
        for parent in ancestry::parents(&graph[&id]) {
            stack.push((parent.clone(), false));
        }
    }
    Ok(order)
}

pub(super) fn export(
    repo: &Repo,
    destination: &Path,
    line_name: &str,
    author: Option<&str>,
    actor: &str,
    allow_restricted: bool,
    quiet: bool,
) -> Result<()> {
    verify::check(repo, actor)?;
    if let Some(author) = author {
        validate_author(author)?;
    }
    let line = read_line(repo, line_name)?;
    let head = line
        .head_snapshot
        .as_deref()
        .context("line has no saved history")?;
    let history = ordered_history(repo, head)?;
    if author.is_none() && history.iter().any(|snapshot| snapshot.git.is_none()) {
        bail!("native snapshots require --author 'Name <email>'");
    }
    let formats: BTreeSet<_> = history
        .iter()
        .filter_map(|s| s.git.as_ref().map(|m| m.object_format.as_str()))
        .collect();
    if formats.len() > 1 {
        bail!("cannot preserve mixed Git object formats in one export");
    }
    let object_format = formats.first().copied().unwrap_or("sha1");
    let mut restricted = line.policy != public_policy();
    for snapshot in &history {
        restricted |= snapshot.policy != public_policy()
            || read_change(repo, &snapshot.change_id)?.policy != public_policy()
            || snapshot
                .files
                .iter()
                .any(|file| file.policy != public_policy());
        if snapshot.message.contains('\0') {
            bail!("Git commit messages cannot contain NUL");
        }
    }
    if restricted && !allow_restricted {
        bail!("history contains restricted data; exporting without access policies requires --allow-restricted");
    }
    let reference = format!("refs/heads/{line_name}");
    run(&repo.root, &["check-ref-format", &reference])?;
    let absolute = std::env::current_dir()?.join(destination);
    let parent = fs::canonicalize(
        absolute
            .parent()
            .context("export destination has no parent")?,
    )?;
    let destination = parent.join(
        absolute
            .file_name()
            .context("export needs a new directory name")?,
    );
    if destination.starts_with(&repo.root) {
        bail!("Git export must be outside the source working tree");
    }
    fs::create_dir(&destination).context("Git export destination must not already exist")?;
    let result = (|| -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(0o700))?;
        }
        fs::write(
            destination.join("RGIT_EXPORT_INCOMPLETE"),
            b"Do not use until export succeeds.\n",
        )?;
        run(
            &destination,
            &[
                "init",
                "--bare",
                "--template=",
                &format!("--object-format={object_format}"),
                ".",
            ],
        )?;
        let bindings = write_history(
            &destination,
            repo,
            &history,
            &reference,
            author,
            object_format,
        )?;
        run(&destination, &["symbolic-ref", "HEAD", &reference])?;
        run(&destination, &["fsck", "--full", "--strict"])?;
        for snapshot in bindings {
            write_json(repo, &snapshot_path(repo, &snapshot.id)?, &snapshot)?;
            record_operation(
                repo,
                OperationKind::BindGitIdentity {
                    snapshot_id: snapshot.id.clone(),
                    object_id: snapshot
                        .git
                        .as_ref()
                        .context("missing export provenance")?
                        .object_id
                        .clone(),
                },
                admin_policy(),
                format!("bound Git identity for `{}`", snapshot.id),
                None,
            )?;
        }
        fs::remove_file(destination.join("RGIT_EXPORT_INCOMPLETE"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result?;
    if !quiet {
        println!(
            "exported {} snapshots to Git at {}",
            history.len(),
            destination.display()
        );
    }
    Ok(())
}

fn validate_author(author: &str) -> Result<()> {
    let Some((name, email)) = author.rsplit_once(" <") else {
        bail!("author must be Name <email>");
    };
    if name.is_empty()
        || !email.ends_with('>')
        || !email.contains('@')
        || author.chars().any(|c| c.is_control())
        || name.contains(['<', '>'])
        || email[..email.len() - 1].contains(['<', '>'])
    {
        bail!("invalid export author identity");
    }
    Ok(())
}

fn output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let result = git(root).args(args).output().context("failed to run Git")?;
    if !result.status.success() {
        bail!(
            "Git command failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(result.stdout)
}

fn input(root: &Path, args: &[&str], bytes: &[u8]) -> Result<Vec<u8>> {
    let mut child = git(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().context("Git input pipe unavailable")?;
    let written = stdin.write_all(bytes);
    drop(stdin);
    let result = child.wait_with_output()?;
    written?;
    if !result.status.success() {
        bail!(
            "Git rejected object: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    Ok(result.stdout)
}

fn write_object(destination: &Path, kind: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let actual = input(
        destination,
        &["hash-object", "-t", kind, "-w", "--stdin"],
        bytes,
    )?;
    if std::str::from_utf8(&actual)?.trim() != expected {
        bail!("Git object digest differs from expected value");
    }
    Ok(())
}

fn write_history(
    destination: &Path,
    repo: &Repo,
    history: &[Snapshot],
    reference: &str,
    author: Option<&str>,
    format: &str,
) -> Result<Vec<Snapshot>> {
    let mut commits = BTreeMap::new();
    let mut bindings = Vec::new();
    let mut written = BTreeSet::new();
    for snapshot in history {
        let (tree, objects) = git_objects::tree(repo, &snapshot.files, format)?;
        for object in objects {
            if written.insert(object.id.clone()) {
                write_object(destination, object.kind, &object.bytes, &object.id)?;
            }
        }
        let parents: Vec<String> = ancestry::parents(snapshot)
            .map(|id| commits.get(id).cloned().context("export parent missing"))
            .collect::<Result<_>>()?;
        let bytes = if let Some(metadata) = &snapshot.git {
            let original = metadata.parse()?;
            if original.tree != tree || original.parents != parents {
                bail!("imported Git provenance no longer matches snapshot");
            }
            metadata.raw_commit.clone()
        } else {
            let author = author.context("native export requires author")?;
            let mut header = format!("tree {tree}\n");
            for parent in &parents {
                header.push_str(&format!("parent {parent}\n"));
            }
            header.push_str(&format!(
                "author {author} {} +0000\ncommitter {author} {} +0000\nrgit-snapshot {}\n\n",
                snapshot.created_at / 1000,
                snapshot.created_at / 1000,
                snapshot.id
            ));
            header.push_str(&snapshot.message);
            header.into_bytes()
        };
        let id = git_objects::object_id("commit", &bytes, format)?;
        if let Some(metadata) = &snapshot.git {
            if metadata.object_id != id {
                bail!("imported Git identity changed");
            }
        }
        write_object(destination, "commit", &bytes, &id)?;
        if snapshot.git.is_none() {
            let mut bound = snapshot.clone();
            bound.git = Some(git_objects::CommitMetadata {
                object_id: id.clone(),
                object_format: format.to_string(),
                raw_commit: bytes,
            });
            bindings.push(bound);
        }
        commits.insert(snapshot.id.clone(), id);
    }
    let head = history.last().context("empty export history")?;
    run(destination, &["update-ref", reference, &commits[&head.id]])?;
    Ok(bindings)
}

pub(super) fn import(
    repo: &Repo,
    source: &Path,
    revision: &str,
    into: &str,
    actor_name: &str,
    policy: AccessPolicy,
) -> Result<()> {
    import_history(repo, source, revision, into, actor_name, policy, false)
}

pub(super) fn import_history(
    repo: &Repo,
    source: &Path,
    revision: &str,
    into: &str,
    actor_name: &str,
    policy: AccessPolicy,
    allow_update: bool,
) -> Result<()> {
    verify::check(repo, actor_name)?;
    validate_named_key(into)?;
    let source = fs::canonicalize(source).context("Git import requires a local repository path")?;
    run(&source, &["fsck", "--full", "--strict"])?;
    let format = String::from_utf8(output(&source, &["rev-parse", "--show-object-format"])?)?
        .trim()
        .to_string();
    if !matches!(format.as_str(), "sha1" | "sha256") {
        bail!("unsupported Git object format");
    }
    let tip = String::from_utf8(output(
        &source,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?)?
    .trim()
    .to_string();
    git_objects::parse_id(tip.as_bytes(), &format)?;
    let path = line_path(repo, into)?;
    let mut line = if path.try_exists()? {
        read_line(repo, into)?
    } else {
        Line {
            name: into.to_string(),
            head_snapshot: None,
            policy: policy.clone(),
            created_at: now()?,
        }
    };
    if line.head_snapshot.is_some() && !allow_update {
        bail!("Git import target line must be empty");
    }
    let revisions = String::from_utf8(output(
        &source,
        &["rev-list", "--reverse", "--topo-order", &tip],
    )?)?;
    if let Some(previous) = &line.head_snapshot {
        let previous = read_snapshot(repo, previous)?;
        let provenance = previous
            .git
            .context("fetch target contains native work; use a separate tracking line")?;
        if provenance.object_format != format
            || !revisions.lines().any(|id| id == provenance.object_id)
        {
            bail!("remote branch is not a fast-forward of the tracking line; fetch into a new line to inspect it");
        }
    }
    let mut mapped = BTreeMap::new();
    for snapshot in read_dir_json::<Snapshot>(repo, &repo.path(&["snapshots"]))? {
        if let Some(metadata) = &snapshot.git {
            if metadata.object_format == format
                && mapped
                    .insert(metadata.object_id.clone(), snapshot.id)
                    .is_some()
            {
                bail!("duplicate Git provenance requires reconciliation before import");
            }
        }
    }
    let mut imported = 0;
    let mut tip_change = None;
    for oid in revisions.lines() {
        git_objects::parse_id(oid.as_bytes(), &format)?;
        if let Some(id) = mapped.get(oid) {
            if oid == tip {
                tip_change = Some(read_snapshot(repo, id)?.change_id);
            }
            continue;
        }
        let metadata = git_objects::CommitMetadata {
            object_id: oid.to_string(),
            object_format: format.clone(),
            raw_commit: output(&source, &["cat-file", "commit", oid])?,
        };
        let parsed = metadata.parse()?;
        let parents: Vec<String> = parsed
            .parents
            .iter()
            .map(|id| {
                mapped
                    .get(id)
                    .cloned()
                    .context("Git history is incomplete or shallow")
            })
            .collect::<Result<_>>()?;
        // Reconcile an acknowledged/ambiguous native push without inventing a second history.
        if let Some(id) = &parsed.native_snapshot {
            if let Ok(mut candidate) = read_snapshot(repo, id) {
                if candidate.git.is_none()
                    && ancestry::parents(&candidate).cloned().collect::<Vec<_>>() == parents
                    && candidate.message.as_bytes() == parsed.message
                    && candidate.created_at / 1000 == parsed.timestamp
                {
                    let (tree, _) = git_objects::tree(repo, &candidate.files, &format)?;
                    if tree == parsed.tree {
                        candidate.git = Some(metadata.clone());
                        write_json(repo, &snapshot_path(repo, id)?, &candidate)?;
                        record_operation(
                            repo,
                            OperationKind::BindGitIdentity {
                                snapshot_id: id.clone(),
                                object_id: oid.to_string(),
                            },
                            admin_policy(),
                            format!("reconciled Git identity for `{id}`"),
                            None,
                        )?;
                        mapped.insert(oid.to_string(), id.clone());
                        if oid == tip {
                            tip_change = Some(candidate.change_id);
                        }
                        continue;
                    }
                }
            }
        }
        let files = import_files(repo, &source, oid, &format, &policy)?;
        let (tree, _) = git_objects::tree(repo, &files, &format)?;
        if tree != parsed.tree {
            bail!("Git tree cannot be represented without changing its identity");
        }
        let change_id = format!("chg_{}", new_id_suffix());
        let snapshot_id = format!("snap_{}", new_id_suffix());
        let message = String::from_utf8_lossy(&parsed.message).into_owned();
        let change = Change {
            id: change_id.clone(),
            name: message.lines().next().unwrap_or("Git import").to_string(),
            base_snapshot: parents.first().cloned(),
            target_line: into.to_string(),
            current_snapshot: Some(snapshot_id.clone()),
            policy: policy.clone(),
            created_at: parsed
                .timestamp
                .checked_mul(1000)
                .context("Git timestamp exceeds native range")?,
        };
        let snapshot = Snapshot {
            git: Some(metadata),
            id: snapshot_id.clone(),
            change_id: change_id.clone(),
            parent_snapshot: parents.first().cloned(),
            merge_parents: parents.into_iter().skip(1).collect(),
            message,
            manifest_hash: manifest_hash(&files)?,
            files,
            policy: policy.clone(),
            created_at: parsed
                .timestamp
                .checked_mul(1000)
                .context("Git timestamp exceeds native range")?,
        };
        write_json(repo, &change_path(repo, &change_id)?, &change)?;
        write_json(repo, &snapshot_path(repo, &snapshot_id)?, &snapshot)?;
        mapped.insert(oid.to_string(), snapshot_id);
        imported += 1;
        if oid == tip {
            tip_change = Some(change_id);
        }
    }
    let snapshot_id = mapped
        .get(&tip)
        .context("Git import contained no tip")?
        .clone();
    let change_id = tip_change.context("Git import tip change missing")?;
    line.head_snapshot = Some(snapshot_id.clone());
    line.policy = policy;
    write_json(repo, &line_path(repo, into)?, &line)?;
    let mut workspace = read_workspace(repo)?;
    if workspace.current_change.is_none() {
        workspace.current_change = Some(change_id.clone());
        write_json(repo, &repo.path(&["workspace.json"]), &workspace)?;
    }
    record_operation(
        repo,
        OperationKind::ImportGit {
            line: into.to_string(),
            change_id,
            snapshot_id,
        },
        admin_policy(),
        format!("imported Git revision into `{into}`"),
        None,
    )?;
    verify::check(repo, actor_name)?;
    println!(
        "imported {} Git commits into {into}; working files are unchanged",
        imported
    );
    println!("use workspace switch or workspace restore --discard-changes --as {actor_name} to materialize saved files");
    Ok(())
}

fn import_files(
    repo: &Repo,
    source: &Path,
    commit: &str,
    format: &str,
    policy: &AccessPolicy,
) -> Result<Vec<FileEntry>> {
    let listing = output(source, &["ls-tree", "-r", "-z", "--full-tree", commit])?;
    let mut files = Vec::new();
    for record in listing
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let split = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("invalid Git tree record")?;
        let fields: Vec<_> = std::str::from_utf8(&record[..split])?.split(' ').collect();
        if fields.len() != 3 || fields[1] != "blob" || !matches!(fields[0], "100644" | "100755") {
            bail!("Git import currently supports regular files only; symlinks and submodules require support before import");
        }
        let path = std::str::from_utf8(&record[split + 1..])
            .context("Git filename is not representable as UTF-8")?
            .to_string();
        transaction::validate_working_key(&path)?;
        let oid = git_objects::parse_id(fields[2].as_bytes(), format)?;
        let bytes = output(source, &["cat-file", "blob", &oid])?;
        if git_objects::object_id("blob", &bytes, format)? != oid {
            bail!("Git blob identity mismatch");
        }
        let hash = hash_bytes(&bytes);
        let destination = repo.path(&["blobs", &hash]);
        if destination.try_exists()? {
            let metadata = fs::symlink_metadata(&destination)?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || fs::read(&destination)? != bytes
            {
                bail!("stored blob failed verification");
            }
        } else {
            transaction::publish_file(&destination, &bytes)?;
        }
        files.push(FileEntry {
            path,
            executable: fields[0] == "100755",
            hash,
            bytes: bytes.len() as u64,
            policy: policy.clone(),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
