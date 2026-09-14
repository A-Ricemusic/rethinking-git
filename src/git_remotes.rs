use super::*;
use std::process::{Command as ProcessCommand, Stdio};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("rgit-transport-{}", Uuid::new_v4().simple()));
        fs::create_dir(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        let path = fs::canonicalize(path)?;
        fs::create_dir(path.join("no-hooks"))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn validate_remote(remote: &str) -> Result<()> {
    if remote.is_empty() || remote.starts_with('-') || remote.chars().any(char::is_control) {
        bail!("invalid Git remote");
    }
    if let Some(rest) = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("ssh://"))
    {
        let authority = rest.split('/').next().unwrap_or("");
        if authority.is_empty() || remote.contains(['?', '#']) {
            bail!("remote must use a repository URL without query credentials");
        }
        if let Some((user, _)) = authority.rsplit_once('@') {
            if remote.starts_with("https://") || user.contains(':') {
                bail!("use configured Git credentials instead of credentials embedded in a URL");
            }
        }
    } else if !remote.starts_with("file://") && (remote.contains("://") || remote.contains("::")) {
        bail!("remote protocol is unsupported; use HTTPS, SSH, or a local repository");
    }
    Ok(())
}

pub(super) fn resolve_remote(remote: &str) -> Result<String> {
    validate_remote(remote)?;
    // Git runs in private scratch directories. Resolve local paths at the caller,
    // while preserving URL and scp-style SSH syntax for Git's transport parser.
    if Path::new(remote).is_absolute()
        || !remote.contains(':')
        || remote.starts_with("./")
        || remote.starts_with("../")
    {
        let path = fs::canonicalize(remote)?;
        let path = path
            .to_str()
            .context("local Git remote path must be UTF-8")?
            .to_string();
        // Rust canonicalization uses Win32 verbatim prefixes, which Git's
        // remote parser can mistake for an scp-style host. Use Git path syntax.
        #[cfg(windows)]
        let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
            format!("//{unc}").replace('\\', "/")
        } else {
            path.strip_prefix(r"\\?\")
                .unwrap_or(&path)
                .replace('\\', "/")
        };
        return Ok(path);
    }
    Ok(remote.to_string())
}

fn transport(root: &Path, hooks: &Path) -> ProcessCommand {
    let mut command = ProcessCommand::new("git");
    command.current_dir(root);
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("GIT_")
            && !matches!(
                name.as_ref(),
                "GIT_ASKPASS"
                    | "GIT_SSH"
                    | "GIT_SSH_COMMAND"
                    | "GIT_SSH_VARIANT"
                    | "GIT_TERMINAL_PROMPT"
                    | "GIT_SSL_CAINFO"
                    | "GIT_SSL_CAPATH"
                    | "GIT_CONFIG_GLOBAL"
                    | "GIT_CONFIG_SYSTEM"
            )
        {
            command.env_remove(key);
        }
    }
    command
        .args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "protocol.ssh.allow=always",
            "-c",
            "protocol.file.allow=always",
            "-c",
            "protocol.http.allow=never",
            "-c",
            "protocol.git.allow=never",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "http.sslVerify=true",
            "-c",
        ])
        .arg(format!("core.hooksPath={}", hooks.display()))
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

fn run(root: &Path, scratch: &Scratch, args: &[&str]) -> Result<()> {
    let mut command = transport(root, &scratch.0.join("no-hooks"));
    command.args(args);
    let status = if output::enabled() {
        // Git's --porcelain push report uses stdout. Keep the JSON channel
        // exclusive to our outcome, while streaming transport diagnostics.
        let mut child = command
            .stdout(Stdio::piped())
            .spawn()
            .context("failed to run Git transport")?;
        let forwarded = {
            let mut stdout = child
                .stdout
                .take()
                .context("Git transport stdout is missing")?;
            std::io::copy(&mut stdout, &mut std::io::stderr().lock())
        };
        let status = child.wait().context("failed to wait for Git transport")?;
        forwarded.context("failed to forward Git transport diagnostics")?;
        status
    } else {
        command.status().context("failed to run Git transport")?
    };
    if !status.success() {
        bail!("Git transport failed; remote authorization or fast-forward checks may have rejected the operation");
    }
    Ok(())
}

pub(super) fn fetch(
    repo: &Repo,
    remote: &str,
    branch: &str,
    into: &str,
    actor: &str,
    policy: AccessPolicy,
) -> Result<()> {
    let imported = fetch_history(repo, remote, branch, into, actor, policy)?;
    git_bridge::report_import(imported, into, actor);
    Ok(())
}

fn fetch_history(
    repo: &Repo,
    remote: &str,
    branch: &str,
    into: &str,
    actor: &str,
    policy: AccessPolicy,
) -> Result<usize> {
    verify::check(repo, actor)?;
    let remote = git_tracking::resolve_named(repo, remote)?;
    validate_named_key(into)?;
    let scratch = Scratch::new()?;
    run(
        &scratch.0,
        &scratch,
        &["check-ref-format", &format!("refs/heads/{branch}")],
    )?;
    run(
        &scratch.0,
        &scratch,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            "--single-branch",
            "--branch",
            branch,
            "--",
            &remote,
            "repository.git",
        ],
    )?;
    git_bridge::import_history(
        repo,
        &scratch.0.join("repository.git"),
        &format!("refs/heads/{branch}"),
        into,
        actor,
        policy,
        true,
    )
}

pub(super) fn pull(repo: &Repo, args: &GitPullArgs) -> Result<()> {
    verify::check(repo, &args.as_actor)?;
    let (line, remote, branch) = git_tracking::target(
        repo,
        args.line.as_deref(),
        args.remote.as_deref(),
        args.branch.as_deref(),
    )?;
    let before = read_workspace(repo)?;
    let current = before
        .current_change
        .as_deref()
        .map(|id| read_change(repo, id))
        .transpose()?;
    if current
        .as_ref()
        .is_some_and(|change| change.target_line != line)
    {
        bail!("current change targets another line; switch or retarget before pulling");
    }
    let previous = read_line(repo, &line)?;
    if previous.head_snapshot.is_none() {
        bail!("pull requires a populated line; use git clone or git fetch for initial history");
    }
    let policy = if args.domains.is_empty() {
        previous.policy.clone()
    } else {
        policy_from_domains(args.domains.clone())
    };
    fetch_history(repo, &remote, &branch, &line, &args.as_actor, policy)?;
    let updated = read_line(repo, &line)?;
    let tip = updated
        .head_snapshot
        .as_deref()
        .context("pulled line has no snapshot")?;
    // Import may assign an initially absent workspace pointer. Materialization
    // must compare against the actual pre-command workspace, not that overlay.
    write_json(repo, &repo.path(&["workspace.json"]), &before)?;
    if previous.head_snapshot == updated.head_snapshot && before.current_change.is_some() {
        output::record(
            "git_pull",
            serde_json::json!({"line":line,"snapshot_id":tip,"changed":false}),
        );
        println!("remote tip unchanged; workspace preserved");
        return Ok(());
    }
    if let Some(saved) = current
        .as_ref()
        .and_then(|change| change.current_snapshot.as_deref())
    {
        let saved = read_snapshot(repo, saved)?;
        let base = ancestry::merge_base(repo, Some(tip), &saved, None)?;
        if base.as_ref().map(|snapshot| &snapshot.id) != Some(&saved.id) {
            bail!("current change has saved work absent from the incoming line; integrate or switch before pulling");
        }
    }
    let snapshot = read_snapshot(repo, tip)?;
    checkout::restore(repo, Some(tip), false, &args.as_actor)?;
    create_change(repo, &format!("pull {}", line), &line, snapshot.policy)?;
    output::record(
        "git_pull",
        serde_json::json!({"line":line,"snapshot_id":tip,"changed":true}),
    );
    println!("pulled {} into {}; new change is ready", branch, line);
    Ok(())
}

pub(super) fn push(repo: &Repo, args: &GitPushArgs) -> Result<()> {
    verify::check(repo, &args.as_actor)?;
    let (line, remote, branch) = git_tracking::target(
        repo,
        args.line.as_deref(),
        args.remote.as_deref(),
        args.branch.as_deref(),
    )?;
    if let Some(expected) = &args.expect_snapshot {
        validate_object_id(expected, "snap_")?;
        if read_line(repo, &line)?.head_snapshot.as_ref() != Some(expected) {
            return Err(CliFailure::StaleSnapshot.into());
        }
    }
    let scratch = Scratch::new()?;
    run(
        &scratch.0,
        &scratch,
        &["check-ref-format", &format!("refs/heads/{}", branch)],
    )?;
    let exported = scratch.0.join("repository.git");
    git_bridge::export(
        repo,
        &exported,
        &line,
        args.author.as_deref(),
        &args.as_actor,
        args.allow_restricted,
        if args.dry_run {
            git_bridge::ExportMode::Preview
        } else {
            git_bridge::ExportMode::Transport
        },
    )?;
    if args.dry_run {
        return preview(
            repo,
            &exported,
            &scratch,
            &remote,
            &line,
            &branch,
            args.max_commits.unwrap_or(50),
        );
    }
    run(
        &exported,
        &scratch,
        &[
            "push",
            "--porcelain",
            "--",
            &remote,
            &format!("refs/heads/{}:refs/heads/{}", line, branch),
        ],
    )?;
    output::record(
        "git_push",
        serde_json::json!({"line":line,"branch":branch,"snapshot_id":read_line(repo,&line)?.head_snapshot}),
    );
    println!("published {} to remote branch {}", line, branch);
    Ok(())
}

fn capture_git(root: &Path, scratch: &Scratch, args: &[&str]) -> Result<String> {
    let output = transport(root, &scratch.0.join("no-hooks"))
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(args)
        .output()
        .context("failed to run Git preview")?;
    if !output.status.success() {
        bail!("Git preview failed; check remote access and branch availability");
    }
    String::from_utf8(output.stdout).context("Git preview output was not UTF-8")
}

fn preview(
    repo: &Repo,
    exported: &Path,
    scratch: &Scratch,
    remote: &str,
    line: &str,
    branch: &str,
    max_commits: u32,
) -> Result<()> {
    let reference = format!("refs/heads/{branch}");
    let local = capture_git(
        exported,
        scratch,
        &["rev-parse", &format!("refs/heads/{line}")],
    )?
    .trim()
    .to_string();
    let advertised = capture_git(
        exported,
        scratch,
        &["ls-remote", "--refs", "--", remote, &reference],
    )?;
    let remote_tip = if advertised.trim().is_empty() {
        None
    } else {
        // Fetch into scratch only. Compare the actually fetched tip, since the
        // remote can advance between discovery and transfer.
        run(
            exported,
            scratch,
            &["fetch", "--no-tags", "--", remote, &reference],
        )?;
        Some(
            capture_git(exported, scratch, &["rev-parse", "FETCH_HEAD"])?
                .trim()
                .to_string(),
        )
    };
    let (ahead, behind) = if let Some(tip) = &remote_tip {
        let counts = capture_git(
            exported,
            scratch,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{local}...{tip}"),
            ],
        )?;
        let mut counts = counts.split_whitespace();
        let ahead: usize = counts.next().context("missing ahead count")?.parse()?;
        let behind: usize = counts.next().context("missing behind count")?.parse()?;
        (ahead, behind)
    } else {
        (
            capture_git(exported, scratch, &["rev-list", "--count", &local])?
                .trim()
                .parse()?,
            0,
        )
    };
    let state = match (remote_tip.is_some(), ahead, behind) {
        (false, _, _) => "new_branch",
        (true, 0, 0) => "up_to_date",
        (true, _, 0) => "fast_forward",
        (true, 0, _) => "behind",
        _ => "diverged",
    };
    let limit = format!("--max-count={max_commits}");
    let mut arguments = vec!["log", "--reverse", &limit, "--format=%H%x09%s", &local];
    if let Some(tip) = &remote_tip {
        arguments.extend(["--not", tip]);
    }
    let log = capture_git(exported, scratch, &arguments)?;
    let commits: Vec<_> = log
        .lines()
        .map(|entry| {
            let (id, subject) = entry.split_once('\t').unwrap_or((entry, ""));
            serde_json::json!({"git_commit":id,"subject":subject})
        })
        .collect();
    output::record(
        "git_push_preview",
        serde_json::json!({
            "line":line, "branch":branch, "remote":remote,
            "snapshot_id":read_line(repo,line)?.head_snapshot,
            "local_commit":local, "remote_commit":remote_tip,
            "state":state, "ahead":ahead, "behind":behind,
            "fast_forward_allowed":behind == 0, "commits":commits,
            "commits_total":ahead, "commits_truncated":commits.len() < ahead,
        }),
    );
    println!("{line} -> {remote} ({branch}): {state}; {ahead} outgoing, {behind} incoming commits");
    println!("showing {} of {ahead} outgoing commits", commits.len());
    for commit in &commits {
        println!(
            "{} {}",
            commit["git_commit"].as_str().unwrap_or(""),
            commit["subject"].as_str().unwrap_or("")
        );
    }
    println!("Preview only: publishes saved line ancestry, excluding unsaved files and unintegrated changes. Remote state and server permissions are checked again on push.");
    Ok(())
}

pub(super) fn retarget(repo: &Repo, target: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let id = workspace
        .current_change
        .context("workspace has no current change")?;
    let mut change = read_change(repo, &id)?;
    let line = read_line(repo, target)?;
    if !can_access(&actor, &change.policy) || !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }
    change.target_line = target.to_string();
    write_json(repo, &change_path(repo, &id)?, &change)?;
    record_operation(
        repo,
        OperationKind::RetargetChange {
            change_id: id.clone(),
            line: target.to_string(),
        },
        admin_policy(),
        format!("retargeted change `{id}` to `{target}`"),
        None,
    )?;
    output::record(
        "change_retargeted",
        serde_json::json!({"id":id,"target_line":target}),
    );
    println!("retargeted {id} to {target}");
    Ok(())
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct CloneRequest {
    remote: String,
    branch: String,
    policy: AccessPolicy,
}

pub(super) fn clone_repository(args: &GitCloneArgs) -> Result<()> {
    let request = CloneRequest {
        remote: resolve_remote(&args.remote)?,
        branch: args.branch.clone(),
        policy: policy_from_domains(args.domains.clone()),
    };
    let repo = if args.resume {
        let destination = fs::canonicalize(&args.destination)?;
        let repo = Repo::discover_from(destination.clone())?;
        if repo.root != destination {
            bail!("resume destination is not a native repository root");
        }
        let recorded: CloneRequest = read_json(&repo, &repo.path(&["clone-request.json"]))
            .context("destination has no resumable clone request")?;
        if recorded != request {
            bail!("resume must use the original remote, branch and domains");
        }
        repo
    } else {
        fs::create_dir(&args.destination).context("clone destination must be a new directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&args.destination, fs::Permissions::from_mode(0o700))?;
        }
        let destination = fs::canonicalize(&args.destination)?;
        let repo = initialize_repo(destination)?;
        write_json(&repo, &repo.path(&["clone-request.json"]), &request)?;
        repo.transaction.commit()?;
        repo
    };
    let result = (|| -> Result<()> {
        let before = read_workspace(&repo)?;
        fetch(
            &repo,
            &request.remote,
            &request.branch,
            DEFAULT_LINE,
            ADMIN_DOMAIN,
            request.policy,
        )?;
        let head = read_line(&repo, DEFAULT_LINE)?
            .head_snapshot
            .context("remote branch has no saved head")?;
        let change = read_snapshot(&repo, &head)?.change_id;
        // Import can assign the first workspace pointer. Checkout must compare
        // against the actual pre-fetch baseline so new/untracked files are safe.
        write_json(&repo, &repo.path(&["workspace.json"]), &before)?;
        checkout::switch(&repo, &change, ADMIN_DOMAIN)?;
        git_tracking::configure_clone(&repo, &request.remote, &request.branch)?;
        verify::check(&repo, ADMIN_DOMAIN)?;
        repo.transaction.commit()?;
        fs::remove_file(repo.path(&["clone-request.json"]))?;
        #[cfg(unix)]
        fs::File::open(&repo.meta)?.sync_all()?;
        Ok(())
    })();
    result.with_context(|| {
        format!(
            "clone incomplete at {}; rerun the same clone with --resume after addressing the error",
            repo.root.display()
        )
    })?;
    output::record(
        "git_clone",
        serde_json::json!({"destination":repo.root.to_str(),"destination_display":repo.root.display().to_string(),"branch":args.branch,"snapshot_id":read_line(&repo,DEFAULT_LINE)?.head_snapshot}),
    );
    println!(
        "cloned Git branch {} into {}",
        args.branch,
        repo.root.display()
    );
    Ok(())
}
