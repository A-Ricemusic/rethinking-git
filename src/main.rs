use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

mod ancestry;
mod backup;
mod checkout;
mod checkout_paths;
mod cli_failure;
mod git_bridge;
mod git_objects;
mod git_remotes;
mod identity;
mod ignore_rules;
mod initialization;
mod lines;
mod resolution;
mod status_json;
mod text_diff;
mod text_merge;
mod transaction;
mod tree_conflicts;
mod verify;

use cli_failure::CliFailure;

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
    /// Configure author metadata for future native snapshots (not authentication).
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },
    /// Exchange saved history with Git (requires Git installed).
    Git {
        #[command(subcommand)]
        command: GitCommand,
    },
    /// Inspect repository integrity.
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    /// Initialize source control in the current directory.
    Init {
        /// Continue an interrupted initialization without replacing saved history.
        #[arg(long)]
        resume: bool,
    },
    /// Show changed files since the current change's latest snapshot.
    Status {
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        /// Emit one versioned JSON status document for automation.
        #[arg(long)]
        json: bool,
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
enum IdentityCommand {
    Set { name: String, email: String },
    Show,
}

#[derive(clap::Args)]
struct GitPushArgs {
    remote: String,
    #[arg(long, default_value = DEFAULT_LINE)]
    line: String,
    #[arg(long, default_value = DEFAULT_LINE)]
    branch: String,
    #[arg(long)]
    author: Option<String>,
    #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
    as_actor: String,
    #[arg(long)]
    allow_restricted: bool,
}

#[derive(clap::Args)]
struct GitCloneArgs {
    remote: String,
    destination: PathBuf,
    #[arg(long, default_value = DEFAULT_LINE)]
    branch: String,
    #[arg(long = "domain", default_value = ADMIN_DOMAIN)]
    domains: Vec<String>,
    /// Resume the matching incomplete clone, preserving later working edits.
    #[arg(long)]
    resume: bool,
}

#[derive(Subcommand)]
enum GitCommand {
    /// Clone a Git branch into a new native repository and working directory.
    Clone(GitCloneArgs),
    /// Fetch a Git branch using configured Git/SSH credentials.
    Fetch {
        remote: String,
        #[arg(long, default_value = DEFAULT_LINE)]
        branch: String,
        #[arg(long)]
        into: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        #[arg(long = "domain", default_value = ADMIN_DOMAIN)]
        domains: Vec<String>,
    },
    /// Publish a line with Git's normal fast-forward and server authorization checks.
    Push(GitPushArgs),
    /// Import a local Git revision and its complete ancestry into an empty line.
    Import {
        source: PathBuf,
        #[arg(long, default_value = "HEAD")]
        revision: String,
        #[arg(long, default_value = DEFAULT_LINE)]
        into: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        #[arg(long = "domain", default_value = ADMIN_DOMAIN)]
        domains: Vec<String>,
    },
    /// Export a line and its ancestry to a new bare Git repository.
    Export {
        destination: PathBuf,
        #[arg(long, default_value = DEFAULT_LINE)]
        line: String,
        /// Identity for native snapshots, e.g. Name <email@example.com>.
        #[arg(long)]
        author: Option<String>,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        /// Explicitly export restricted contents without their rgit access policies.
        #[arg(long)]
        allow_restricted: bool,
    },
}

#[derive(Subcommand)]
enum RepoCommand {
    /// Create a verified copy of saved history in a new directory.
    Backup {
        destination: PathBuf,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Verify saved records, references, manifests and blobs without repairing them.
    Verify {
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
}

#[derive(Subcommand)]
enum ChangeCommand {
    /// Retarget the current change to another line without rewriting its snapshots.
    Retarget {
        line: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
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
    /// Create a line at another line's saved head, preserving its policy.
    Create {
        name: String,
        #[arg(long, default_value = DEFAULT_LINE)]
        from: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Move a saved line head with a compare-and-swap guard; working files are unchanged.
    Reset {
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        expected_head: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
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
        /// Show unified text hunks, modes and explicit binary/large-file notices.
        #[arg(long)]
        patch: bool,
    },
    /// Diff two snapshots.
    Snapshot {
        old_snapshot: String,
        new_snapshot: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        /// Show unified text hunks, modes and explicit binary/large-file notices.
        #[arg(long)]
        patch: bool,
    },
    /// Diff a line's parent snapshot against its head snapshot.
    Line {
        /// Line to diff.
        #[arg(default_value = DEFAULT_LINE)]
        line: String,
        /// Actor whose permissioned view should be used.
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
        /// Show unified text hunks, modes and explicit binary/large-file notices.
        #[arg(long)]
        patch: bool,
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
    /// Resolve against the exact recorded source snapshots.
    Resolve {
        conflict_id: String,
        #[arg(
            long,
            value_enum,
            required_unless_present = "from_working",
            conflicts_with = "from_working"
        )]
        take: Option<Resolution>,
        /// Capture the conflict path's current working contents as a custom resolution.
        #[arg(long)]
        from_working: bool,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
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
    /// Check out an existing change, preserving untracked files and refusing dirty tracked files.
    Switch {
        change_id: String,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Restore tracked files from the current snapshot, or another snapshot.
    Restore {
        #[arg(long)]
        from: Option<String>,
        /// Explicitly discard modifications to tracked files.
        #[arg(long)]
        discard_changes: bool,
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
    /// Show the current workspace state visible to an actor.
    Info {
        #[arg(long = "as", default_value = PUBLIC_DOMAIN)]
        as_actor: String,
    },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
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
    /// Saved logical modes for the actual workspace, independent of change ancestry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode_snapshot: Option<String>,
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

impl Change {
    fn workspace_base_snapshot_id(&self) -> Option<&str> {
        self.current_snapshot
            .as_deref()
            .or(self.base_snapshot.as_deref())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<git_objects::CommitMetadata>,
    id: String,
    change_id: String,
    parent_snapshot: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    merge_parents: Vec<String>,
    message: String,
    manifest_hash: String,
    files: Vec<FileEntry>,
    policy: AccessPolicy,
    created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileEntry {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    symlink: bool,
    path: String,
    #[serde(default, skip_serializing_if = "is_false")]
    executable: bool,
    hash: String,
    bytes: u64,
    policy: AccessPolicy,
}

impl FileEntry {
    fn flags(&self) -> transaction::WorkingFlags {
        transaction::WorkingFlags {
            executable: self.executable,
            symlink: self.symlink,
        }
    }
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
    FileDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
enum Resolution {
    #[value(skip)]
    Custom,
    Base,
    Line,
    Incoming,
    Delete,
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
    #[serde(default)]
    resolution: Option<Resolution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacement: Option<FileEntry>,
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
    SetIdentity,
    CreateLine {
        line: String,
        source_line: String,
        snapshot_id: Option<String>,
    },
    ResetLine {
        line: String,
        previous_snapshot: String,
        snapshot_id: String,
    },
    BindGitIdentity {
        snapshot_id: String,
        object_id: String,
    },
    RetargetChange {
        change_id: String,
        line: String,
    },
    ImportGit {
        line: String,
        change_id: String,
        snapshot_id: String,
    },
    SwitchWorkspace {
        change_id: String,
    },
    RestoreWorkspace {
        snapshot_id: Option<String>,
    },
    ResolveConflict {
        conflict_id: String,
    },
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
    transaction: transaction::CommandTransaction,
}

#[derive(Serialize)]
struct FileDiff {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
    #[serde(rename = "hidden_count")]
    hidden: usize,
}

struct DiffInput {
    visible: Vec<FileEntry>,
    hidden_by_path: BTreeMap<String, FileEntry>,
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

    if let Command::Init { resume } = &cli.command {
        return initialization::initialize(std::env::current_dir()?, *resume).map(|_| ());
    }
    if let Command::Git {
        command: GitCommand::Clone(args),
    } = &cli.command
    {
        return git_remotes::clone_repository(args);
    }
    let repo = Repo::discover()?;
    let result = match cli.command {
        Command::Identity {
            command: IdentityCommand::Set { name, email },
        } => identity::set(&repo, &name, &email),
        Command::Identity {
            command: IdentityCommand::Show,
        } => identity::show(&repo),
        Command::Init { .. }
        | Command::Git {
            command: GitCommand::Clone(_),
        } => unreachable!(),
        Command::Git {
            command:
                GitCommand::Fetch {
                    remote,
                    branch,
                    into,
                    as_actor,
                    domains,
                },
        } => git_remotes::fetch(
            &repo,
            &remote,
            &branch,
            &into,
            &as_actor,
            policy_from_domains(domains),
        ),
        Command::Git {
            command: GitCommand::Push(args),
        } => git_remotes::push(&repo, &args),
        Command::Git {
            command:
                GitCommand::Import {
                    source,
                    revision,
                    into,
                    as_actor,
                    domains,
                },
        } => git_bridge::import(
            &repo,
            &source,
            &revision,
            &into,
            &as_actor,
            policy_from_domains(domains),
        ),
        Command::Git {
            command:
                GitCommand::Export {
                    destination,
                    line,
                    author,
                    as_actor,
                    allow_restricted,
                },
        } => git_bridge::export(
            &repo,
            &destination,
            &line,
            author.as_deref(),
            &as_actor,
            allow_restricted,
            false,
        ),
        Command::Repo {
            command:
                RepoCommand::Backup {
                    destination,
                    as_actor,
                },
        } => backup::backup(&repo, &destination, &as_actor),
        Command::Repo {
            command: RepoCommand::Verify { as_actor },
        } => verify::verify(&repo, &as_actor),
        Command::Status { as_actor, json } => status(&repo, &as_actor, json),
        Command::Snapshot { message, domains } => {
            create_snapshot(&repo, &message, policy_from_domains(domains))
        }
        Command::SnapshotInfo { command } => match command {
            SnapshotCommand::List { as_actor } => list_snapshots(&repo, &as_actor),
            SnapshotCommand::Show {
                snapshot_id,
                as_actor,
            } => show_snapshot(&repo, &snapshot_id, &as_actor),
        },
        Command::Change { command } => match command {
            ChangeCommand::Retarget { line, as_actor } => {
                git_remotes::retarget(&repo, &line, &as_actor)
            }
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
        },
        Command::Actor { command } => match command {
            ActorCommand::Set { name, domains } => set_actor(&repo, &name, domains),
            ActorCommand::List => list_actors(&repo),
        },
        Command::Access { command } => match command {
            AccessCommand::Path { path, domains } => set_path_policy(&repo, &path, domains),
            AccessCommand::List => list_path_policies(&repo),
        },
        Command::Line { command } => match command {
            LineCommand::Create {
                name,
                from,
                as_actor,
            } => lines::create(&repo, &name, &from, &as_actor),
            LineCommand::Reset {
                line,
                to,
                expected_head,
                as_actor,
            } => lines::reset(&repo, &line, &to, &expected_head, &as_actor),
            LineCommand::List { as_actor } => list_lines(&repo, &as_actor),
            LineCommand::Integrate { line, as_actor } => integrate_line(&repo, &line, &as_actor),
            LineCommand::View { line, as_actor } => view_line(&repo, &line, &as_actor),
            LineCommand::History { line, as_actor } => line_history(&repo, &line, &as_actor),
        },
        Command::Diff { command } => match command {
            DiffCommand::Workspace { as_actor, patch } => diff_workspace(&repo, &as_actor, patch),
            DiffCommand::Snapshot {
                old_snapshot,
                new_snapshot,
                as_actor,
                patch,
            } => diff_snapshots(&repo, &old_snapshot, &new_snapshot, &as_actor, patch),
            DiffCommand::Line {
                line,
                as_actor,
                patch,
            } => diff_line(&repo, &line, &as_actor, patch),
        },
        Command::Merge { command } => match command {
            MergeCommand::Preview {
                change_id,
                line,
                as_actor,
            } => merge_preview(&repo, change_id.as_deref(), &line, &as_actor),
        },
        Command::Conflict { command } => match command {
            ConflictCommand::Resolve {
                conflict_id,
                take,
                from_working,
                as_actor,
            } => resolution::resolve_conflict(
                &repo,
                &conflict_id,
                if from_working {
                    Resolution::Custom
                } else {
                    take.context("resolution choice missing")?
                },
                &as_actor,
            ),
            ConflictCommand::List { as_actor } => list_conflicts(&repo, &as_actor),
            ConflictCommand::Show {
                conflict_id,
                as_actor,
            } => show_conflict(&repo, &conflict_id, &as_actor),
        },
        Command::Workspace { command } => match command {
            WorkspaceCommand::Switch {
                change_id,
                as_actor,
            } => checkout::switch(&repo, &change_id, &as_actor),
            WorkspaceCommand::Restore {
                from,
                discard_changes,
                as_actor,
            } => checkout::restore(&repo, from.as_deref(), discard_changes, &as_actor),
            WorkspaceCommand::Info { as_actor } => workspace_info(&repo, &as_actor),
        },
        Command::Op { command } => match command {
            OpCommand::Log { as_actor } => op_log(&repo, &as_actor),
        },
    };
    if result.is_ok()
        || result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<CliFailure>())
            == Some(&CliFailure::IntegrationConflicted)
    {
        repo.transaction.commit()?;
    }
    result
}

impl Repo {
    fn discover() -> Result<Self> {
        Self::discover_from(std::env::current_dir().context("failed to read current directory")?)
    }

    fn discover_from(mut dir: PathBuf) -> Result<Self> {
        loop {
            let meta = dir.join(META_DIR);
            let exists = match fs::symlink_metadata(&meta) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            if exists {
                initialization::preflight(&meta)?;
                let transaction = transaction::CommandTransaction::open(&meta)?;
                let repo = Self {
                    root: dir,
                    meta,
                    transaction,
                };
                let config: RepoConfig = read_json(&repo, &repo.meta.join("repo.json"))
                    .context("repository configuration is missing or invalid")?;
                if config.format_version != FORMAT_VERSION {
                    bail!(
                        "unsupported repository format {}; expected {}; migration is required",
                        config.format_version,
                        FORMAT_VERSION
                    );
                }
                return Ok(repo);
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

fn initialize_repo(root: PathBuf) -> Result<Repo> {
    initialization::initialize(root, false)
}

fn set_actor(repo: &Repo, name: &str, domains: Vec<String>) -> Result<()> {
    let path = actor_path(repo, name)?;
    if path.try_exists().context("failed to inspect actor entry")? {
        // The legacy slash-to-double-underscore encoding is not injective.
        // Preserve its on-disk compatibility without allowing aliases to overwrite grants.
        read_actor(repo, name)?;
    }
    let actor = Actor {
        name: name.to_string(),
        domains: normalize_domains(domains),
    };

    write_json(repo, &actor_path(repo, name)?, &actor)?;
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
    let mut actors = read_dir_json::<Actor>(repo, &repo.path(&["actors"]))?;
    actors.sort_by(|a, b| a.name.cmp(&b.name));

    for actor in actors {
        println!("{} domains:{}", actor.name, actor.domains.join(","));
    }

    Ok(())
}

fn set_path_policy(repo: &Repo, prefix: &str, domains: Vec<String>) -> Result<()> {
    let mut policies = read_path_policies(repo)?;
    let normalized_prefix = repository_relative_path(Path::new(prefix))?;
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

    write_json(repo, &repo.path(&["path-policies.json"]), &policies)?;
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
        id: format!("chg_{}", new_id_suffix()),
        name: name.to_string(),
        base_snapshot,
        target_line: target.name,
        current_snapshot: None,
        policy: policy.clone(),
        created_at: now()?,
    };
    let mut workspace = read_workspace(repo)?;
    if workspace.mode_snapshot.is_none() {
        workspace.mode_snapshot = workspace
            .current_change
            .as_deref()
            .map(|id| read_change(repo, id))
            .transpose()?
            .and_then(|previous| previous.workspace_base_snapshot_id().map(str::to_string));
    }
    workspace.current_change = Some(change.id.clone());

    write_json(repo, &change_path(repo, &change.id)?, &change)?;
    write_json(repo, &repo.path(&["workspace.json"]), &workspace)?;
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
    let mut changes = read_dir_json::<Change>(repo, &repo.path(&["changes"]))?;
    changes.sort_by_key(|change| change.created_at);

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
            visible_snapshot_id(repo, change.current_snapshot.as_deref(), &actor)?,
            change.policy.domains.join(",")
        );
    }

    Ok(())
}

fn show_change(repo: &Repo, change_id: &str, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let change = read_change(repo, change_id).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    println!("change: {} ({})", change.name, change.id);
    println!("target line: {}", change.target_line);
    println!("domains: {}", change.policy.domains.join(","));
    println!(
        "current snapshot: {}",
        visible_snapshot_id(repo, change.current_snapshot.as_deref(), &actor)?
    );

    if let Some(snapshot_id) = change.current_snapshot.as_deref() {
        let snapshot = read_snapshot(repo, snapshot_id)?;
        print_snapshot_summary(repo, &snapshot, &actor)?;
    }

    Ok(())
}

fn create_snapshot(repo: &Repo, message: &str, requested_policy: AccessPolicy) -> Result<()> {
    let mut workspace = read_workspace(repo)?;
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
        author: identity::configured(repo)?,
        git: None,
        id: format!("snap_{}", new_id_suffix()),
        change_id: change.id.clone(),
        parent_snapshot: change.workspace_base_snapshot_id().map(str::to_string),
        merge_parents: Vec::new(),
        message: message.to_string(),
        manifest_hash,
        files,
        policy: snapshot_policy.clone(),
        created_at: now()?,
    };

    change.current_snapshot = Some(snapshot.id.clone());
    workspace.mode_snapshot = Some(snapshot.id.clone());
    write_json(repo, &repo.path(&["workspace.json"]), &workspace)?;

    write_json(repo, &snapshot_path(repo, &snapshot.id)?, &snapshot)?;
    write_json(repo, &change_path(repo, &change.id)?, &change)?;
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
    let mut snapshots = read_dir_json::<Snapshot>(repo, &repo.path(&["snapshots"]))?;
    snapshots.sort_by_key(|snapshot| snapshot.created_at);

    for snapshot in snapshots {
        if !can_access(&actor, &snapshot.policy) {
            continue;
        }

        let (visible, hidden) = visible_files_with_hidden(snapshot.files, &actor);
        println!(
            "{} change:{} files:{} hidden:{} domains:{} message:{}",
            snapshot.id,
            visible_change_id(repo, &snapshot.change_id, &actor)?,
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
    let snapshot = read_snapshot(repo, snapshot_id).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &snapshot.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    print_snapshot_summary(repo, &snapshot, &actor)?;
    let (visible, hidden) = visible_files_with_hidden(snapshot.files, &actor);

    for file in visible {
        println!("{} {} bytes", file.path, file.bytes);
    }

    if hidden > 0 {
        println!("hidden: {hidden} restricted file(s)");
    }

    Ok(())
}

fn status(repo: &Repo, actor_name: &str, json: bool) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let Some(change_id) = workspace.current_change else {
        if json {
            return status_json::print(repo, &actor, None, None);
        }
        println!("workspace has no current change");
        println!("next: rgit change new <name>");
        return Ok(());
    };

    let change = read_change(repo, &change_id)?;
    if !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    let previous = read_optional_snapshot(repo, change.workspace_base_snapshot_id())?
        .map_or_else(Vec::new, |snapshot| snapshot.files);
    let current = scan_working_tree(repo, false)?;
    let diff = permissioned_diff(previous, current, &actor);
    if json {
        return status_json::print(repo, &actor, Some(&change), Some(&diff));
    }

    println!("actor: {}", actor.name);
    println!("change: {} ({})", change.name, change.id);
    println!(
        "snapshot: {}",
        visible_snapshot_id(repo, change.current_snapshot.as_deref(), &actor)?
    );
    diff.print();

    Ok(())
}

fn diff_workspace(repo: &Repo, actor_name: &str, patch: bool) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;
    let Some(change_id) = workspace.current_change else {
        println!("workspace has no current change");
        println!("next: rgit change new <name>");
        return Ok(());
    };
    let change = read_change(repo, &change_id)?;

    if !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    let previous = read_optional_snapshot(repo, change.workspace_base_snapshot_id())?
        .map_or_else(Vec::new, |snapshot| snapshot.files);
    let current = scan_working_tree(repo, false)?;
    if patch {
        return text_diff::print(repo, &actor, previous, current, true);
    }
    let diff = permissioned_diff(previous, current, &actor);

    println!("actor: {}", actor.name);
    println!("diff: workspace");
    println!(
        "base snapshot: {}",
        visible_snapshot_id(repo, change.workspace_base_snapshot_id(), &actor)?
    );
    diff.print();
    Ok(())
}

fn diff_snapshots(
    repo: &Repo,
    old_snapshot: &str,
    new_snapshot: &str,
    actor_name: &str,
    patch: bool,
) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let old = read_snapshot(repo, old_snapshot).map_err(cli_failure::unavailable_if_missing)?;
    let new = read_snapshot(repo, new_snapshot).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &old.policy) || !can_access(&actor, &new.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    if patch {
        return text_diff::print(repo, &actor, old.files, new.files, false);
    }
    let diff = permissioned_diff(old.files, new.files, &actor);

    println!("actor: {}", actor.name);
    println!("diff: {old_snapshot} -> {new_snapshot}");
    diff.print();
    Ok(())
}

fn diff_line(repo: &Repo, line_name: &str, actor_name: &str, patch: bool) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let line = read_line(repo, line_name).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
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
    if patch {
        return text_diff::print(repo, &actor, parent_files, head.files, false);
    }
    let diff = permissioned_diff(parent_files, head.files, &actor);

    println!("actor: {}", actor.name);
    println!("diff: line {line_name}");
    println!(
        "head snapshot: {}",
        visible_snapshot_id(repo, Some(head_snapshot_id), &actor)?
    );
    println!(
        "parent snapshot: {}",
        visible_snapshot_id(repo, head.parent_snapshot.as_deref(), &actor)?
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
    let mut change = read_change(repo, &change_id)?;
    let line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) || !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    let incoming_snapshot_id = change
        .current_snapshot
        .as_deref()
        .ok_or_else(|| anyhow!("change has no snapshot; run `rgit snapshot` first"))?;
    let incoming = read_snapshot(repo, incoming_snapshot_id)?;

    if change.target_line != line.name {
        println!(
            "change `{}` targets `{}`, not `{}`",
            change.id, change.target_line, line.name
        );
        return Ok(());
    }

    let base_snapshot = ancestry::merge_base(
        repo,
        line.head_snapshot.as_deref(),
        &incoming,
        change.base_snapshot.as_deref(),
    )?;
    change.base_snapshot = base_snapshot.as_ref().map(|snapshot| snapshot.id.clone());
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
        return Err(CliFailure::OperationUnavailable.into());
    }

    let mut plan = plan_merge(base_files, line_files, incoming_files);
    resolution::apply_resolutions(repo, &actor, &line, &change, &incoming, &mut plan)?;
    let generated = text_merge::apply(repo, &mut plan, &base_snapshot, &line_snapshot, &incoming)?;
    if plan.conflicts.is_empty() {
        text_merge::verify(repo, &plan, &generated)?;
    }

    println!("actor: {}", actor.name);
    println!("merge preview: {} -> {}", change.name, line.name);
    println!(
        "base snapshot: {}",
        change.base_snapshot.as_deref().unwrap_or("none")
    );
    println!(
        "line head: {}",
        visible_snapshot_id(repo, line.head_snapshot.as_deref(), &actor)?
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
    let mut conflicts = read_dir_json::<Conflict>(repo, &repo.path(&["conflicts"]))?;
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
    let conflict = read_conflict(repo, conflict_id).map_err(cli_failure::unavailable_if_missing)?;

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
        return Err(CliFailure::OperationUnavailable.into());
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
    let mut lines = read_dir_json::<Line>(repo, &repo.path(&["lines"]))?;
    lines.sort_by(|a, b| a.name.cmp(&b.name));

    for line in lines {
        if !can_access(&actor, &line.policy) {
            continue;
        }

        println!(
            "{} head:{} domains:{}",
            line.name,
            visible_snapshot_id(repo, line.head_snapshot.as_deref(), &actor)?,
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
    let mut change = read_change(repo, change_id)?;
    let mut line = read_line(repo, line_name)?;

    if !can_access(&actor, &line.policy) || !can_access(&actor, &change.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    let incoming_snapshot_id = change
        .current_snapshot
        .as_deref()
        .ok_or_else(|| anyhow!("change has no snapshot; run `rgit snapshot` first"))?;
    let incoming = read_snapshot(repo, incoming_snapshot_id)?;

    if change.target_line != line.name {
        println!(
            "change `{}` targets `{}`, not `{}`",
            change.id, change.target_line, line.name
        );
        return Ok(());
    }

    let base_snapshot = ancestry::merge_base(
        repo,
        line.head_snapshot.as_deref(),
        &incoming,
        change.base_snapshot.as_deref(),
    )?;
    change.base_snapshot = base_snapshot.as_ref().map(|snapshot| snapshot.id.clone());
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
        return Err(CliFailure::OperationUnavailable.into());
    }

    if base_snapshot
        .as_ref()
        .is_some_and(|base| base.id == incoming.id)
    {
        println!("change is already integrated into {}", line.name);
        return Ok(());
    }
    let mut plan = plan_merge(base_files, line_files, incoming_files);
    resolution::apply_resolutions(repo, &actor, &line, &change, &incoming, &mut plan)?;
    let generated = text_merge::apply(repo, &mut plan, &base_snapshot, &line_snapshot, &incoming)?;

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
        return Err(CliFailure::IntegrationConflicted.into());
    }

    text_merge::verify(repo, &plan, &generated)?;
    text_merge::publish(repo, &generated)?;

    let integrated_policy = source_policy;
    let public_integration_message = if integrated_policy.domains == [PUBLIC_DOMAIN] {
        Some(format!("integrated change into `{}`", line.name))
    } else {
        None
    };
    let integrated_snapshot = Snapshot {
        author: identity::configured(repo)?,
        git: None,
        id: format!("snap_{}", new_id_suffix()),
        change_id: change.id.clone(),
        parent_snapshot: line.head_snapshot.clone(),
        merge_parents: if line.head_snapshot.as_deref() == Some(incoming.id.as_str()) {
            Vec::new()
        } else {
            vec![incoming.id.clone()]
        },
        message: format!("merge {} into {}", change.name, line.name),
        manifest_hash: manifest_hash(&plan.merged_files)?,
        files: plan.merged_files,
        policy: integrated_policy.clone(),
        created_at: now()?,
    };
    write_json(
        repo,
        &snapshot_path(repo, &integrated_snapshot.id)?,
        &integrated_snapshot,
    )?;

    line.head_snapshot = Some(integrated_snapshot.id.clone());
    write_json(repo, &line_path(repo, &line.name)?, &line)?;
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
    let line = read_line(repo, line_name).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
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
    let line = read_line(repo, line_name).map_err(cli_failure::unavailable_if_missing)?;

    if !can_access(&actor, &line.policy) {
        return Err(CliFailure::OperationUnavailable.into());
    }

    let mut operations = read_dir_json::<Operation>(repo, &repo.path(&["operations"]))?;
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

fn print_snapshot_summary(repo: &Repo, snapshot: &Snapshot, actor: &Actor) -> Result<()> {
    if !can_access(actor, &snapshot.policy) {
        println!("snapshot: restricted");
        return Ok(());
    }
    let (_, hidden) = visible_files_with_hidden(snapshot.files.clone(), actor);

    println!("snapshot: {}", snapshot.id);
    println!(
        "change: {}",
        visible_change_id(repo, &snapshot.change_id, actor)?
    );
    println!(
        "parent: {}",
        visible_snapshot_id(repo, snapshot.parent_snapshot.as_deref(), actor)?
    );
    println!("domains: {}", snapshot.policy.domains.join(","));
    println!("message: {}", snapshot.message);
    if let Some(author) = identity::snapshot_author(snapshot)? {
        println!("author: {author}");
    }
    println!("hidden files: {hidden}");
    Ok(())
}

fn visible_snapshot_id(repo: &Repo, id: Option<&str>, actor: &Actor) -> Result<String> {
    let Some(id) = id else {
        return Ok("none".into());
    };
    let snapshot = read_snapshot(repo, id)?;
    Ok(if can_access(actor, &snapshot.policy) {
        id
    } else {
        "restricted"
    }
    .into())
}

fn visible_change_id(repo: &Repo, id: &str, actor: &Actor) -> Result<String> {
    let change = read_change(repo, id)?;
    Ok(if can_access(actor, &change.policy) {
        id
    } else {
        "restricted"
    }
    .into())
}

fn workspace_info(repo: &Repo, actor_name: &str) -> Result<()> {
    let actor = read_actor(repo, actor_name)?;
    let workspace = read_workspace(repo)?;

    match workspace.current_change {
        Some(change_id) => {
            let change = read_change(repo, &change_id)?;
            if !can_access(&actor, &change.policy) {
                println!("change is hidden from actor `{}`", actor.name);
                return Ok(());
            }
            println!("current change: {} ({})", change.name, change.id);
            println!("domains: {}", change.policy.domains.join(","));
            println!(
                "current snapshot: {}",
                visible_snapshot_id(repo, change.current_snapshot.as_deref(), &actor)?
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
    let mut operations = read_dir_json::<Operation>(repo, &repo.path(&["operations"]))?;
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
        id: format!("op_{}", new_id_suffix()),
        kind,
        policy,
        private_message,
        public_message,
        created_at: now()?,
    };
    write_json(repo, &operation_path(repo, &operation.id)?, &operation)
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
            (Some(before), Some(after)) if before != after => modified.push(path),
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
            hidden_by_path.insert(file.path.clone(), file);
        }
    }

    DiffInput {
        visible,
        hidden_by_path,
    }
}

fn hidden_changed_paths<T: PartialEq>(
    previous: &BTreeMap<String, T>,
    current: &BTreeMap<String, T>,
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

    let mut plan = MergePlan {
        merged_files,
        conflicts,
    };
    tree_conflicts::group(&mut plan, [&base, &line, &incoming]);
    plan
}

fn same_file(left: Option<&FileEntry>, right: Option<&FileEntry>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right,
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
            write_json(repo, &conflict_path(repo, &refreshed.id)?, &refreshed)?;
            stored.push(refreshed);
            continue;
        }

        let policy = combined_access_policy([
            pending.policy.clone(),
            change.policy.clone(),
            line.policy.clone(),
        ]);
        let conflict = Conflict {
            id: format!("conf_{}", new_id_suffix()),
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
            resolution: None,
            replacement: None,
            created_at: now()?,
        };
        write_json(repo, &conflict_path(repo, &conflict.id)?, &conflict)?;
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
    let conflicts = read_dir_json::<Conflict>(repo, &repo.path(&["conflicts"]))?;

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
    read_json(repo, &repo.path(&["workspace.json"]))
}

fn read_actor(repo: &Repo, name: &str) -> Result<Actor> {
    let value: Actor = read_json(repo, &actor_path(repo, name)?)
        .with_context(|| format!("actor `{name}` not found"))?;
    if value.name != name {
        bail!("stored actor identity does not match requested identity");
    }
    Ok(value)
}

fn read_change(repo: &Repo, id: &str) -> Result<Change> {
    let value: Change = read_json(repo, &change_path(repo, id)?)
        .with_context(|| format!("change `{id}` not found"))?;
    if value.id != id {
        bail!("stored change identity does not match requested identity");
    }
    Ok(value)
}

fn read_snapshot(repo: &Repo, id: &str) -> Result<Snapshot> {
    let value: Snapshot = read_json(repo, &snapshot_path(repo, id)?)
        .with_context(|| format!("snapshot `{id}` not found"))?;
    if value.id != id {
        bail!("stored snapshot identity does not match requested identity");
    }
    Ok(value)
}

fn read_line(repo: &Repo, name: &str) -> Result<Line> {
    let value: Line = read_json(repo, &line_path(repo, name)?)
        .with_context(|| format!("line `{name}` not found"))?;
    if value.name != name {
        bail!("stored line identity does not match requested identity");
    }
    Ok(value)
}

fn read_conflict(repo: &Repo, id: &str) -> Result<Conflict> {
    let value: Conflict = read_json(repo, &conflict_path(repo, id)?)
        .with_context(|| format!("conflict `{id}` not found"))?;
    if value.id != id {
        bail!("stored conflict identity does not match requested identity");
    }
    Ok(value)
}

fn read_path_policies(repo: &Repo) -> Result<Vec<PathPolicy>> {
    read_json(repo, &repo.path(&["path-policies.json"]))
}

fn actor_path(repo: &Repo, name: &str) -> Result<PathBuf> {
    validate_named_key(name)?;
    Ok(repo.path(&["actors", &format!("{}.json", file_name(name))]))
}

fn change_path(repo: &Repo, id: &str) -> Result<PathBuf> {
    validate_object_id(id, "chg_")?;
    Ok(repo.path(&["changes", &format!("{id}.json")]))
}

fn conflict_path(repo: &Repo, id: &str) -> Result<PathBuf> {
    validate_object_id(id, "conf_")?;
    Ok(repo.path(&["conflicts", &format!("{id}.json")]))
}

fn line_path(repo: &Repo, name: &str) -> Result<PathBuf> {
    validate_named_key(name)?;
    Ok(repo.path(&["lines", &format!("{}.json", file_name(name))]))
}

fn snapshot_path(repo: &Repo, id: &str) -> Result<PathBuf> {
    validate_object_id(id, "snap_")?;
    Ok(repo.path(&["snapshots", &format!("{id}.json")]))
}

fn operation_path(repo: &Repo, id: &str) -> Result<PathBuf> {
    validate_object_id(id, "op_")?;
    Ok(repo.path(&["operations", &format!("{id}.json")]))
}

fn scan_working_tree(repo: &Repo, store_blobs: bool) -> Result<Vec<FileEntry>> {
    let path_policies = read_path_policies(repo)?;
    let mut files = Vec::new();

    let workspace = read_workspace(repo)?;
    let baseline = workspace
        .current_change
        .as_deref()
        .map(|id| read_change(repo, id))
        .transpose()?
        .map(|change| read_optional_snapshot(repo, change.workspace_base_snapshot_id()))
        .transpose()?
        .flatten();
    let mut rules = ignore_rules::Rules::new(
        &repo.root,
        baseline
            .as_ref()
            .map(|s| s.files.iter().map(|f| f.path.clone()).collect())
            .unwrap_or_default(),
    );
    #[cfg(not(unix))]
    let inherited_modes: BTreeMap<String, transaction::WorkingFlags> =
        read_optional_snapshot(repo, workspace.mode_snapshot.as_deref())?
            .or(baseline)
            .into_iter()
            .flat_map(|s| s.files)
            .map(|f| (f.path.clone(), f.flags()))
            .collect();
    let mut walker = WalkDir::new(&repo.root).into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.context("failed to read directory entry")?;
        if entry.depth() > 0
            && (!should_scan(entry.path())
                || !rules.includes(entry.path(), entry.file_type().is_dir())?)
        {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
            bail!("unsupported special filesystem entry");
        }

        let path = entry.path();
        let bytes = if entry.file_type().is_symlink() {
            transaction::read_link_bytes(path)?
        } else {
            fs::read(path).with_context(|| format!("failed to read {}", path.display()))?
        };
        let hash = hash_bytes(&bytes);
        let relative_path = repository_relative_path(
            path.strip_prefix(&repo.root)
                .context("failed to compute relative path")?,
        )?;

        if store_blobs {
            let blob_path = repo.path(&["blobs", &hash]);
            if blob_path
                .try_exists()
                .context("failed to inspect stored blob")?
            {
                let stored =
                    verify::read_blob(repo, &hash).context("failed to verify stored blob")?;
                if stored != bytes {
                    bail!("stored blob failed verification; snapshot was not published");
                }
            } else {
                transaction::publish_file(&blob_path, &bytes)?;
            }
        }

        #[cfg(unix)]
        let symlink = entry.file_type().is_symlink();
        #[cfg(not(unix))]
        let symlink = entry.file_type().is_symlink()
            || inherited_modes
                .get(&relative_path)
                .is_some_and(|f| f.symlink);
        if symlink {
            transaction::validate_link(&bytes)?;
        }
        files.push(FileEntry {
            symlink,
            #[cfg(unix)]
            executable: transaction::is_executable(path)?,
            #[cfg(not(unix))]
            executable: !symlink
                && inherited_modes
                    .get(&relative_path)
                    .is_some_and(|f| f.executable),
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
    !name.is_some_and(|name| {
        name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(META_DIR)
    })
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

// Convert actual host path components; never trim names or reinterpret a Unix
// backslash as a separator. The JSON prototype cannot losslessly encode non-UTF-8.
fn repository_relative_path(path: &Path) -> Result<String> {
    let mut segments = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => {
                segments.push(name.to_str().context("non-UTF-8 paths are not supported")?);
            }
            _ => bail!("path must be a nonempty repository-relative path without parent traversal"),
        }
    }
    if segments.is_empty() {
        bail!("path must be a nonempty repository-relative path without parent traversal");
    }
    Ok(segments.join("/"))
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

fn read_dir_json<T: DeserializeOwned>(repo: &Repo, dir: &Path) -> Result<Vec<T>> {
    repo.transaction
        .list(dir)?
        .iter()
        .map(|path| read_json(repo, path))
        .collect()
}

fn read_json<T: DeserializeOwned>(repo: &Repo, path: &Path) -> Result<T> {
    let bytes = repo.transaction.read(path)?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_json<T: Serialize>(repo: &Repo, path: &Path, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value).context("failed to serialize json")?;
    repo.transaction
        .stage(path, format!("{json}\n").into_bytes())
}

fn is_false(value: &bool) -> bool {
    !value
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn operation_kind(kind: &OperationKind) -> &'static str {
    match kind {
        OperationKind::InitRepo => "init_repo",
        OperationKind::SetIdentity => "set_identity",
        OperationKind::CreateLine { .. } => "create_line",
        OperationKind::ResetLine { .. } => "reset_line",
        OperationKind::BindGitIdentity { .. } => "bind_git_identity",
        OperationKind::RetargetChange { .. } => "retarget_change",
        OperationKind::ImportGit { .. } => "import_git",
        OperationKind::SwitchWorkspace { .. } => "switch_workspace",
        OperationKind::RestoreWorkspace { .. } => "restore_workspace",
        OperationKind::SetActor { .. } => "set_actor",
        OperationKind::SetPathPolicy { .. } => "set_path_policy",
        OperationKind::CreateChange { .. } => "create_change",
        OperationKind::CreateSnapshot { .. } => "create_snapshot",
        OperationKind::IntegrateLine { .. } => "integrate_line",
        OperationKind::CreateConflict { .. } => "create_conflict",
        OperationKind::ResolveConflict { .. } => "resolve_conflict",
    }
}

fn conflict_kind(kind: &ConflictKind) -> &'static str {
    match kind {
        ConflictKind::BothModified => "both_modified",
        ConflictKind::DeleteModify => "delete_modify",
        ConflictKind::AddAdd => "add_add",
        ConflictKind::FileDirectory => "file_directory",
    }
}

fn conflict_status(status: &ConflictStatus) -> &'static str {
    match status {
        ConflictStatus::Unresolved => "unresolved",
        ConflictStatus::Resolved => "resolved",
    }
}

fn validate_object_id(id: &str, prefix: &str) -> Result<()> {
    let suffix = id.strip_prefix(prefix).unwrap_or("");
    if !matches!(suffix.len(), 12 | 32)
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid object identifier");
    }
    Ok(())
}

fn validate_named_key(name: &str) -> Result<()> {
    if name.is_empty() || file_name(name).len() > 200 {
        bail!("invalid actor or line name");
    }
    for component in name.split('/') {
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase()
            .replace('¹', "1")
            .replace('²', "2")
            .replace('³', "3");
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if component.is_empty()
            || component.ends_with(['.', ' '])
            || reserved
            || component.chars().any(|character| {
                character.is_control()
                    || matches!(character, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            })
        {
            bail!("invalid actor or line name");
        }
    }
    Ok(())
}

fn file_name(name: &str) -> String {
    name.replace('/', "__")
}

fn new_id_suffix() -> String {
    Uuid::new_v4().simple().to_string()
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

    #[cfg(unix)]
    #[test]
    fn relative_paths_reject_non_utf8_without_lossy_replacement() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let path = PathBuf::from(OsString::from_vec(vec![b'f', 0xff]));
        assert!(repository_relative_path(&path).is_err());
    }

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
            symlink: false,
            executable: false,
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

    fn change_with_snapshots(base: Option<&str>, current: Option<&str>) -> Change {
        Change {
            id: "chg_test".to_string(),
            name: "test".to_string(),
            base_snapshot: base.map(str::to_string),
            target_line: DEFAULT_LINE.to_string(),
            current_snapshot: current.map(str::to_string),
            policy: public_policy(),
            created_at: 0,
        }
    }

    #[test]
    fn workspace_base_uses_change_base_before_first_snapshot() {
        let change = change_with_snapshots(Some("snap_line"), None);

        assert_eq!(change.workspace_base_snapshot_id(), Some("snap_line"));
    }

    #[test]
    fn workspace_base_uses_current_snapshot_after_first_snapshot() {
        let change = change_with_snapshots(Some("snap_line"), Some("snap_current"));

        assert_eq!(change.workspace_base_snapshot_id(), Some("snap_current"));
        assert_eq!(
            change_with_snapshots(None, None).workspace_base_snapshot_id(),
            None
        );
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
    fn structural_conflicts_include_pending_parent_and_all_descendant_policies() {
        let base = vec![file("a", "base", &[PUBLIC_DOMAIN])];
        let line = vec![file("a", "modified", &[PUBLIC_DOMAIN])];
        let incoming = vec![
            file("a/b", "child", &["team/security"]),
            file("a/deep/c", "deep", &[PUBLIC_DOMAIN]),
            file("a-other", "independent", &[PUBLIC_DOMAIN]),
        ];
        let plan = plan_merge(base, line, incoming);
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].path, "a");
        assert_eq!(plan.conflicts[0].kind, ConflictKind::FileDirectory);
        assert!(!can_access_pending_conflict(
            &actor("public", &[PUBLIC_DOMAIN]),
            &plan.conflicts[0]
        ));
        assert_eq!(plan.merged_files.len(), 1);
        assert_eq!(plan.merged_files[0].path, "a-other");
    }

    #[test]
    fn uncontested_structural_changes_and_identical_changes_remain_clean() {
        let base = vec![file("a", "base", &[PUBLIC_DOMAIN])];
        let directory = vec![file("a/b", "child", &[PUBLIC_DOMAIN])];
        for line in [base.clone(), directory.clone()] {
            let plan = plan_merge(base.clone(), line, directory.clone());
            assert!(plan.conflicts.is_empty());
            assert_eq!(plan.merged_files, directory);
        }
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
            resolution: None,
            replacement: None,
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
            resolution: None,
            replacement: None,
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
    fn should_scan_protects_metadata_and_leaves_user_directories_to_ignore_rules() {
        assert!(!should_scan(Path::new(".git")));
        assert!(!should_scan(Path::new(".rgit")));
        assert!(should_scan(Path::new("target")));
        assert!(should_scan(Path::new("node_modules")));
        assert!(!should_scan(Path::new(".GIT")));
        assert!(!should_scan(Path::new(".RGIT")));
        assert!(should_scan(Path::new("src")));
    }
}
