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

fn validate_remote(remote: &str) -> Result<()> {
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

fn resolve_remote(remote: &str) -> Result<String> {
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
    let status = transport(root, &scratch.0.join("no-hooks"))
        .args(args)
        .status()
        .context("failed to run Git transport")?;
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
    verify::check(repo, actor)?;
    let remote = resolve_remote(remote)?;
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

pub(super) fn push(repo: &Repo, args: &GitPushArgs) -> Result<()> {
    verify::check(repo, &args.as_actor)?;
    let remote = resolve_remote(&args.remote)?;
    let scratch = Scratch::new()?;
    run(
        &scratch.0,
        &scratch,
        &["check-ref-format", &format!("refs/heads/{}", args.branch)],
    )?;
    let exported = scratch.0.join("repository.git");
    git_bridge::export(
        repo,
        &exported,
        &args.line,
        args.author.as_deref(),
        &args.as_actor,
        args.allow_restricted,
        true,
    )?;
    run(
        &exported,
        &scratch,
        &[
            "push",
            "--porcelain",
            "--",
            &remote,
            &format!("refs/heads/{}:refs/heads/{}", args.line, args.branch),
        ],
    )?;
    println!("published {} to remote branch {}", args.line, args.branch);
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
    println!("retargeted {id} to {target}");
    Ok(())
}
