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
    /// Inspect snapshots.
    SnapshotInfo {
        #[command(subcommand)]
        command: SnapshotCommand,
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
    /// Compare permissioned file states.
    Diff {
        #[command(subcommand)]
        command: DiffCommand,
    },
    /// Preview merges without changing a line.
    Merge {
        #[command(subcommand)]
        command: MergeCommand,
    },
    /// Inspect merge conflicts.
    Conflict {
        #[command(subcommand)]
        command: ConflictCommand,
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
        /// Line this change is intended to integrate into.
        #[arg(long = "target", default_value = DEFAULT_LINE)]
        target: String,
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
    /// Show one change if the actor can see it.
    Show {
        change_id: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum SnapshotCommand {
    /// List snapshots visible to an actor.
    List {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Show files in one snapshot as an actor.
    Show {
        snapshot_id: String,
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
        /// Actor performing the integration.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
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
    /// Show visible line integration history.
    History {
        /// Line to inspect.
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum DiffCommand {
    /// Diff the current workspace against the current change snapshot.
    Workspace {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Diff two snapshots.
    Snapshot {
        old_snapshot: String,
        new_snapshot: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Diff a line's parent snapshot against its head snapshot.
    Line {
        /// Line to diff.
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum MergeCommand {
    /// Preview integrating the current or selected change into a line.
    Preview {
        /// Change to preview. Defaults to the current workspace change.
        change_id: Option<String>,
        /// Target line.
        #[arg(long = "into", default_value = DEFAULT_LINE)]
        line: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum ConflictCommand {
    /// List unresolved conflicts visible to an actor.
    List {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Show one conflict if visible to an actor.
    Show {
        conflict_id: String,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct AccessPolicy {
    domains: Vec<String>,
    redaction: Redaction,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Actor {
    name: String,
    domains: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    base_snapshot: Option<String>,
    #[serde(default = "default_line_name")]
    target_line: String,
    current_snapshot: Option<String>,
    policy: AccessPolicy,
    created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConflictStatus {
    Unresolved,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ConflictKind {
    BothModified,
    DeleteModify,
    AddAdd,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Conflict {
    id: String,
    line: String,
    change_id: String,
    base_snapshot: Option<String>,
    line_snapshot: Option<String>,
    incoming_snapshot: String,
    path: String,
    kind: ConflictKind,
    policy: AccessPolicy,
    #[serde(default = "public_policy")]
    line_policy: AccessPolicy,
    #[serde(default = "public_policy")]
    change_policy: AccessPolicy,
    #[serde(default = "public_policy")]
    file_policy: AccessPolicy,
    #[serde(default)]
    file_policies: Vec<AccessPolicy>,
    #[serde(default = "public_policy")]
    source_policy: AccessPolicy,
    status: ConflictStatus,
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
    CreateConflict {
        conflict_id: String,
        change_id: String,
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

struct DiffInput {
    visible: Vec<FileEntry>,
    hidden_by_path: BTreeMap<String, String>,
}

struct MergePlan {
    merged_files: Vec<FileEntry>,
    conflicts: Vec<PendingConflict>,
}

#[derive(Clone)]
struct PendingConflict {
    path: String,
    kind: ConflictKind,
    policy: AccessPolicy,
    file_policies: Vec<AccessPolicy>,
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
        Command::SnapshotInfo { command } => {
            let repo = Repo::discover()?;
            match command {
                SnapshotCommand::List { as_actor } => list_snapshots(&repo, &as_actor),
                SnapshotCommand::Show {
                    snapshot_id,
                    as_actor,
                } => show_snapshot(&repo, &snapshot_id, &as_actor),
            }
        }
        Command::Change { command } => {
            let repo = Repo::discover()?;
            match command {
                ChangeCommand::New {
                    name,
                    target,
                    domains,
                } => create_change(&repo, &name, &target, policy_from_domains(domains)),
                ChangeCommand::List { as_actor } => list_changes(&repo, &as_actor),
                ChangeCommand::Show {
                    change_id,
                    as_actor,
                } => show_change(&repo, &change_id, &as_actor),
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
                LineCommand::Integrate { line, as_actor } => {
                    integrate_line(&repo, &line, &as_actor)
                }
                LineCommand::View { line, as_actor } => view_line(&repo, &line, &as_actor),
                LineCommand::History { line, as_actor } => line_history(&repo, &line, &as_actor),
            }
        }
        Command::Diff { command } => {
            let repo = Repo::discover()?;
            match command {
                DiffCommand::Workspace { as_actor } => diff_workspace(&repo, &as_actor),
                DiffCommand::Snapshot {
                    old_snapshot,
                    new_snapshot,
                    as_actor,
                } => diff_snapshots(&repo, &old_snapshot, &new_snapshot, &as_actor),
                DiffCommand::Line { line, as_actor } => diff_line(&repo, &line, &as_actor),
            }
        }
        Command::Merge { command } => {
            let repo = Repo::discover()?;
            match command {
                MergeCommand::Preview {
                    change_id,
                    line,
                    as_actor,
                } => merge_preview(&repo, change_id.as_deref(), &line, &as_actor),
            }
        }
        Command::Conflict { command } => {
            let repo = Repo::discover()?;
            match command {
                ConflictCommand::List { as_actor } => list_conflicts(&repo, &as_actor),
                ConflictCommand::Show {
                    conflict_id,
                    as_actor,
                } => show_conflict(&repo, &conflict_id, &as_actor),
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
        "conflicts",
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

fn create_change(repo: &Repo, name: &str, target_line: &str, policy: AccessPolicy) -> Result<()> {
    let target = read_line(repo, target_line)?;
    let base_snapshot = read_line(repo, target_line)
        .ok()
        .and_then(|line| line.head_snapshot);
    let change = Change {
        id: format!("chg_{}", short_id()),
        name: name.to_string(),
        base_snapshot,
        target_line: target.name,
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
    println!("target line: {}", change.target_line);
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

fn show_change(repo: &Repo, change_id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let change = read_change(repo, change_id)?;

    if !can_access(&actor, &change.policy) {
        println!("change `{change_id}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    println!("change: {} ({})", change.name, change.id);
    println!("target line: {}", change.target_line);
    println!("domains: {}", change.policy.domains.join(","));
    println!(
        "current snapshot: {}",
        change.current_snapshot.as_deref().unwrap_or("none")
    );

    if let Some(snapshot_id) = change.current_snapshot.as_deref() {
        let snapshot = read_snapshot(repo, snapshot_id)?;
        print_snapshot_summary(&snapshot, &actor);
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

fn list_snapshots(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut snapshots = read_dir_json::<Snapshot>(&repo.path(&["snapshots"]))?;
    snapshots.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    for snapshot in snapshots {
        if !can_access(&actor, &snapshot.policy) {
            continue;
        }

        let (visible, hidden) = visible_files_with_hidden(snapshot.files, &actor);
        println!(
            "{} change:{} files:{} hidden:{} domains:{} message:{}",
            snapshot.id,
            snapshot.change_id,
            visible.len(),
            hidden,
            snapshot.policy.domains.join(","),
            snapshot.message
        );
    }

    Ok(())
}

fn show_snapshot(repo: &Repo, snapshot_id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let snapshot = read_snapshot(repo, snapshot_id)?;

    if !can_access(&actor, &snapshot.policy) {
        println!(
            "snapshot `{snapshot_id}` is hidden from actor `{}`",
            actor.name
        );
        return Ok(());
    }

    print_snapshot_summary(&snapshot, &actor);
    let (visible, hidden) = visible_files_with_hidden(snapshot.files, &actor);

    for file in visible {
        println!("{} {} bytes", file.path, file.bytes);
    }

    if hidden > 0 {
        println!("hidden: {hidden} restricted file(s)");
    }

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

fn diff_workspace(repo: &Repo, actor_name: &str) -> Result<()> {
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

    let previous = match &change.current_snapshot {
        Some(snapshot_id) => read_snapshot(repo, snapshot_id)?.files,
        None => Vec::new(),
    };
    let current = scan_working_tree(repo, false)?;
    let diff = permissioned_diff(previous, current, &actor);

    println!("actor: {}", actor.name);
    println!("diff: workspace");
    println!(
        "base snapshot: {}",
        change.current_snapshot.as_deref().unwrap_or("none")
    );
    diff.print();
    Ok(())
}

fn diff_snapshots(
    repo: &Repo,
    old_snapshot: &str,
    new_snapshot: &str,
    actor_name: &str,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let old = read_snapshot(repo, old_snapshot)?;
    let new = read_snapshot(repo, new_snapshot)?;

    if !can_access(&actor, &old.policy) || !can_access(&actor, &new.policy) {
        println!(
            "one or more snapshots are hidden from actor `{}`",
            actor.name
        );
        return Ok(());
    }

    let diff = permissioned_diff(old.files, new.files, &actor);

    println!("actor: {}", actor.name);
    println!("diff: {old_snapshot} -> {new_snapshot}");
    diff.print();
    Ok(())
}

fn diff_line(repo: &Repo, line_name: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) {
        println!("line `{line_name}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    let Some(head_snapshot_id) = line.head_snapshot.as_deref() else {
        println!("line `{line_name}` has no head snapshot");
        return Ok(());
    };
    let head = read_snapshot(repo, head_snapshot_id)?;
    let parent_files = match head.parent_snapshot.as_deref() {
        Some(parent_id) => read_snapshot(repo, parent_id)?.files,
        None => Vec::new(),
    };
    let diff = permissioned_diff(parent_files, head.files, &actor);

    println!("actor: {}", actor.name);
    println!("diff: line {line_name}");
    println!("head snapshot: {head_snapshot_id}");
    println!(
        "parent snapshot: {}",
        head.parent_snapshot.as_deref().unwrap_or("none")
    );
    diff.print();
    Ok(())
}

fn merge_preview(
    repo: &Repo,
    change_id: Option<&str>,
    line_name: &str,
    actor_name: &str,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let change_id = match change_id {
        Some(change_id) => change_id.to_string(),
        None => read_workspace(repo)?
            .current_change
            .ok_or_else(|| anyhow!("workspace has no current change"))?,
    };
    let change = read_change(repo, &change_id)?;
    let incoming_snapshot_id = change
        .current_snapshot
        .as_deref()
        .ok_or_else(|| anyhow!("change has no snapshot; run `rgit snapshot` first"))?;
    let incoming = read_snapshot(repo, incoming_snapshot_id)?;
    let line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) {
        println!("line `{line_name}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    if !can_access(&actor, &change.policy) {
        println!("change `{change_id}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    if change.target_line != line.name {
        println!(
            "change `{}` targets `{}`, not `{}`",
            change.id, change.target_line, line.name
        );
        return Ok(());
    }

    let base_snapshot = read_optional_snapshot(repo, change.base_snapshot.as_deref())?;
    let line_snapshot = read_optional_snapshot(repo, line.head_snapshot.as_deref())?;
    let base_files = optional_snapshot_files(&base_snapshot);
    let line_files = optional_snapshot_files(&line_snapshot);
    let incoming_files = incoming.files.clone();

    if !can_access_merge_sources(
        &actor,
        [&base_snapshot, &line_snapshot, &Some(incoming.clone())],
        [&base_files, &line_files, &incoming_files],
    ) {
        println!("merge preview unavailable for actor `{}`", actor.name);
        return Ok(());
    }

    let plan = plan_merge(base_files, line_files, incoming_files);

    println!("actor: {}", actor.name);
    println!("merge preview: {} -> {}", change.name, line.name);
    println!(
        "base snapshot: {}",
        change.base_snapshot.as_deref().unwrap_or("none")
    );
    println!(
        "line head: {}",
        line.head_snapshot.as_deref().unwrap_or("none")
    );
    println!("incoming snapshot: {incoming_snapshot_id}");

    if plan.conflicts.is_empty() {
        println!("result: clean");
        println!("merged files: {}", plan.merged_files.len());
    } else {
        println!("result: conflicts");
        for conflict in plan.conflicts {
            if can_access_pending_conflict(&actor, &conflict) {
                println!("{} {}", conflict_kind(&conflict.kind), conflict.path);
            }
        }
    }

    Ok(())
}

fn list_conflicts(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let mut conflicts = read_dir_json::<Conflict>(&repo.path(&["conflicts"]))?;
    conflicts.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    for conflict in conflicts {
        if conflict.status != ConflictStatus::Unresolved {
            continue;
        }

        if can_access_conflict(&actor, &conflict) {
            print_conflict_for_actor(&conflict, &actor);
        }
    }

    Ok(())
}

fn show_conflict(repo: &Repo, conflict_id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let Ok(conflict) = read_conflict(repo, conflict_id) else {
        println!("conflict is restricted or not found");
        return Ok(());
    };

    if can_access_conflict(&actor, &conflict) {
        println!("conflict: {}", conflict.id);
        println!("line: {}", conflict.line);
        println!("change: {}", conflict.change_id);
        println!("status: {}", conflict_status(&conflict.status));
        println!("path: {}", conflict.path);
        println!("kind: {}", conflict_kind(&conflict.kind));
        println!(
            "base snapshot: {}",
            conflict.base_snapshot.as_deref().unwrap_or("none")
        );
        println!(
            "line snapshot: {}",
            conflict.line_snapshot.as_deref().unwrap_or("none")
        );
        println!("incoming snapshot: {}", conflict.incoming_snapshot);
        println!("domains: {}", conflict.policy.domains.join(","));
    } else {
        println!("conflict is restricted or not found");
    }

    Ok(())
}

fn print_conflict_for_actor(conflict: &Conflict, actor: &Actor) {
    if can_access_conflict(actor, conflict) {
        println!(
            "{} {} {} line:{} change:{}",
            conflict.id,
            conflict_kind(&conflict.kind),
            conflict.path,
            conflict.line,
            conflict.change_id
        );
    } else {
        println!("restricted_conflict requires authorized resolver");
    }
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

fn integrate_line(repo: &Repo, line_name: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let change_id = workspace.current_change.as_deref().ok_or_else(|| {
        anyhow!("workspace has no current change; run `rgit change new <name>` first")
    })?;
    let change = read_change(repo, change_id)?;
    let incoming_snapshot_id = change
        .current_snapshot
        .as_deref()
        .ok_or_else(|| anyhow!("change has no snapshot; run `rgit snapshot` first"))?;
    let incoming = read_snapshot(repo, incoming_snapshot_id)?;
    let mut line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) {
        println!("line `{line_name}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    if !can_access(&actor, &change.policy) {
        println!(
            "change `{}` is hidden from actor `{}`",
            change.id, actor.name
        );
        return Ok(());
    }

    if change.target_line != line.name {
        println!(
            "change `{}` targets `{}`, not `{}`",
            change.id, change.target_line, line.name
        );
        return Ok(());
    }

    let base_snapshot = read_optional_snapshot(repo, change.base_snapshot.as_deref())?;
    let line_snapshot = read_optional_snapshot(repo, line.head_snapshot.as_deref())?;
    let source_policy = merge_source_policy(
        &line.policy,
        &change.policy,
        &incoming,
        &base_snapshot,
        &line_snapshot,
    );
    let base_files = optional_snapshot_files(&base_snapshot);
    let line_files = optional_snapshot_files(&line_snapshot);
    let incoming_files = incoming.files.clone();

    if !can_access_merge_sources(
        &actor,
        [&base_snapshot, &line_snapshot, &Some(incoming.clone())],
        [&base_files, &line_files, &incoming_files],
    ) {
        println!("integration unavailable for actor `{}`", actor.name);
        return Ok(());
    }

    let plan = plan_merge(base_files, line_files, incoming_files);

    if !plan.conflicts.is_empty() {
        let conflicts = store_conflicts(
            repo,
            &line,
            &change,
            &incoming,
            source_policy,
            plan.conflicts,
        )?;
        println!("integration blocked");
        for conflict in conflicts {
            if can_access_conflict(&actor, &conflict) {
                print_conflict_for_actor(&conflict, &actor);
            }
        }
        return Ok(());
    }

    let integrated_policy = source_policy;
    let public_integration_message = if integrated_policy.domains == [PUBLIC_DOMAIN] {
        Some(format!("integrated change into `{}`", line.name))
    } else {
        None
    };
    let integrated_snapshot = Snapshot {
        id: format!("snap_{}", short_id()),
        change_id: change.id.clone(),
        parent_snapshot: line.head_snapshot.clone(),
        message: format!("merge {} into {}", change.name, line.name),
        manifest_hash: manifest_hash(&plan.merged_files)?,
        files: plan.merged_files,
        policy: integrated_policy.clone(),
        created_at: now()?,
    };
    write_json(
        &snapshot_path(repo, &integrated_snapshot.id),
        &integrated_snapshot,
    )?;

    line.head_snapshot = Some(integrated_snapshot.id.clone());
    write_json(&line_path(repo, &line.name), &line)?;
    record_operation(
        repo,
        OperationKind::IntegrateLine {
            line: line.name.clone(),
            change_id: change.id.clone(),
            snapshot_id: integrated_snapshot.id.clone(),
        },
        integrated_policy,
        format!(
            "integrated change `{}` ({}) into `{}`",
            change.name, change.id, line.name
        ),
        public_integration_message,
    )?;

    println!("integrated {} into {}", change.id, line.name);
    println!("line head: {}", integrated_snapshot.id);
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

fn line_history(repo: &Repo, line_name: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) {
        println!("line `{line_name}` is hidden from actor `{}`", actor.name);
        return Ok(());
    }

    let mut operations = read_dir_json::<Operation>(&repo.path(&["operations"]))?;
    operations.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    for operation in operations {
        let OperationKind::IntegrateLine { line, .. } = &operation.kind else {
            continue;
        };

        if line != line_name {
            continue;
        }

        if can_access(&actor, &operation.policy) {
            println!("{}", integration_history_message(repo, &operation)?);
        } else if let Some(public_message) = operation.public_message {
            println!("{}", public_message);
        }
    }

    Ok(())
}

fn integration_history_message(repo: &Repo, operation: &Operation) -> Result<String> {
    let OperationKind::IntegrateLine {
        line,
        change_id,
        snapshot_id,
    } = &operation.kind
    else {
        return Ok(operation.private_message.clone());
    };
    let change = read_change(repo, change_id)?;

    Ok(format!(
        "integrated {} ({}) snapshot:{} into {}",
        change.name, change.id, snapshot_id, line
    ))
}

fn print_snapshot_summary(snapshot: &Snapshot, actor: &Actor) {
    let (_, hidden) = visible_files_with_hidden(snapshot.files.clone(), actor);

    println!("snapshot: {}", snapshot.id);
    println!("change: {}", snapshot.change_id);
    println!(
        "parent: {}",
        snapshot.parent_snapshot.as_deref().unwrap_or("none")
    );
    println!("domains: {}", snapshot.policy.domains.join(","));
    println!("message: {}", snapshot.message);
    println!("hidden files: {hidden}");
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
            println!("{}", public_message);
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

#[cfg(test)]
fn operation_visible_message<'a>(operation: &'a Operation, actor: &Actor) -> Option<&'a str> {
    if can_access(actor, &operation.policy) {
        Some(operation.private_message.as_str())
    } else {
        operation.public_message.as_deref()
    }
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

fn permissioned_diff(previous: Vec<FileEntry>, current: Vec<FileEntry>, actor: &Actor) -> FileDiff {
    let previous = diff_input(previous, actor);
    let current = diff_input(current, actor);
    let hidden = hidden_changed_paths(&previous.hidden_by_path, &current.hidden_by_path);

    diff_files(previous.visible, current.visible, hidden)
}

fn diff_input(files: Vec<FileEntry>, actor: &Actor) -> DiffInput {
    let mut visible = Vec::new();
    let mut hidden_by_path = BTreeMap::new();

    for file in files {
        if can_access(actor, &file.policy) {
            visible.push(file);
        } else {
            hidden_by_path.insert(file.path, file.hash);
        }
    }

    DiffInput {
        visible,
        hidden_by_path,
    }
}

fn hidden_changed_paths(
    previous: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> usize {
    let previous_paths = previous.keys().cloned().collect::<BTreeSet<_>>();
    let current_paths = current.keys().cloned().collect::<BTreeSet<_>>();
    let all_paths = previous_paths
        .union(&current_paths)
        .cloned()
        .collect::<Vec<_>>();

    all_paths
        .into_iter()
        .filter(|path| previous.get(path) != current.get(path))
        .count()
}

fn plan_merge(base: Vec<FileEntry>, line: Vec<FileEntry>, incoming: Vec<FileEntry>) -> MergePlan {
    let base = manifest_map(base);
    let line = manifest_map(line);
    let incoming = manifest_map(incoming);
    let base_paths = base.keys().cloned().collect::<BTreeSet<_>>();
    let line_paths = line.keys().cloned().collect::<BTreeSet<_>>();
    let incoming_paths = incoming.keys().cloned().collect::<BTreeSet<_>>();
    let all_paths = base_paths
        .union(&line_paths)
        .cloned()
        .collect::<BTreeSet<_>>()
        .union(&incoming_paths)
        .cloned()
        .collect::<Vec<_>>();

    let mut merged_files = Vec::new();
    let mut conflicts = Vec::new();

    for path in all_paths {
        let base_entry = base.get(&path);
        let line_entry = line.get(&path);
        let incoming_entry = incoming.get(&path);

        if same_file(line_entry, incoming_entry) {
            if let Some(file) = line_entry {
                merged_files.push(file.clone());
            }
            continue;
        }

        let line_changed = !same_file(base_entry, line_entry);
        let incoming_changed = !same_file(base_entry, incoming_entry);

        match (line_changed, incoming_changed) {
            (false, true) => {
                if let Some(file) = incoming_entry {
                    merged_files.push(file.clone());
                }
            }
            (true, false) => {
                if let Some(file) = line_entry {
                    merged_files.push(file.clone());
                }
            }
            (false, false) => {
                if let Some(file) = base_entry {
                    merged_files.push(file.clone());
                }
            }
            (true, true) => {
                conflicts.push(PendingConflict {
                    path: path.clone(),
                    kind: conflict_kind_for_entries(base_entry, line_entry, incoming_entry),
                    policy: combined_policy(line_entry, incoming_entry, base_entry),
                    file_policies: present_file_policies(line_entry, incoming_entry, base_entry),
                });
            }
        }
    }

    merged_files.sort_by(|a, b| a.path.cmp(&b.path));

    MergePlan {
        merged_files,
        conflicts,
    }
}

fn same_file(left: Option<&FileEntry>, right: Option<&FileEntry>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left.hash == right.hash,
        _ => false,
    }
}

fn conflict_kind_for_entries(
    base: Option<&FileEntry>,
    line: Option<&FileEntry>,
    incoming: Option<&FileEntry>,
) -> ConflictKind {
    match (base, line, incoming) {
        (None, Some(_), Some(_)) => ConflictKind::AddAdd,
        (Some(_), None, Some(_)) | (Some(_), Some(_), None) => ConflictKind::DeleteModify,
        _ => ConflictKind::BothModified,
    }
}

fn combined_policy(
    primary: Option<&FileEntry>,
    secondary: Option<&FileEntry>,
    fallback: Option<&FileEntry>,
) -> AccessPolicy {
    let mut domains = BTreeSet::new();

    for entry in [primary, secondary, fallback].into_iter().flatten() {
        for domain in &entry.policy.domains {
            domains.insert(domain.clone());
        }
    }

    if domains.is_empty() {
        return public_policy();
    }

    AccessPolicy {
        domains: domains.into_iter().collect(),
        redaction: Redaction::Placeholder,
    }
}

fn present_file_policies(
    primary: Option<&FileEntry>,
    secondary: Option<&FileEntry>,
    fallback: Option<&FileEntry>,
) -> Vec<AccessPolicy> {
    let mut policies = Vec::new();

    for policy in [primary, secondary, fallback]
        .into_iter()
        .flatten()
        .map(|entry| entry.policy.clone())
    {
        if !policies.contains(&policy) {
            policies.push(policy);
        }
    }

    policies
}

fn read_optional_snapshot(repo: &Repo, snapshot_id: Option<&str>) -> Result<Option<Snapshot>> {
    snapshot_id.map(|id| read_snapshot(repo, id)).transpose()
}

fn optional_snapshot_files(snapshot: &Option<Snapshot>) -> Vec<FileEntry> {
    snapshot
        .as_ref()
        .map(|snapshot| snapshot.files.clone())
        .unwrap_or_default()
}

fn store_conflicts(
    repo: &Repo,
    line: &Line,
    change: &Change,
    incoming: &Snapshot,
    source_policy: AccessPolicy,
    conflicts: Vec<PendingConflict>,
) -> Result<Vec<Conflict>> {
    let mut stored = Vec::new();

    for pending in conflicts {
        if let Some(existing) =
            find_existing_unresolved_conflict(repo, line, change, incoming, &pending.path)?
        {
            let refreshed =
                refresh_conflict(existing, change, incoming, source_policy.clone(), pending);
            write_json(&conflict_path(repo, &refreshed.id), &refreshed)?;
            stored.push(refreshed);
            continue;
        }

        let policy = combined_access_policy([
            pending.policy.clone(),
            change.policy.clone(),
            line.policy.clone(),
        ]);
        let conflict = Conflict {
            id: format!("conf_{}", short_id()),
            line: line.name.clone(),
            change_id: change.id.clone(),
            base_snapshot: change.base_snapshot.clone(),
            line_snapshot: line.head_snapshot.clone(),
            incoming_snapshot: incoming.id.clone(),
            path: pending.path,
            kind: pending.kind,
            policy,
            line_policy: line.policy.clone(),
            change_policy: change.policy.clone(),
            file_policy: pending.policy,
            file_policies: pending.file_policies,
            source_policy: source_policy.clone(),
            status: ConflictStatus::Unresolved,
            created_at: now()?,
        };
        write_json(&conflict_path(repo, &conflict.id), &conflict)?;
        record_operation(
            repo,
            OperationKind::CreateConflict {
                conflict_id: conflict.id.clone(),
                change_id: change.id.clone(),
            },
            admin_policy(),
            format!("created merge conflict `{}`", conflict.id),
            None,
        )?;
        stored.push(conflict);
    }

    Ok(stored)
}

fn refresh_conflict(
    mut conflict: Conflict,
    change: &Change,
    incoming: &Snapshot,
    source_policy: AccessPolicy,
    pending: PendingConflict,
) -> Conflict {
    conflict.incoming_snapshot = incoming.id.clone();
    conflict.kind = pending.kind;
    conflict.policy = combined_access_policy([
        pending.policy.clone(),
        change.policy.clone(),
        conflict.line_policy.clone(),
    ]);
    conflict.change_policy = change.policy.clone();
    conflict.file_policy = pending.policy;
    conflict.file_policies = pending.file_policies;
    conflict.source_policy = source_policy;
    conflict
}

fn find_existing_unresolved_conflict(
    repo: &Repo,
    line: &Line,
    change: &Change,
    _incoming: &Snapshot,
    path: &str,
) -> Result<Option<Conflict>> {
    let conflicts = read_dir_json::<Conflict>(&repo.path(&["conflicts"]))?;

    Ok(conflicts.into_iter().find(|conflict| {
        conflict.status == ConflictStatus::Unresolved
            && conflict.line == line.name
            && conflict.change_id == change.id
            && conflict.base_snapshot == change.base_snapshot
            && conflict.line_snapshot == line.head_snapshot
            && conflict.path == path
    }))
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

fn read_conflict(repo: &Repo, id: &str) -> Result<Conflict> {
    read_json(&conflict_path(repo, id)).with_context(|| format!("conflict `{id}` not found"))
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

fn conflict_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["conflicts", &format!("{id}.json")])
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

fn can_access_conflict(actor: &Actor, conflict: &Conflict) -> bool {
    let file_policies = if conflict.file_policies.is_empty() {
        vec![conflict.file_policy.clone()]
    } else {
        conflict.file_policies.clone()
    };

    can_access(actor, &conflict.line_policy)
        && can_access(actor, &conflict.change_policy)
        && can_access(actor, &conflict.source_policy)
        && file_policies
            .iter()
            .all(|file_policy| can_access(actor, file_policy))
}

fn can_access_pending_conflict(actor: &Actor, conflict: &PendingConflict) -> bool {
    conflict
        .file_policies
        .iter()
        .all(|file_policy| can_access(actor, file_policy))
}

fn can_access_merge_inputs<const N: usize>(actor: &Actor, file_sets: [&[FileEntry]; N]) -> bool {
    file_sets
        .iter()
        .flat_map(|files| files.iter())
        .all(|file| can_access(actor, &file.policy))
}

fn can_access_merge_sources<const S: usize, const F: usize>(
    actor: &Actor,
    snapshots: [&Option<Snapshot>; S],
    file_sets: [&[FileEntry]; F],
) -> bool {
    snapshots.iter().all(|snapshot| {
        snapshot
            .as_ref()
            .map(|snapshot| can_access(actor, &snapshot.policy))
            .unwrap_or(true)
    }) && can_access_merge_inputs(actor, file_sets)
}

fn merge_source_policy(
    line_policy: &AccessPolicy,
    change_policy: &AccessPolicy,
    incoming: &Snapshot,
    base_snapshot: &Option<Snapshot>,
    line_snapshot: &Option<Snapshot>,
) -> AccessPolicy {
    integration_metadata_policy([
        line_policy,
        change_policy,
        &incoming.policy,
        base_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.policy)
            .unwrap_or(line_policy),
        line_snapshot
            .as_ref()
            .map(|snapshot| &snapshot.policy)
            .unwrap_or(line_policy),
    ])
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

fn combined_access_policy<const N: usize>(policies: [AccessPolicy; N]) -> AccessPolicy {
    let mut domains = BTreeSet::new();

    for policy in policies {
        for domain in policy.domains {
            domains.insert(domain);
        }
    }

    if domains.is_empty() {
        return public_policy();
    }

    AccessPolicy {
        domains: domains.into_iter().collect(),
        redaction: Redaction::Placeholder,
    }
}

fn policy_from_domains(domains: Vec<String>) -> AccessPolicy {
    AccessPolicy {
        domains: normalize_domains(domains),
        redaction: Redaction::Omit,
    }
}

fn integration_metadata_policy<const N: usize>(policies: [&AccessPolicy; N]) -> AccessPolicy {
    if policies
        .iter()
        .all(|policy| policy.domains == [PUBLIC_DOMAIN])
    {
        public_policy()
    } else {
        admin_policy()
    }
}

fn default_line_name() -> String {
    DEFAULT_LINE.to_string()
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
        OperationKind::CreateConflict { .. } => "create_conflict",
    }
}

fn conflict_kind(kind: &ConflictKind) -> &'static str {
    match kind {
        ConflictKind::BothModified => "both_modified",
        ConflictKind::DeleteModify => "delete_modify",
        ConflictKind::AddAdd => "add_add",
    }
}

fn conflict_status(status: &ConflictStatus) -> &'static str {
    match status {
        ConflictStatus::Unresolved => "unresolved",
        ConflictStatus::Resolved => "resolved",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(name: &str, domains: &[&str]) -> Actor {
        Actor {
            name: name.to_string(),
            domains: domains.iter().map(|domain| domain.to_string()).collect(),
        }
    }

    fn policy(domains: &[&str]) -> AccessPolicy {
        AccessPolicy {
            domains: domains.iter().map(|domain| domain.to_string()).collect(),
            redaction: Redaction::Omit,
        }
    }

    fn file(path: &str, hash: &str, domains: &[&str]) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            hash: hash.to_string(),
            bytes: 1,
            policy: policy(domains),
        }
    }

    fn operation(
        domains: &[&str],
        private_message: &str,
        public_message: Option<&str>,
    ) -> Operation {
        Operation {
            id: "op_test".to_string(),
            kind: OperationKind::InitRepo,
            policy: policy(domains),
            private_message: private_message.to_string(),
            public_message: public_message.map(str::to_string),
            created_at: 0,
        }
    }

    #[test]
    fn normalize_domains_defaults_to_public() {
        assert_eq!(
            normalize_domains(Vec::new()),
            vec![PUBLIC_DOMAIN.to_string()]
        );
        assert_eq!(
            normalize_domains(vec!["".to_string(), "  ".to_string()]),
            vec![PUBLIC_DOMAIN.to_string()]
        );
    }

    #[test]
    fn normalize_domains_trims_sorts_and_deduplicates() {
        assert_eq!(
            normalize_domains(vec![
                " team/security ".to_string(),
                "public".to_string(),
                "team/security".to_string(),
            ]),
            vec!["public".to_string(), "team/security".to_string()]
        );
    }

    #[test]
    fn can_access_allows_domain_intersection() {
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);

        assert!(can_access(&bob, &policy(&[PUBLIC_DOMAIN])));
        assert!(can_access(&bob, &policy(&["team/security"])));
        assert!(!can_access(&bob, &policy(&[ADMIN_DOMAIN])));
    }

    #[test]
    fn admin_domain_can_access_everything() {
        let admin = actor("admin", &[PUBLIC_DOMAIN, ADMIN_DOMAIN]);

        assert!(can_access(&admin, &policy(&[PUBLIC_DOMAIN])));
        assert!(can_access(&admin, &policy(&["team/security"])));
        assert!(can_access(&admin, &policy(&["customer/acme"])));
    }

    #[test]
    fn policy_for_path_uses_public_when_no_policy_matches() {
        let policies = vec![PathPolicy {
            prefix: "security".to_string(),
            policy: policy(&["team/security"]),
        }];

        assert_eq!(policy_for_path("src/app.ts", &policies), public_policy());
    }

    #[test]
    fn policy_for_path_matches_exact_path_and_descendants() {
        let policies = vec![PathPolicy {
            prefix: "security".to_string(),
            policy: policy(&["team/security"]),
        }];

        assert_eq!(
            policy_for_path("security", &policies),
            policy(&["team/security"])
        );
        assert_eq!(
            policy_for_path("security/repro.test.ts", &policies),
            policy(&["team/security"])
        );
        assert_eq!(
            policy_for_path("security-notes.md", &policies),
            public_policy()
        );
    }

    #[test]
    fn policy_for_path_prefers_longest_matching_prefix() {
        let policies = vec![
            PathPolicy {
                prefix: "security".to_string(),
                policy: policy(&["team/security"]),
            },
            PathPolicy {
                prefix: "security/prod".to_string(),
                policy: policy(&[ADMIN_DOMAIN]),
            },
        ];

        assert_eq!(
            policy_for_path("security/prod/secrets.env", &policies),
            policy(&[ADMIN_DOMAIN])
        );
    }

    #[test]
    fn visible_files_filters_restricted_entries_and_counts_hidden() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let files = vec![
            file("src/app.ts", "a", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "b", &["team/security"]),
            file(".env", "c", &[ADMIN_DOMAIN]),
        ];

        let (visible, hidden) = visible_files_with_hidden(files, &alice);

        assert_eq!(hidden, 2);
        assert_eq!(
            visible
                .into_iter()
                .map(|entry| entry.path)
                .collect::<Vec<_>>(),
            vec!["src/app.ts"]
        );
    }

    #[test]
    fn visible_files_allows_security_actor_but_not_admin_only_file() {
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let files = vec![
            file("src/app.ts", "a", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "b", &["team/security"]),
            file(".env", "c", &[ADMIN_DOMAIN]),
        ];

        let (visible, hidden) = visible_files_with_hidden(files, &bob);

        assert_eq!(hidden, 1);
        assert_eq!(
            visible
                .into_iter()
                .map(|entry| entry.path)
                .collect::<Vec<_>>(),
            vec!["src/app.ts", "security/repro.test.ts"]
        );
    }

    #[test]
    fn diff_files_tracks_added_modified_deleted_and_hidden_count() {
        let previous = vec![
            file("deleted.ts", "a", &[PUBLIC_DOMAIN]),
            file("modified.ts", "before", &[PUBLIC_DOMAIN]),
            file("same.ts", "same", &[PUBLIC_DOMAIN]),
        ];
        let current = vec![
            file("added.ts", "b", &[PUBLIC_DOMAIN]),
            file("modified.ts", "after", &[PUBLIC_DOMAIN]),
            file("same.ts", "same", &[PUBLIC_DOMAIN]),
        ];

        let diff = diff_files(previous, current, 2);

        assert_eq!(diff.added, vec!["added.ts"]);
        assert_eq!(diff.modified, vec!["modified.ts"]);
        assert_eq!(diff.deleted, vec!["deleted.ts"]);
        assert_eq!(diff.hidden, 2);
    }

    #[test]
    fn permissioned_diff_hides_restricted_path_names_but_counts_changes() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let previous = vec![
            file("src/app.ts", "old", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "old-secret", &["team/security"]),
        ];
        let current = vec![
            file("src/app.ts", "new", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "new-secret", &["team/security"]),
            file(".env", "env", &[ADMIN_DOMAIN]),
        ];

        let diff = permissioned_diff(previous, current, &alice);

        assert_eq!(diff.added, Vec::<String>::new());
        assert_eq!(diff.modified, vec!["src/app.ts"]);
        assert_eq!(diff.deleted, Vec::<String>::new());
        assert_eq!(diff.hidden, 2);
    }

    #[test]
    fn permissioned_diff_shows_security_file_to_security_actor_but_hides_admin_file() {
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let previous = vec![
            file("src/app.ts", "old", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "old-secret", &["team/security"]),
        ];
        let current = vec![
            file("src/app.ts", "new", &[PUBLIC_DOMAIN]),
            file("security/repro.test.ts", "new-secret", &["team/security"]),
            file(".env", "env", &[ADMIN_DOMAIN]),
        ];

        let diff = permissioned_diff(previous, current, &bob);

        assert_eq!(diff.added, Vec::<String>::new());
        assert_eq!(diff.modified, vec!["security/repro.test.ts", "src/app.ts"]);
        assert_eq!(diff.deleted, Vec::<String>::new());
        assert_eq!(diff.hidden, 1);
    }

    #[test]
    fn hidden_changed_paths_counts_added_deleted_and_modified_hidden_files() {
        let previous = BTreeMap::from([
            ("deleted.secret".to_string(), "old".to_string()),
            ("modified.secret".to_string(), "old".to_string()),
            ("same.secret".to_string(), "same".to_string()),
        ]);
        let current = BTreeMap::from([
            ("added.secret".to_string(), "new".to_string()),
            ("modified.secret".to_string(), "new".to_string()),
            ("same.secret".to_string(), "same".to_string()),
        ]);

        assert_eq!(hidden_changed_paths(&previous, &current), 3);
    }

    #[test]
    fn plan_merge_combines_non_overlapping_changes() {
        let base = vec![file("app.ts", "base", &[PUBLIC_DOMAIN])];
        let line = vec![
            file("app.ts", "base", &[PUBLIC_DOMAIN]),
            file("line.ts", "line", &[PUBLIC_DOMAIN]),
        ];
        let incoming = vec![
            file("app.ts", "incoming", &[PUBLIC_DOMAIN]),
            file("feature.ts", "feature", &[PUBLIC_DOMAIN]),
        ];

        let plan = plan_merge(base, line, incoming);

        assert!(plan.conflicts.is_empty());
        assert_eq!(
            plan.merged_files
                .into_iter()
                .map(|entry| (entry.path, entry.hash))
                .collect::<Vec<_>>(),
            vec![
                ("app.ts".to_string(), "incoming".to_string()),
                ("feature.ts".to_string(), "feature".to_string()),
                ("line.ts".to_string(), "line".to_string()),
            ]
        );
    }

    #[test]
    fn plan_merge_detects_both_modified_conflict() {
        let base = vec![file("app.ts", "base", &[PUBLIC_DOMAIN])];
        let line = vec![file("app.ts", "line", &[PUBLIC_DOMAIN])];
        let incoming = vec![file("app.ts", "incoming", &[PUBLIC_DOMAIN])];

        let plan = plan_merge(base, line, incoming);

        assert!(plan.merged_files.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].path, "app.ts");
        assert_eq!(plan.conflicts[0].kind, ConflictKind::BothModified);
    }

    #[test]
    fn plan_merge_detects_delete_modify_conflict() {
        let base = vec![file("app.ts", "base", &[PUBLIC_DOMAIN])];
        let line = Vec::new();
        let incoming = vec![file("app.ts", "incoming", &[PUBLIC_DOMAIN])];

        let plan = plan_merge(base, line, incoming);

        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::DeleteModify);
    }

    #[test]
    fn plan_merge_detects_add_add_conflict() {
        let base = Vec::new();
        let line = vec![file("app.ts", "line", &[PUBLIC_DOMAIN])];
        let incoming = vec![file("app.ts", "incoming", &[PUBLIC_DOMAIN])];

        let plan = plan_merge(base, line, incoming);

        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, ConflictKind::AddAdd);
    }

    #[test]
    fn plan_merge_conflict_policy_combines_domains() {
        let base = vec![file("security/repro.ts", "base", &["team/security"])];
        let line = vec![file("security/repro.ts", "line", &["team/security"])];
        let incoming = vec![file("security/repro.ts", "incoming", &[ADMIN_DOMAIN])];

        let plan = plan_merge(base, line, incoming);

        assert_eq!(
            plan.conflicts[0].policy.domains,
            vec![ADMIN_DOMAIN.to_string(), "team/security".to_string()]
        );
    }

    #[test]
    fn present_file_policies_deduplicates_equivalent_policies() {
        let base = file("app.ts", "base", &[PUBLIC_DOMAIN]);
        let line = file("app.ts", "line", &[PUBLIC_DOMAIN]);
        let incoming = file("app.ts", "incoming", &["team/security"]);

        assert_eq!(
            present_file_policies(Some(&base), Some(&line), Some(&incoming)),
            vec![policy(&[PUBLIC_DOMAIN]), policy(&["team/security"])]
        );
    }

    #[test]
    fn combined_access_policy_includes_file_change_and_line_domains() {
        let combined = combined_access_policy([
            policy(&[PUBLIC_DOMAIN]),
            policy(&["team/security"]),
            policy(&["release/private"]),
        ]);

        assert_eq!(
            combined.domains,
            vec![
                "public".to_string(),
                "release/private".to_string(),
                "team/security".to_string(),
            ]
        );
    }

    #[test]
    fn integration_metadata_policy_is_public_only_when_line_and_change_are_public() {
        assert_eq!(
            integration_metadata_policy([&policy(&[PUBLIC_DOMAIN]), &policy(&[PUBLIC_DOMAIN])]),
            public_policy()
        );
        assert_eq!(
            integration_metadata_policy([&policy(&["team/security"]), &policy(&[PUBLIC_DOMAIN])]),
            admin_policy()
        );
        assert_eq!(
            integration_metadata_policy([&policy(&[PUBLIC_DOMAIN]), &policy(&["team/security"])]),
            admin_policy()
        );
        assert_eq!(
            integration_metadata_policy([
                &policy(&[PUBLIC_DOMAIN]),
                &policy(&[PUBLIC_DOMAIN]),
                &policy(&["team/security"]),
            ]),
            admin_policy()
        );
    }

    #[test]
    fn can_access_conflict_requires_line_change_and_file_access() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let conflict = Conflict {
            id: "conf_test".to_string(),
            line: DEFAULT_LINE.to_string(),
            change_id: "chg_test".to_string(),
            base_snapshot: None,
            line_snapshot: None,
            incoming_snapshot: "snap_test".to_string(),
            path: "src/app.ts".to_string(),
            kind: ConflictKind::BothModified,
            policy: combined_access_policy([policy(&[PUBLIC_DOMAIN]), policy(&["team/security"])]),
            line_policy: policy(&[PUBLIC_DOMAIN]),
            change_policy: policy(&["team/security"]),
            file_policy: policy(&[PUBLIC_DOMAIN]),
            file_policies: vec![policy(&[PUBLIC_DOMAIN])],
            source_policy: policy(&[PUBLIC_DOMAIN]),
            status: ConflictStatus::Unresolved,
            created_at: 0,
        };

        assert!(!can_access_conflict(&alice, &conflict));
        assert!(can_access_conflict(&bob, &conflict));
    }

    #[test]
    fn can_access_conflict_requires_every_file_side() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let conflict = Conflict {
            id: "conf_test".to_string(),
            line: DEFAULT_LINE.to_string(),
            change_id: "chg_test".to_string(),
            base_snapshot: None,
            line_snapshot: None,
            incoming_snapshot: "snap_test".to_string(),
            path: "src/app.ts".to_string(),
            kind: ConflictKind::BothModified,
            policy: combined_access_policy([policy(&[PUBLIC_DOMAIN]), policy(&["team/security"])]),
            line_policy: policy(&[PUBLIC_DOMAIN]),
            change_policy: policy(&[PUBLIC_DOMAIN]),
            file_policy: policy(&[PUBLIC_DOMAIN]),
            file_policies: vec![policy(&[PUBLIC_DOMAIN]), policy(&["team/security"])],
            source_policy: policy(&[PUBLIC_DOMAIN]),
            status: ConflictStatus::Unresolved,
            created_at: 0,
        };

        assert!(!can_access_conflict(&alice, &conflict));
        assert!(can_access_conflict(&bob, &conflict));
    }

    #[test]
    fn pending_conflict_requires_every_file_side() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let plan = plan_merge(
            vec![file("app.ts", "base", &[PUBLIC_DOMAIN])],
            vec![file("app.ts", "line", &[PUBLIC_DOMAIN])],
            vec![file("app.ts", "incoming", &["team/security"])],
        );

        assert_eq!(plan.conflicts.len(), 1);
        assert!(!can_access_pending_conflict(&alice, &plan.conflicts[0]));
        assert!(can_access_pending_conflict(&bob, &plan.conflicts[0]));
    }

    #[test]
    fn merge_inputs_require_access_to_every_source_file() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let public_files = vec![file("app.ts", "base", &[PUBLIC_DOMAIN])];
        let restricted_files = vec![file("security/repro.ts", "secret", &["team/security"])];

        assert!(!can_access_merge_inputs(
            &alice,
            [&public_files, &restricted_files]
        ));
        assert!(can_access_merge_inputs(
            &bob,
            [&public_files, &restricted_files]
        ));
    }

    #[test]
    fn operation_visible_message_returns_private_message_for_authorized_actor() {
        let bob = actor("bob", &[PUBLIC_DOMAIN, "team/security"]);
        let operation = operation(
            &["team/security"],
            "created change `fix-token-replay`",
            Some("integrated restricted change into `main`"),
        );

        assert_eq!(
            operation_visible_message(&operation, &bob),
            Some("created change `fix-token-replay`")
        );
    }

    #[test]
    fn operation_visible_message_returns_public_message_for_redacted_actor() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let operation = operation(
            &["team/security"],
            "created change `fix-token-replay`",
            Some("integrated restricted change into `main`"),
        );

        assert_eq!(
            operation_visible_message(&operation, &alice),
            Some("integrated restricted change into `main`")
        );
    }

    #[test]
    fn operation_visible_message_hides_operation_without_public_redaction() {
        let alice = actor("alice", &[PUBLIC_DOMAIN]);
        let operation = operation(&[ADMIN_DOMAIN], "set actor `bob`", None);

        assert_eq!(operation_visible_message(&operation, &alice), None);
    }

    #[test]
    fn should_scan_ignores_source_control_and_dependency_directories() {
        assert!(!should_scan(Path::new(".git")));
        assert!(!should_scan(Path::new(".rgit")));
        assert!(!should_scan(Path::new("target")));
        assert!(!should_scan(Path::new("node_modules")));
        assert!(should_scan(Path::new("src")));
    }
}
