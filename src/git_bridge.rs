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
    author: &str,
    actor: &str,
    allow_restricted: bool,
) -> Result<()> {
    verify::verify(repo, actor)?;
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
    let line = read_line(repo, line_name)?;
    let head = line
        .head_snapshot
        .as_deref()
        .context("line has no saved history")?;
    let history = ordered_history(repo, head)?;
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
        run(&destination, &["init", "--bare", "--template=", "."])?;
        let mut child = git(&destination)
            .args(["fast-import", "--quiet"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().context("Git input pipe unavailable")?;
        let written = write_history(&mut stdin, repo, &history, &reference, author);
        drop(stdin);
        let output = child.wait_with_output()?;
        written?;
        if !output.status.success() {
            bail!(
                "Git import rejected export: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        run(&destination, &["symbolic-ref", "HEAD", &reference])?;
        run(&destination, &["fsck", "--full", "--strict"])?;
        fs::remove_file(destination.join("RGIT_EXPORT_INCOMPLETE"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&destination);
    }
    result?;
    println!(
        "exported {} snapshots to Git at {}",
        history.len(),
        destination.display()
    );
    Ok(())
}

fn write_history(
    output: &mut impl Write,
    repo: &Repo,
    history: &[Snapshot],
    reference: &str,
    author: &str,
) -> Result<()> {
    writeln!(output, "feature done")?;
    let mut marks = BTreeMap::new();
    for (index, snapshot) in history.iter().enumerate() {
        let mark = index + 1;
        writeln!(output, "reset {reference}\n")?;
        writeln!(output, "commit {reference}\nmark :{mark}\nauthor {author} {} +0000\ncommitter {author} {} +0000", snapshot.created_at, snapshot.created_at)?;
        writeln!(output, "data {}", snapshot.message.len())?;
        output.write_all(snapshot.message.as_bytes())?;
        writeln!(output)?;
        for (index, parent) in ancestry::parents(snapshot).enumerate() {
            let parent_mark = marks.get(parent).context("export parent not yet emitted")?;
            writeln!(
                output,
                "{} :{parent_mark}",
                if index == 0 { "from" } else { "merge" }
            )?;
        }
        writeln!(output, "deleteall")?;
        for file in &snapshot.files {
            let bytes = fs::read(repo.path(&["blobs", &file.hash]))?;
            if hash_bytes(&bytes) != file.hash || bytes.len() as u64 != file.bytes {
                bail!("blob changed during export");
            }
            writeln!(
                output,
                "M {} inline {}",
                if file.executable { "100755" } else { "100644" },
                quote_path(&file.path)
            )?;
            writeln!(output, "data {}", bytes.len())?;
            output.write_all(&bytes)?;
            writeln!(output)?;
        }
        writeln!(output)?;
        marks.insert(snapshot.id.clone(), mark);
    }
    writeln!(output, "done")?;
    Ok(())
}

fn quote_path(path: &str) -> String {
    let mut quoted = String::from("\"");
    for byte in path.bytes() {
        match byte {
            b'"' => quoted.push_str("\\\""),
            b'\\' => quoted.push_str("\\\\"),
            0x20..=0x7e => quoted.push(char::from(byte)),
            byte => quoted.push_str(&format!("\\{byte:03o}")),
        }
    }
    quoted.push('"');
    quoted
}
