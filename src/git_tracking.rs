//! Shared Git destinations; credentials remain in the installed Git client's helpers.
use super::*;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    #[serde(default)]
    remotes: BTreeMap<String, String>,
    #[serde(default)]
    upstreams: BTreeMap<String, Upstream>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Upstream {
    remote: String,
    branch: String,
}

impl Config {
    pub(super) fn is_empty(&self) -> bool {
        self.remotes.is_empty() && self.upstreams.is_empty()
    }

    pub(super) fn validate(&self) -> Result<()> {
        for (name, destination) in &self.remotes {
            validate_named_key(name)?;
            git_remotes::validate_remote(destination)?;
            // Relative paths must be resolved when configured, never at use time.
            if !Path::new(destination).is_absolute() && !destination.contains(':') {
                bail!("saved Git remote must be an absolute path or URL");
            }
        }
        for (line, upstream) in &self.upstreams {
            validate_named_key(line)?;
            validate_branch(&upstream.branch)?;
            if !self.remotes.contains_key(&upstream.remote) {
                bail!("upstream refers to a missing Git remote");
            }
        }
        Ok(())
    }
}

// refs/heads/BRANCH syntax, without requiring Git for ordinary repository verify.
fn validate_branch(branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || branch
            .chars()
            .any(|c| c.is_control() || " ~^:?*[\\".contains(c))
        || branch
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
    {
        bail!("invalid Git branch name");
    }
    Ok(())
}

#[derive(Subcommand)]
pub(super) enum RemoteCommand {
    /// Add or replace a named destination. Relative local paths are saved as absolute paths.
    Set {
        name: String,
        destination: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// List saved destinations (requires the admin view).
    List {
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Remove a destination only when no line tracks it.
    Remove {
        name: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
pub(super) enum UpstreamCommand {
    /// Associate a native line with a named remote and Git branch.
    Set {
        remote: String,
        branch: String,
        #[arg(long)]
        line: Option<String>,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Show tracking for the selected or current line.
    Show {
        #[arg(long)]
        line: Option<String>,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Stop tracking the selected or current line.
    Unset {
        #[arg(long)]
        line: Option<String>,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

fn read(repo: &Repo) -> Result<RepoConfig> {
    let config: RepoConfig = read_json(repo, &repo.path(&["repo.json"]))?;
    config.git.validate()?;
    Ok(config)
}

fn save(repo: &Repo, config: &RepoConfig) -> Result<()> {
    config.git.validate()?;
    write_json(repo, &repo.path(&["repo.json"]), config)
}

pub(super) fn current_line(repo: &Repo, requested: Option<&str>) -> Result<String> {
    let line = match requested {
        Some(line) => line.to_string(),
        None => read_workspace(repo)?
            .current_change
            .map(|id| read_change(repo, &id).map(|change| change.target_line))
            .transpose()?
            .unwrap_or_else(|| DEFAULT_LINE.to_string()),
    };
    read_line(repo, &line)?;
    Ok(line)
}

pub(super) fn resolve_named(repo: &Repo, remote: &str) -> Result<String> {
    let config = read(repo)?;
    git_remotes::resolve_remote(
        config
            .git
            .remotes
            .get(remote)
            .map(String::as_str)
            .unwrap_or(remote),
    )
}

pub(super) fn target(
    repo: &Repo,
    line: Option<&str>,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<(String, String, String)> {
    // Preserve the original explicit-remote CLI default. Only the new
    // no-remote workflow infers its line from the current workspace.
    let line = current_line(repo, line.or(remote.map(|_| DEFAULT_LINE)))?;
    let config = read(repo)?;
    let upstream = config.git.upstreams.get(&line);
    let remote = remote
        .or_else(|| upstream.map(|u| u.remote.as_str()))
        .context(
        "line has no Git upstream; use upstream set REMOTE BRANCH --as admin, or supply a remote",
    )?;
    let branch = branch
        .or_else(|| {
            upstream
                .filter(|u| u.remote == remote)
                .map(|u| u.branch.as_str())
        })
        .unwrap_or(DEFAULT_LINE);
    validate_branch(branch)?;
    Ok((line, resolve_named(repo, remote)?, branch.to_string()))
}

pub(super) fn remote(repo: &Repo, command: &RemoteCommand) -> Result<()> {
    let actor = match command {
        RemoteCommand::Set { as_actor, .. }
        | RemoteCommand::List { as_actor }
        | RemoteCommand::Remove { as_actor, .. } => as_actor,
    };
    verify::check(repo, actor)?;
    let mut config = read(repo)?;
    match command {
        RemoteCommand::Set {
            name, destination, ..
        } => {
            validate_named_key(name)?;
            let destination = git_remotes::resolve_remote(destination)?;
            let changed = config.git.remotes.get(name) != Some(&destination);
            config.git.remotes.insert(name.clone(), destination.clone());
            save(repo, &config)?;
            output::record(
                "git_remote",
                serde_json::json!({"name":name,"destination":destination,"changed":changed}),
            );
            println!("saved remote {name}: {destination}");
        }
        RemoteCommand::List { .. } => {
            output::record(
                "git_remotes",
                serde_json::json!({"remotes":config.git.remotes}),
            );
            for (name, destination) in config.git.remotes {
                println!("{name}: {destination}");
            }
        }
        RemoteCommand::Remove { name, .. } => {
            if config
                .git
                .upstreams
                .values()
                .any(|upstream| upstream.remote == *name)
            {
                bail!("remote is tracked by a line; unset or change its upstream first");
            }
            let changed = config.git.remotes.remove(name).is_some();
            save(repo, &config)?;
            output::record(
                "git_remote_removed",
                serde_json::json!({"name":name,"changed":changed}),
            );
            println!("removed remote {name} (changed: {changed})");
        }
    }
    Ok(())
}

pub(super) fn upstream(repo: &Repo, command: &UpstreamCommand) -> Result<()> {
    let (line, actor) = match command {
        UpstreamCommand::Set { line, as_actor, .. }
        | UpstreamCommand::Show { line, as_actor }
        | UpstreamCommand::Unset { line, as_actor } => (line, as_actor),
    };
    verify::check(repo, actor)?;
    let line = current_line(repo, line.as_deref())?;
    let mut config = read(repo)?;
    match command {
        UpstreamCommand::Set { remote, branch, .. } => {
            validate_branch(branch)?;
            if !config.git.remotes.contains_key(remote) {
                bail!("configure the named remote first with remote set");
            }
            config.git.upstreams.insert(
                line.clone(),
                Upstream {
                    remote: remote.clone(),
                    branch: branch.clone(),
                },
            );
            save(repo, &config)?;
        }
        UpstreamCommand::Unset { .. } => {
            config.git.upstreams.remove(&line);
            save(repo, &config)?;
        }
        UpstreamCommand::Show { .. } => {}
    }
    let upstream = config.git.upstreams.get(&line);
    output::record(
        "git_upstream",
        serde_json::json!({"line":line,"upstream":upstream}),
    );
    match upstream {
        Some(upstream) => println!("{line} tracks {}/{}", upstream.remote, upstream.branch),
        None => println!("{line} has no Git upstream"),
    }
    Ok(())
}

pub(super) fn configure_clone(repo: &Repo, remote: &str, branch: &str) -> Result<()> {
    let mut config = read(repo)?;
    config.git.remotes.insert("origin".into(), remote.into());
    config.git.upstreams.insert(
        DEFAULT_LINE.into(),
        Upstream {
            remote: "origin".into(),
            branch: branch.into(),
        },
    );
    save(repo, &config)
}
