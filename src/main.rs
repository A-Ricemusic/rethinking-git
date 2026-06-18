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
const FORMAT_VERSION: u32 = 1;

#[derive(Parser)]
#[command(name = "rgit")]
#[command(about = "A jj-inspired source control prototype.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize source control in the current directory.
    Init,
    /// Show changed files since the current change's latest snapshot.
    Status,
    /// Capture the current files as a snapshot on the current change.
    Snapshot {
        /// Human note for why this snapshot exists.
        #[arg(short, long, default_value = "manual snapshot")]
        message: String,
    },
    /// Work with logical changes.
    Change {
        #[command(subcommand)]
        command: ChangeCommand,
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
    },
    /// List known changes.
    List,
}

#[derive(Subcommand)]
enum WorkspaceCommand {
    /// Show the current workspace state.
    Info,
}

#[derive(Subcommand)]
enum OpCommand {
    /// Show repository operations.
    Log,
}

#[derive(Serialize, Deserialize)]
struct RepoConfig {
    format_version: u32,
    repo_id: String,
    created_at: u64,
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
    created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct FileEntry {
    path: String,
    hash: String,
    bytes: u64,
}

#[derive(Serialize, Deserialize)]
struct Operation {
    id: String,
    kind: OperationKind,
    message: String,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OperationKind {
    InitRepo,
    CreateChange {
        change_id: String,
    },
    CreateSnapshot {
        change_id: String,
        snapshot_id: String,
    },
}

struct Repo {
    root: PathBuf,
    meta: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => init_repo(),
        Command::Status => {
            let repo = Repo::discover()?;
            status(&repo)
        }
        Command::Snapshot { message } => {
            let repo = Repo::discover()?;
            create_snapshot(&repo, &message)
        }
        Command::Change { command } => {
            let repo = Repo::discover()?;
            match command {
                ChangeCommand::New { name } => create_change(&repo, &name),
                ChangeCommand::List => list_changes(&repo),
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
                OpCommand::Log => op_log(&repo),
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

fn init_repo() -> Result<()> {
    let root = std::env::current_dir().context("failed to read current directory")?;
    let meta = root.join(META_DIR);

    if meta.exists() {
        bail!("repository already exists at {}", meta.display());
    }

    fs::create_dir(&meta).context("failed to create .rgit directory")?;
    for dir in ["blobs", "changes", "operations", "snapshots"] {
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

    write_json(&repo.path(&["repo.json"]), &config)?;
    write_json(&repo.path(&["workspace.json"]), &workspace)?;
    record_operation(
        &repo,
        OperationKind::InitRepo,
        "initialized repository".to_string(),
    )?;

    println!("initialized rgit repository");
    println!("next: rgit change new <name>");
    Ok(())
}

fn create_change(repo: &Repo, name: &str) -> Result<()> {
    let change = Change {
        id: format!("chg_{}", short_id()),
        name: name.to_string(),
        current_snapshot: None,
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
        format!("created change `{}`", change.name),
    )?;

    println!("created change {}", change.id);
    println!("workspace now points at `{}`", change.name);
    Ok(())
}

fn list_changes(repo: &Repo) -> Result<()> {
    let workspace = read_workspace(repo)?;
    let mut changes = read_dir_json::<Change>(&repo.path(&["changes"]))?;
    changes.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    for change in changes {
        let marker = if workspace.current_change.as_deref() == Some(change.id.as_str()) {
            "*"
        } else {
            " "
        };
        println!(
            "{marker} {} {} snapshot:{}",
            change.id,
            change.name,
            change.current_snapshot.as_deref().unwrap_or("none")
        );
    }

    Ok(())
}

fn create_snapshot(repo: &Repo, message: &str) -> Result<()> {
    let workspace = read_workspace(repo)?;
    let change_id = workspace.current_change.as_deref().ok_or_else(|| {
        anyhow!("workspace has no current change; run `rgit change new <name>` first")
    })?;
    let mut change = read_change(repo, change_id)?;
    let files = scan_working_tree(repo, true)?;
    let manifest_hash = manifest_hash(&files)?;
    let snapshot = Snapshot {
        id: format!("snap_{}", short_id()),
        change_id: change.id.clone(),
        parent_snapshot: change.current_snapshot.clone(),
        message: message.to_string(),
        manifest_hash,
        files,
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
        format!("created snapshot for `{}`", change.name),
    )?;

    println!("created snapshot {}", snapshot.id);
    println!("change: {}", change.name);
    Ok(())
}

fn status(repo: &Repo) -> Result<()> {
    let workspace = read_workspace(repo)?;
    let Some(change_id) = workspace.current_change else {
        println!("workspace has no current change");
        println!("next: rgit change new <name>");
        return Ok(());
    };

    let change = read_change(repo, &change_id)?;
    let current = scan_working_tree(repo, false)?;
    let previous = match &change.current_snapshot {
        Some(snapshot_id) => read_snapshot(repo, snapshot_id)?.files,
        None => Vec::new(),
    };
    let diff = diff_files(previous, current);

    println!("change: {} ({})", change.name, change.id);
    println!(
        "snapshot: {}",
        change.current_snapshot.as_deref().unwrap_or("none")
    );
    diff.print();

    Ok(())
}

fn workspace_info(repo: &Repo) -> Result<()> {
    let workspace = read_workspace(repo)?;

    match workspace.current_change {
        Some(change_id) => {
            let change = read_change(repo, &change_id)?;
            println!("current change: {} ({})", change.name, change.id);
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

fn op_log(repo: &Repo) -> Result<()> {
    let mut operations = read_dir_json::<Operation>(&repo.path(&["operations"]))?;
    operations.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    for operation in operations {
        println!(
            "{} {} {}",
            operation.id,
            operation_kind(&operation.kind),
            operation.message
        );
    }

    Ok(())
}

fn record_operation(repo: &Repo, kind: OperationKind, message: String) -> Result<()> {
    let operation = Operation {
        id: format!("op_{}", short_id()),
        kind,
        message,
        created_at: now()?,
    };
    write_json(&operation_path(repo, &operation.id), &operation)
}

fn diff_files(previous: Vec<FileEntry>, current: Vec<FileEntry>) -> FileDiff {
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
    }
}

struct FileDiff {
    added: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

impl FileDiff {
    fn print(&self) {
        print_paths("added", &self.added);
        print_paths("modified", &self.modified);
        print_paths("deleted", &self.deleted);

        if self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty() {
            println!("clean");
        }
    }
}

fn read_workspace(repo: &Repo) -> Result<Workspace> {
    read_json(&repo.path(&["workspace.json"]))
}

fn read_change(repo: &Repo, id: &str) -> Result<Change> {
    read_json(&change_path(repo, id)).with_context(|| format!("change `{id}` not found"))
}

fn read_snapshot(repo: &Repo, id: &str) -> Result<Snapshot> {
    read_json(&snapshot_path(repo, id)).with_context(|| format!("snapshot `{id}` not found"))
}

fn change_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["changes", &format!("{id}.json")])
}

fn snapshot_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["snapshots", &format!("{id}.json")])
}

fn operation_path(repo: &Repo, id: &str) -> PathBuf {
    repo.path(&["operations", &format!("{id}.json")])
}

fn scan_working_tree(repo: &Repo, store_blobs: bool) -> Result<Vec<FileEntry>> {
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
        let relative_path = path
            .strip_prefix(&repo.root)
            .context("failed to compute relative path")?
            .to_string_lossy()
            .replace('\\', "/");

        if store_blobs {
            let blob_path = repo.path(&["blobs", &hash]);
            if !blob_path.exists() {
                fs::write(&blob_path, &bytes)
                    .with_context(|| format!("failed to write {}", blob_path.display()))?;
            }
        }

        files.push(FileEntry {
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
    !matches!(name, Some(".git" | META_DIR | "target"))
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
        OperationKind::CreateChange { .. } => "create_change",
        OperationKind::CreateSnapshot { .. } => "create_snapshot",
    }
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
