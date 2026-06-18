use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

const META_DIR: &str = ".rgit";
const FORMAT_VERSION: u32 = 2;
const PUBLIC_DOMAIN: &str = "public";
const ADMIN_DOMAIN: &str = "admin";
const DEFAULT_LINE: &str = "main";

#[derive(Parser)]
#[command(name = "rgit")]
#[command(about = "A permission-aware, jj-inspired source control prototype.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize source control in the current directory.
    Init,
    /// Show changed files since the current change's latest snapshot.
    Status {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Capture the current files as a snapshot on the current change.
    Snapshot {
        /// Human note for why this snapshot exists.
        #[arg(short, long, default_value = "manual snapshot")]
        message: String,
        /// Domains allowed to see this snapshot's metadata.
        #[arg(long = "domain")]
        domains: Vec<String>,
    },
    /// Work with logical changes.
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
    },
    /// Manage permission actors.
    Actor {
        #[command(subcommand)]
        command: ActorCommand,
    },
    /// Manage path-level access policies.
    Access {
        #[command(subcommand)]
        command: AccessCommand,
    },
    /// Work with shared lines such as main.
    Line {
        #[command(subcommand)]
        command: LineCommand,
    },
    /// Inspect the current workspace.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Inspect the operation log.
    Op {
        #[command(subcommand)]
        command: OpCommand,
    },
}

#[derive(Subcommand)]
enum ChangeCommand {
    /// Create a new logical change and make the workspace point at it.
    New {
        /// Short, human-readable name for the change.
        name: String,
        /// Domains allowed to see this change.
        #[arg(long = "domain")]
        domains: Vec<String>,
    },
    /// List changes visible to an actor.
    List {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum ActorCommand {
    /// Create or replace an actor with domain grants.
    Set {
        name: String,
        /// Domain grant for this actor.
        #[arg(long = "domain")]
        domains: Vec<String>,
    },
    /// List actors.
    List,
}

#[derive(Subcommand)]
enum AccessCommand {
    /// Assign domains to a path prefix for future snapshots.
    Path {
        path: String,
        /// Domains allowed to see matching file entries.
        #[arg(long = "domain")]
        domains: Vec<String>,
    },
    /// List path policies.
    List,
}

#[derive(Subcommand)]
enum LineCommand {
    /// List lines visible to an actor.
    List {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Integrate the current change's latest snapshot into a line.
    Integrate {
        /// Line to update.
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
    },
    /// Show the files visible on a line to an actor.
    View {
        /// Line to view.
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Show the current workspace state.
    Info,
}

#[derive(Subcommand)]
enum OpCommand {
    /// Show repository operations visible to an actor.
    Log {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct AccessPolicy {
    domains: Vec<String>,
    redaction: Redaction,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Redaction {
    Omit,
    Placeholder,
    MetadataOnly,
}

#[derive(Serialize, Deserialize)]
struct RepoConfig {
    format_version: u32,
    repo_id: String,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct Actor {
    name: String,
    domains: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct PathPolicy {
    prefix: String,
    policy: AccessPolicy,
}

#[derive(Serialize, Deserialize)]
struct Workspace {
    current_change: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Change {
    id: String,
    name: String,
    current_snapshot: Option<String>,
    policy: AccessPolicy,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    id: String,
    change_id: String,
    parent_snapshot: Option<String>,
    message: String,
    manifest_hash: String,
    files: Vec<FileEntry>,
    policy: AccessPolicy,
    created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct FileEntry {
    path: String,
    hash: String,
    bytes: u64,
    policy: AccessPolicy,
}

#[derive(Serialize, Deserialize)]
struct Line {
    name: String,
    head_snapshot: Option<String>,
    policy: AccessPolicy,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct Operation {
    id: String,
    kind: OperationKind,
    policy: AccessPolicy,
    private_message: String,
    public_message: Option<String>,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationKind {
    InitRepo,
    SetActor {
        actor: String,
    },
    SetPathPolicy {
        prefix: String,
    },
    CreateChange {
        change_id: String,
    },
    CreateSnapshot {
        change_id: String,
        snapshot_id: String,
    },
    IntegrateLine {
        line: String,
        change_id: String,
        snapshot_id: String,
    },
}

struct Repo {
    root: PathBuf,
    meta: PathBuf,
}

struct FileDiff {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
    hidden: usize,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => init_repo(),
        Command::Status { as_actor } => {
            let repo = Repo::discover()?;
            status(&repo, &as_actor)
        }
        Command::Snapshot { message, domains } => {
            let repo = Repo::discover()?;
            create_snapshot(&repo, &message, policy_from_domains(domains))
        }
        Command::Change { command } => {
            let repo = Repo::discover()?;
            match command {
                ChangeCommand::New { name, domains } => {
                    create_change(&repo, &name, policy_from_domains(domains))
                }
                ChangeCommand::List { as_actor } => list_changes(&repo, &as_actor),
            }
        }
        Command::Actor { command } => {
            let repo = Repo::discover()?;
            match command {
                ActorCommand::Set { name, domains } => set_actor(&repo, &name, domains),
                ActorCommand::List => list_actors(&repo),
            }
        }
        Command::Access { command } => {
            let repo = Repo::discover()?;
            match command {
                AccessCommand::Path { path, domains } => set_path_policy(&repo, &path, domains),
                AccessCommand::List => list_path_policies(&repo),
            }
        }
        Command::Line { command } => {
            let repo = Repo::discover()?;
            match command {
                LineCommand::List { as_actor } => list_lines(&repo, &as_actor),
                LineCommand::Integrate { line } => integrate_line(&repo, &line),
                LineCommand::View { line, as_actor } => view_line(&repo, &line, &as_actor),
            }
        }
        Command::Workspace { command } => {
            let repo = Repo::discover()?;
            match command {
                WorkspaceCommand::Info => workspace_info(&repo),
            }
        }
        Command::Op { command } => {
            let repo = Repo::discover()?;
            match command {
                OpCommand::Log { as_actor } => op_log(&repo, &as_actor),
            }
        }
    }
}

impl Repo {
    fn discover() -> Result<Self> {
        let mut dir = std::env::current_dir().context("failed to read current directory")?;

        loop {
            let meta = dir.join(META_DIR);
            if meta.is_dir() {
                return Ok(Self { root: dir, meta });
            }

            if !dir.pop() {
                bail!("not inside an rgit repository; run `rgit init` first");
            }
        }
    }

    fn path(&self, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(self.meta.clone(), |path, part| path.join(part))
    }
}

impl FileDiff {
    fn print(&self) {
        print_paths("added", &self.added);
        print_paths("modified", &self.modified);
        print_paths("deleted", &self.deleted);

        if self.hidden > 0 {
            println!("hidden: {} restricted file(s)", self.hidden);
        }

        if self.added.is_empty()
            && self.modified.is_empty()
            && self.deleted.is_empty()
            && self.hidden == 0
        {
            println!("clean");
        }
    }
}

fn init_repo() -> Result<()> {
    let root = std::env::current_dir().context("failed to read current directory")?;
    let meta = root.join(META_DIR);

    if meta.exists() {
        bail!("repository already exists at {}", meta.display());
    }

    fs::create_dir(&meta).context("failed to create .rgit directory")?;
    for dir in [
        "actors",
        "blobs",
        "changes",
        "lines",
        "operations",
        "snapshots",
    ] {
        fs::create_dir_all(meta.join(dir)).with_context(|| format!("failed to create {dir}"))?;
    }

    let repo = Repo { root, meta };
    let config = RepoConfig {
        format_version: FORMAT_VERSION,
        repo_id: format!("repo_{}", short_id()),
        created_at: now()?,
    };
    let workspace = Workspace {
        current_change: None,
    };
    let public_actor = Actor {
        name: PUBLIC_DOMAIN.to_string(),
        domains: vec![PUBLIC_DOMAIN.to_string()],
    };
    let admin_actor = Actor {
        name: ADMIN_DOMAIN.to_string(),
        domains: vec![PUBLIC_DOMAIN.to_string(), ADMIN_DOMAIN.to_string()],
    };
    let main_line = Line {
        name: DEFAULT_LINE.to_string(),
        head_snapshot: None,
        policy: public_policy(),
        created_at: now()?,
    };

    write_json(&repo.path(&["repo.json"]), &config)?;
    write_json(&repo.path(&["workspace.json"]), &workspace)?;
    write_json(
        &repo.path(&["path-policies.json"]),
        &Vec::<PathPolicy>::new(),
    )?;
    write_json(&actor_path(&repo, PUBLIC_DOMAIN), &public_actor)?;
    write_json(&actor_path(&repo, ADMIN_DOMAIN), &admin_actor)?;
    write_json(&line_path(&repo, DEFAULT_LINE), &main_line)?;
    record_operation(
        &repo,
        OperationKind::InitRepo,
        public_policy(),
        "initialized repository".to_string(),
        None,
    )?;

    println!("initialized rgit repository");
    println!("default line: {DEFAULT_LINE}");
    println!("default actors: public, admin");
    Ok(())
}

fn set_actor(repo: &Repo, name: &str, domains: Vec<String>) -> Result<()> {
    let actor = Actor {
        name: name.to_string(),
        domains: normalize_domains(domains),
    };

    write_json(&actor_path(repo, name), &actor)?;
    record_operation(
        repo,
        OperationKind::SetActor {
            actor: actor.name.clone(),
        },
        admin_policy(),
        format!("set actor `{}`", actor.name),
        None,
    )?;

    println!("actor: {}", actor.name);
    println!("domains: {}", actor.domains.join(", "));
    Ok(())
}

fn list_actors(repo: &Repo) -> Result<()> {
    let mut actors = read_dir_json::<Actor>(&repo.path(&["actors"]))?;
    actors.sort_by(|a, b| a.name.cmp(&b.name));

    for actor in actors {
        println!("{} domains:{}", actor.name, actor.domains.join(","));
    }

    Ok(())
}

fn set_path_policy(repo: &Repo, prefix: &str, domains: Vec<String>) -> Result<()> {
    let mut policies = read_path_policies(repo)?;
    let normalized_prefix = normalize_path(prefix);
    let policy = AccessPolicy {
        domains: normalize_domains(domains),
        redaction: Redaction::Placeholder,
    };

    policies.retain(|item| item.prefix != normalized_prefix);
    policies.push(PathPolicy {
        prefix: normalized_prefix.clone(),
        policy: policy.clone(),
    });
    policies.sort_by(|a, b| a.prefix.cmp(&b.prefix));

    write_json(&repo.path(&["path-policies.json"]), &policies)?;
    record_operation(
        repo,
        OperationKind::SetPathPolicy {
            prefix: normalized_prefix.clone(),
        },
        admin_policy(),
        format!("set path policy `{normalized_prefix}`"),
        None,
    )?;

    println!("path: {normalized_prefix}");
    println!("domains: {}", policy.domains.join(", "));
    Ok(())
}

fn list_path_policies(repo: &Repo) -> Result<()> {
    for policy in read_path_policies(repo)? {
        println!(
            "{} domains:{}",
            policy.prefix,
            policy.policy.domains.join(",")
        );
    }

    Ok(())
}

fn create_change(repo: &Repo, name: &str, policy: AccessPolicy) -> Result<()> {
    let change = Change {
        id: format!("chg_{}", short_id()),
        name: name.to_string(),
        current_snapshot: None,
        policy: policy.clone(),
        created_at: now()?,
    };
    let workspace = Workspace {
        current_change: Some(change.id.clone()),
    };

    write_json(&change_path(repo, &change.id), &change)?;
    write_json(&repo.path(&["workspace.json"]), &workspace)?;
    record_operation(
        repo,
        OperationKind::CreateChange {
            change_id: change.id.clone(),
        },
        policy,
        format!("created change `{}`", change.name),
        None,
    )?;

    println!("created change {}", change.id);
    println!("workspace now points at `{}`", change.name);
    Ok(())
}

fn list_changes(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let mut changes = read_dir_json::<Change>(&repo.path(&["changes"]))?;
    changes.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    for change in changes {
        if !can_access(&actor, &change.policy) {
            continue;
        }

        let marker = if workspace.current_change.as_deref() == Some(change.id.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{marker} {} {} snapshot:{} domains:{}",
            change.id,
            change.name,
            change.current_snapshot.as_deref().unwrap_or("none"),
            change.policy.domains.join(",")
        );
    }

    Ok(())
}

fn create_snapshot(repo: &Repo, message: &str, requested_policy: AccessPolicy) -> Result<()> {
    let workspace = read_workspace(repo)?;
    let change_id = workspace.current_change.as_deref().ok_or_else(|| {
        anyhow!("workspace has no current change; run `rgit change new <name>` first")
    })?;
    let mut change = read_change(repo, change_id)?;
    let snapshot_policy = if requested_policy.domains == [PUBLIC_DOMAIN] {
        change.policy.clone()
    } else {
        requested_policy
    };
    let files = scan_working_tree(repo, true)?;
    let manifest_hash = manifest_hash(&files)?;
    let snapshot = Snapshot {
        id: format!("snap_{}", short_id()),
        change_id: change.id.clone(),
        parent_snapshot: change.current_snapshot.clone(),
        message: message.to_string(),
        manifest_hash,
        files,
        policy: snapshot_policy.clone(),
        created_at: now()?,
    };

    change.current_snapshot = Some(snapshot.id.clone());

    write_json(&snapshot_path(repo, &snapshot.id), &snapshot)?;
    write_json(&change_path(repo, &change.id), &change)?;
    record_operation(
        repo,
        OperationKind::CreateSnapshot {
            change_id: change.id.clone(),
            snapshot_id: snapshot.id.clone(),
        },
        snapshot_policy,
        format!("created snapshot for `{}`", change.name),
        None,
    )?;

    println!("created snapshot {}", snapshot.id);
    println!("change: {}", change.name);
    Ok(())
}

fn status(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let Some(change_id) = workspace.current_change else {
        println!("workspace has no current change");
        println!("next: rgit change new <name>");
        return Ok(());
    };

    let change = read_change(repo, &change_id)?;
    if !can_access(&actor, &change.policy) {
        println!("change is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    let current = visible_files(scan_working_tree(repo, false)?, &actor);
    let previous = match &change.current_snapshot {
        Some(snapshot_id) => read_snapshot(repo, snapshot_id)?.files,
        None => Vec::new(),
    };
    let (previous_visible, previous_hidden) = visible_files_with_hidden(previous, &actor);
    let diff = diff_files(previous_visible, current, previous_hidden);

    println!("actor: {}", actor.name);
    println!("change: {} ({})", change.name, change.id);
    println!(
        "snapshot: {}",
        change.current_snapshot.as_deref().unwrap_or("none")
    );
    diff.print();

    Ok(())
}

fn list_lines(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut lines = read_dir_json::<Line>(&repo.path(&["lines"]))?;
    lines.sort_by(|a, b| a.name.cmp(&b.name));

    for line in lines {
        if !can_access(&actor, &line.policy) {
            continue;
        }

        println!(
            "{} head:{} domains:{}",
            line.name,
            line.head_snapshot.as_deref().unwrap_or("none"),
            line.policy.domains.join(",")
        );
    }

    Ok(())
}

fn integrate_line(repo: &Repo, line_name: &str) -> Result<()> {
    let workspace = read_workspace(repo)?;
    let change_id = workspace.current_change.as_deref().ok_or_else(|| {
        anyhow!("workspace has no current change; run `rgit change new <name>` first")
    })?;
    let change = read_change(repo, change_id)?;
    let snapshot_id = change
        .current_snapshot
        .as_deref()
        .ok_or_else(|| anyhow!("change has no snapshot; run `rgit snapshot` first"))?;
    let snapshot = read_snapshot(repo, snapshot_id)?;
    let mut line = read_line(repo, line_name)?;

    line.head_snapshot = Some(snapshot.id.clone());
    write_json(&line_path(repo, &line.name), &line)?;
    record_operation(
        repo,
        OperationKind::IntegrateLine {
            line: line.name.clone(),
            change_id: change.id.clone(),
            snapshot_id: snapshot.id.clone(),
        },
        change.policy.clone(),
        format!(
            "integrated change `{}` ({}) into `{}`",
            change.name, change.id, line.name
        ),
        Some(format!("integrated restricted change into `{}`", line.name)),
    )?;

    println!("integrated {} into {}", change.id, line.name);
    println!("line head: {}", snapshot.id);
    Ok(())
}

fn view_line(repo: &Repo, line_name: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) {
        println!("line `{line_name}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    let Some(snapshot_id) = line.head_snapshot.as_deref() else {
        println!("line `{line_name}` has no head snapshot");
        return Ok(());
    };
    let snapshot = read_snapshot(repo, snapshot_id)?;
    let (visible, hidden) = visible_files_with_hidden(snapshot.files, &actor);

    println!("actor: {}", actor.name);
    println!("line: {}", line.name);
    if can_access(&actor, &snapshot.policy) {
        println!("snapshot: {}", snapshot.id);
    } else {
        println!("snapshot: restricted");
    }

    for file in visible {
        println!("{} {} bytes", file.path, file.bytes);
    }

    if hidden > 0 {
        println!("hidden: {hidden} restricted file(s)");
    }

    Ok(())
}

fn workspace_info(repo: &Repo) -> Result<()> {
    let workspace = read_workspace(repo)?;

    match workspace.current_change {
        Some(change_id) => {
            let change = read_change(repo, &change_id)?;
            println!("current change: {} ({})", change.name, change.id);
            println!("domains: {}", change.policy.domains.join(","));
            println!(
                "current snapshot: {}",
                change.current_snapshot.as_deref().unwrap_or("none")
            );
        }
        None => {
            println!("current change: none");
        }
    }

    Ok(())
}

fn op_log(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut operations = read_dir_json::<Operation>(&repo.path(&["operations"]))?;
    operations.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    for operation in operations {
        if can_access(&actor, &operation.policy) {
            println!(
                "{} {} {}",
                operation.id,
                operation_kind(&operation.kind),
                operation.private_message
            );
        } else if let Some(public_message) = operation.public_message {
            println!(
                "{} {} {}",
                operation.id,
                operation_kind(&operation.kind),
                public_message
            );
        }
    }

    Ok(())
}

fn record_operation(
    repo: &Repo,
    kind: OperationKind,
    policy: AccessPolicy,
    private_message: String,
    public_message: Option<String>,
) -> Result<()> {
    let operation = Operation {
        id: format!("op_{}", short_id()),
        kind,
        policy,
        private_message,
        public_message,
        created_at: now()?,
    };
    write_json(&operation_path(repo, &operation.id), &operation)
}

fn diff_files(previous: Vec<FileEntry>, current: Vec<FileEntry>, hidden: usize) -> FileDiff {
    let previous_map = manifest_map(previous);
    let current_map = manifest_map(current);
    let previous_paths = previous_map.keys().cloned().collect::<BTreeSet<_>>();
    let current_paths = current_map.keys().cloned().collect::<BTreeSet<_>>();
    let all_paths = previous_paths
        .union(&current_paths)
        .cloned()
        .collect::<Vec<_>>();

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    for path in all_paths {
        match (previous_map.get(&path), current_map.get(&path)) {
            (None, Some(_)) => added.push(path),
            (Some(_), None) => deleted.push(path),
            (Some(before), Some(after)) if before.hash != after.hash => modified.push(path),
            _ => {}
        }
    }

    FileDiff {
        added,
        modified,
        deleted,
        hidden,
    }
}

fn read_workspace(repo: &Repo) -> Result<Workspace> {
    read_json(&repo.path(&["workspace.json"]))
}

fn read_actor(repo: &Repo, name: &str) -> Result<Actor> {
    read_json(&actor_path(repo, name)).with_context(|| format!("actor `{name}` not found"))
}

fn read_change(repo: &Repo, id: &str) -> Result<Change> {
    read_json(&change_path(repo, id)).with_context(|| format!("change `{id}` not found"))
}

fn read_snapshot(repo: &Repo, id: &str) -> Result<Snapshot> {
    read_json(&snapshot_path(repo, id)).with_context(|| format!("snapshot `{id}` not found"))
}

fn read_line(repo: &Repo, name: &str) -> Result<Line> {
    read_json(&line_path(repo, name)).with_context(|| format!("line `{name}` not found"))
}

fn read_path_policies(repo: &Repo) -> Result<Vec<PathPolicy>> {
    read_json(&repo.path(&["path-policies.json"]))
}

fn actor_path(repo: &Repo, name: &str) -> PathBuf {
    repo.path(&["actors", &format!("{}.json", file_name(name))])
}

fn change_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["changes", &format!("{id}.json")])
}

fn line_path(repo: &Repo, name: &str) -> PathBuf {
    repo.path(&["lines", &format!("{}.json", file_name(name))])
}

fn snapshot_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["snapshots", &format!("{id}.json")])
}

fn operation_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["operations", &format!("{id}.json")])
}

fn scan_working_tree(repo: &Repo, store_blobs: bool) -> Result<Vec<FileEntry>> {
    let path_policies = read_path_policies(repo)?;
    let mut files = Vec::new();

    for entry in WalkDir::new(&repo.root)
        .into_iter()
        .filter_entry(|entry| should_scan(entry.path()))
    {
        let entry = entry.context("failed to read directory entry")?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        let hash = hash_bytes(&bytes);
        let relative_path = normalize_path(
            &path
                .strip_prefix(&repo.root)
                .context("failed to compute relative path")?
                .to_string_lossy(),
        );

        if store_blobs {
            let blob_path = repo.path(&["blobs", &hash]);
            if !blob_path.exists() {
                fs::write(&blob_path, &bytes)
                    .with_context(|| format!("failed to write {}", blob_path.display()))?;
            }
        }

        files.push(FileEntry {
            policy: policy_for_path(&relative_path, &path_policies),
            path: relative_path,
            hash,
            bytes: bytes.len() as u64,
        });
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn should_scan(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    !matches!(name, Some(".git" | META_DIR | "target" | "node_modules"))
}

fn visible_files(files: Vec<FileEntry>, actor: &Actor) -> Vec<FileEntry> {
    visible_files_with_hidden(files, actor).0
}

fn visible_files_with_hidden(files: Vec<FileEntry>, actor: &Actor) -> (Vec<FileEntry>, usize) {
    let mut hidden = 0;
    let mut visible = Vec::new();

    for file in files {
        if can_access(actor, &file.policy) {
            visible.push(file);
        } else {
            hidden += 1;
        }
    }

    (visible, hidden)
}

fn policy_for_path(path: &str, policies: &[PathPolicy]) -> AccessPolicy {
    policies
        .iter()
        .filter(|policy| path == policy.prefix || path.starts_with(&format!("{}/", policy.prefix)))
        .max_by_key(|policy| policy.prefix.len())
        .map(|policy| policy.policy.clone())
        .unwrap_or_else(public_policy)
}

fn can_access(actor: &Actor, policy: &AccessPolicy) -> bool {
    if actor.domains.iter().any(|domain| domain == ADMIN_DOMAIN) {
        return true;
    }

    policy.domains.iter().any(|domain| {
        actor
            .domains
            .iter()
            .any(|actor_domain| actor_domain == domain)
    })
}

fn public_policy() -> AccessPolicy {
    AccessPolicy {
        domains: vec![PUBLIC_DOMAIN.to_string()],
        redaction: Redaction::Omit,
    }
}

fn admin_policy() -> AccessPolicy {
    AccessPolicy {
        domains: vec![ADMIN_DOMAIN.to_string()],
        redaction: Redaction::Placeholder,
    }
}

fn policy_from_domains(domains: Vec<String>) -> AccessPolicy {
    AccessPolicy {
        domains: normalize_domains(domains),
        redaction: Redaction::Omit,
    }
}

fn normalize_domains(domains: Vec<String>) -> Vec<String> {
    let mut normalized = domains
        .into_iter()
        .filter(|domain| !domain.trim().is_empty())
        .map(|domain| domain.trim().to_string())
        .collect::<BTreeSet<_>>();

    if normalized.is_empty() {
        normalized.insert(PUBLIC_DOMAIN.to_string());
    }

    normalized.into_iter().collect()
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_start_matches("./").replace('\\', "/")
}

fn manifest_hash(files: &[FileEntry]) -> Result<String> {
    let encoded = serde_json::to_vec(files).context("failed to encode manifest")?;
    Ok(hash_bytes(&encoded))
}

fn manifest_map(entries: Vec<FileEntry>) -> BTreeMap<String, FileEntry> {
    entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect()
}

fn print_paths(label: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }

    println!("{label}:");
    for path in paths {
        println!("  {path}");
    }
}

fn read_dir_json<T: DeserializeOwned>(dir: &Path) -> Result<Vec<T>> {
    let mut values = Vec::new();

    if !dir.exists() {
        return Ok(values);
    }

    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            values.push(read_json(&path)?);
        }
    }

    Ok(values)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(value).context("failed to serialize json")?;
    fs::write(path, format!("{json}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn operation_kind(kind: &OperationKind) -> &'static str {
    match kind {
        OperationKind::InitRepo => "init_repo",
        OperationKind::SetActor { .. } => "set_actor",
        OperationKind::SetPathPolicy { .. } => "set_path_policy",
        OperationKind::CreateChange { .. } => "create_change",
        OperationKind::CreateSnapshot { .. } => "create_snapshot",
        OperationKind::IntegrateLine { .. } => "integrate_line",
    }
}

fn file_name(name: &str) -> String {
    name.replace('/', "__")
}

fn short_id() -> String {
    Uuid::new_v4().simple().to_string()[..12].to_string()
}

fn now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_millis() as u64)
}
