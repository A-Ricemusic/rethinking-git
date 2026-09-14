use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-remote-{}", Uuid::new_v4())));
        fs::create_dir(&repo.0).unwrap();
        repo.ok(&["init"]);
        repo
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_rgit"))
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
    fn ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const AUTHOR: &str = "Fixture Author <fixture@example.test>";
fn destination() -> PathBuf {
    std::env::temp_dir().join(format!("rgit-remote-destination-{}", Uuid::new_v4()))
}
fn git(repo: &Repo, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .current_dir(&repo.0)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

impl Repo {
    fn head(&self, line: &str) -> String {
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(
                self.0
                    .join(format!(".rgit/lines/{}.json", line.replace('/', "__"))),
            )
            .unwrap(),
        )
        .unwrap();
        value["head_snapshot"].as_str().unwrap().to_string()
    }
    fn head_change(&self, line: &str) -> String {
        let value: serde_json::Value = serde_json::from_slice(
            &fs::read(
                self.0
                    .join(format!(".rgit/snapshots/{}.json", self.head(line))),
            )
            .unwrap(),
        )
        .unwrap();
        value["change_id"].as_str().unwrap().to_string()
    }
    fn fetch(&self, remote: &Repo, into: &str) {
        self.ok(&[
            "git",
            "fetch",
            remote.0.to_str().unwrap(),
            "--into",
            into,
            "--as",
            "admin",
            "--domain",
            "public",
        ]);
    }
    fn push(&self, remote: &Repo) -> Output {
        self.run(&[
            "git",
            "push",
            remote.0.to_str().unwrap(),
            "--as",
            "admin",
            "--author",
            AUTHOR,
        ])
    }
}

#[test]
fn two_clients_fetch_merge_and_push_without_overwriting_concurrent_work() {
    let seed = Repo::new();
    seed.ok(&["change", "new", "seed"]);
    fs::write(seed.0.join("base.txt"), "base").unwrap();
    seed.ok(&["snapshot"]);
    seed.ok(&["line", "integrate"]);
    let remote = Repo(destination());
    seed.ok(&[
        "git",
        "export",
        remote.0.to_str().unwrap(),
        "--as",
        "admin",
        "--author",
        AUTHOR,
    ]);
    let alice = Repo::new();
    let bob = Repo::new();
    for client in [&alice, &bob] {
        client.fetch(&remote, "main");
        client.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);
    }
    fs::write(alice.0.join("alice.txt"), "Alice's saved work").unwrap();
    alice.ok(&["snapshot"]);
    alice.ok(&["line", "integrate", "--as", "admin"]);
    fs::write(bob.0.join("bob.txt"), "Bob's saved work").unwrap();
    bob.ok(&["snapshot"]);
    bob.ok(&["line", "integrate", "--as", "admin"]);
    let pushed = alice.push(&remote);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    let head = git(&remote, &["rev-parse", "HEAD"]);
    assert!(!bob.push(&remote).status.success());
    assert_eq!(git(&remote, &["rev-parse", "HEAD"]), head);
    bob.fetch(&remote, "remote/main");
    let fetched = bob.head_change("remote/main");
    bob.ok(&["workspace", "switch", &fetched, "--as", "admin"]);
    bob.ok(&["change", "retarget", "main", "--as", "admin"]);
    bob.ok(&["line", "integrate", "--as", "admin"]);
    let pushed = bob.push(&remote);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    assert_eq!(
        git(&remote, &["show", "HEAD:alice.txt"]),
        b"Alice's saved work"
    );
    assert_eq!(git(&remote, &["show", "HEAD:bob.txt"]), b"Bob's saved work");
    let before_fetch = fs::read_dir(bob.0.join(".rgit/snapshots")).unwrap().count();
    bob.fetch(&remote, "remote/main");
    assert_eq!(
        fs::read_dir(bob.0.join(".rgit/snapshots")).unwrap().count(),
        before_fetch
    );
    bob.ok(&["repo", "verify", "--as", "admin"]);
    let saved_tracking = bob.head("remote/main");
    let old = String::from_utf8(head).unwrap();
    git(&remote, &["update-ref", "refs/heads/main", old.trim()]);
    assert!(!bob
        .run(&[
            "git",
            "fetch",
            remote.0.to_str().unwrap(),
            "--into",
            "remote/main",
            "--as",
            "admin"
        ])
        .status
        .success());
    assert_eq!(bob.head("remote/main"), saved_tracking);
}

#[test]
fn remote_helpers_plaintext_http_and_embedded_credentials_are_refused() {
    let repo = Repo::new();
    for remote in [
        "ext::sh -c echo",
        "http://example.test/repo.git",
        "https://token@example.test/repo.git",
        "ssh://user:password@example.test/repo.git",
    ] {
        assert!(!repo
            .run(&[
                "git",
                "fetch",
                remote,
                "--into",
                "remote/main",
                "--as",
                "admin"
            ])
            .status
            .success());
        assert!(!repo.0.join(".rgit/lines/remote__main.json").exists());
    }
}

#[test]
fn fetch_reconciles_remote_commits_after_lost_local_identity_publication() {
    let source = Repo::new();
    source.ok(&["change", "new", "source"]);
    fs::write(source.0.join("file.txt"), "saved").unwrap();
    source.ok(&["snapshot"]);
    source.ok(&["line", "integrate"]);
    let records: Vec<_> = fs::read_dir(source.0.join(".rgit/snapshots"))
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let operations: std::collections::BTreeSet<_> = fs::read_dir(source.0.join(".rgit/operations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let remote = Repo(destination());
    source.ok(&[
        "git",
        "export",
        remote.0.to_str().unwrap(),
        "--as",
        "admin",
        "--author",
        AUTHOR,
    ]);
    // Model a remote acceptance whose local provenance transaction never committed.
    for (path, bytes) in &records {
        fs::write(path, bytes).unwrap();
    }
    for entry in fs::read_dir(source.0.join(".rgit/operations")).unwrap() {
        let path = entry.unwrap().path();
        if !operations.contains(&path) {
            fs::remove_file(path).unwrap();
        }
    }
    source.fetch(&remote, "remote/main");
    assert_eq!(
        fs::read_dir(source.0.join(".rgit/snapshots"))
            .unwrap()
            .count(),
        records.len()
    );
    assert_eq!(source.head("main"), source.head("remote/main"));
    source.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn relative_local_remotes_resolve_from_the_calling_directory() {
    let source = Repo::new();
    source.ok(&["change", "new", "source"]);
    fs::write(source.0.join("file.txt"), "saved").unwrap();
    source.ok(&["snapshot"]);
    source.ok(&["line", "integrate"]);
    let remote = Repo(destination());
    source.ok(&[
        "git",
        "export",
        remote.0.to_str().unwrap(),
        "--as",
        "admin",
        "--author",
        AUTHOR,
    ]);
    let client = Repo::new();
    let relative = format!("../{}", remote.0.file_name().unwrap().to_str().unwrap());
    client.ok(&[
        "git", "fetch", &relative, "--into", "main", "--as", "admin", "--domain", "public",
    ]);
    client.ok(&["git", "push", &relative, "--as", "admin"]);
    client.ok(&["repo", "verify", "--as", "admin"]);
}

#[test]
fn git_url_rewrites_cannot_enable_plaintext_transport() {
    let repo = Repo::new();
    let config = repo.0.join("transport.config");
    fs::write(&config, "[url \"http://127.0.0.1:1/\"]\n    insteadOf = https://fixture.invalid/\n[protocol \"http\"]\n    allow = always\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&repo.0)
        .env("GIT_CONFIG_GLOBAL", &config)
        .args([
            "git",
            "fetch",
            "https://fixture.invalid/repo",
            "--into",
            "remote/main",
            "--as",
            "admin",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("transport 'http' not allowed"));
    assert!(!repo.0.join(".rgit/lines/remote__main.json").exists());
}

fn clone_native(remote: &Repo, branch: &str) -> Repo {
    let client = Repo(destination());
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .args([
            "git",
            "clone",
            remote.0.to_str().unwrap(),
            client.0.to_str().unwrap(),
            "--branch",
            branch,
            "--domain",
            "public",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    client.ok(&["identity", "set", "Demo Developer", "demo@example.test"]);
    client
}

fn seed_remote() -> (Repo, Repo) {
    let seed = Repo::new();
    seed.ok(&["identity", "set", "Demo Developer", "demo@example.test"]);
    seed.ok(&["workspace", "start", "seed"]);
    fs::write(seed.0.join("base.txt"), "base").unwrap();
    seed.ok(&["snapshot", "--message", "Initial files"]);
    seed.ok(&["line", "integrate", "--as", "admin"]);
    let remote = Repo(destination());
    seed.ok(&["git", "export", remote.0.to_str().unwrap(), "--as", "admin"]);
    (seed, remote)
}

fn preview(client: &Repo) -> serde_json::Value {
    let text = client.ok(&["--output", "json", "push", "--dry-run", "--as", "admin"]);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["records"].as_array().unwrap().len(), 1);
    assert_eq!(value["records"][0]["kind"], "git_push_preview");
    value["records"][0]["data"].clone()
}

// Snapshot all compatibility records, excluding the command lock/journal whose
// housekeeping may change even for read commands.
fn saved_records(client: &Repo) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(client.0.join(".rgit"))
        .into_iter()
        .map(Result::unwrap)
        .filter(|e| {
            e.file_type().is_file() && e.path().extension().is_some_and(|ext| ext == "json")
        })
        .map(|e| {
            (
                e.path().strip_prefix(&client.0).unwrap().to_path_buf(),
                fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn remembered_upstream_preview_and_git_peer_round_trip() {
    let (_seed, remote) = seed_remote();
    git(&remote, &["branch", "release", "main"]);
    let alice = clone_native(&remote, "release");
    let config: serde_json::Value =
        serde_json::from_slice(&fs::read(alice.0.join(".rgit/repo.json")).unwrap()).unwrap();
    assert_eq!(config["git"]["upstreams"]["main"]["branch"], "release");
    assert_eq!(preview(&alice)["state"], "up_to_date");
    alice.ok(&["workspace", "start", "feature"]);
    fs::write(alice.0.join("feature.txt"), "native feature").unwrap();
    alice.ok(&["snapshot", "--message", "Implement native feature"]);
    // A saved change is not published until integrated into the line.
    assert_eq!(preview(&alice)["state"], "up_to_date");
    alice.ok(&["line", "integrate", "--as", "admin"]);
    fs::write(alice.0.join("feature.txt"), "unsaved edit").unwrap();
    let before = saved_records(&alice);
    let remote_before = git(&remote, &["rev-parse", "release"]);
    let report = preview(&alice);
    assert_eq!(report["state"], "fast_forward");
    assert!(report["ahead"].as_u64().unwrap() > 0);
    assert_eq!(report["branch"], "release");
    assert_eq!(before, saved_records(&alice));
    assert_eq!(remote_before, git(&remote, &["rev-parse", "release"]));
    assert_eq!(
        fs::read(alice.0.join("feature.txt")).unwrap(),
        b"unsaved edit"
    );
    assert_eq!(report, preview(&alice));
    alice.ok(&["push", "--as", "admin"]);
    assert_eq!(
        git(&remote, &["show", "release:feature.txt"]),
        b"native feature"
    );
    assert_eq!(
        String::from_utf8(git(&remote, &["rev-parse", "release"]))
            .unwrap()
            .trim(),
        report["local_commit"].as_str().unwrap()
    );
    assert_eq!(preview(&alice)["state"], "up_to_date");
    alice.ok(&["workspace", "restore", "--discard-changes", "--as", "admin"]);

    // An ordinary Git user contributes on the same upstream branch.
    let peer = Repo(destination());
    let output = Command::new("git")
        .args([
            "clone",
            "--branch",
            "release",
            remote.0.to_str().unwrap(),
            peer.0.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    git(&peer, &["config", "user.name", "Git Peer"]);
    git(&peer, &["config", "user.email", "peer@example.test"]);
    fs::write(peer.0.join("peer.txt"), "Git contribution").unwrap();
    git(&peer, &["add", "peer.txt"]);
    git(&peer, &["commit", "-m", "Contribute through Git"]);
    git(&peer, &["push", "origin", "release"]);
    let before = saved_records(&alice);
    assert_eq!(preview(&alice)["state"], "behind");
    assert_eq!(before, saved_records(&alice));
    alice.ok(&["pull", "--as", "admin"]);
    assert_eq!(
        fs::read(alice.0.join("peer.txt")).unwrap(),
        b"Git contribution"
    );
    assert_eq!(preview(&alice)["state"], "up_to_date");
    git(&remote, &["fsck", "--strict"]);
}

#[test]
fn preview_reports_divergence_and_new_branches_without_binding_snapshots() {
    let (_seed, remote) = seed_remote();
    let alice = clone_native(&remote, "main");
    let bob = clone_native(&remote, "main");
    for (client, name) in [(&alice, "alice"), (&bob, "bob")] {
        client.ok(&["workspace", "start", name]);
        fs::write(client.0.join(name), name).unwrap();
        client.ok(&["snapshot"]);
        client.ok(&["line", "integrate", "--as", "admin"]);
    }
    alice.ok(&["git", "push", "--as", "admin"]);
    let before = saved_records(&bob);
    let report = preview(&bob);
    assert_eq!(report["state"], "diverged");
    assert_eq!(report["fast_forward_allowed"], false);
    assert!(report["behind"].as_u64().unwrap() > 0);
    assert_eq!(before, saved_records(&bob));
    assert!(!bob.run(&["push", "--as", "admin"]).status.success());
    assert_eq!(before, saved_records(&bob));
    bob.ok(&["upstream", "set", "origin", "bob-feature", "--as", "admin"]);
    let before = saved_records(&bob);
    assert_eq!(preview(&bob)["state"], "new_branch");
    assert_eq!(before, saved_records(&bob));
    bob.ok(&["push", "--as", "admin"]);
    assert_eq!(git(&remote, &["show", "bob-feature:bob"]), b"bob");
    assert_eq!(preview(&bob)["state"], "up_to_date");
}

#[test]
fn remote_configuration_is_guarded_shared_and_preserved_by_backup() {
    let (_seed, remote) = seed_remote();
    let client = clone_native(&remote, "main");
    let before = saved_records(&client);
    for args in [
        vec!["remote", "list"],
        vec![
            "remote",
            "set",
            "evil",
            "https://token@example.test/repo",
            "--as",
            "admin",
        ],
        vec!["remote", "remove", "origin", "--as", "admin"],
        vec!["upstream", "set", "missing", "main", "--as", "admin"],
        vec!["upstream", "set", "origin", "bad..branch", "--as", "admin"],
    ] {
        assert!(!client.run(&args).status.success(), "{args:?}");
        assert_eq!(before, saved_records(&client));
    }
    let linked = Repo(destination());
    client.ok(&[
        "worktree",
        "add",
        linked.0.to_str().unwrap(),
        "--name",
        "linked",
        "--from",
        "main",
        "--as",
        "admin",
    ]);
    assert!(linked
        .ok(&["upstream", "show", "--as", "admin"])
        .contains("origin/main"));
    assert_eq!(preview(&linked)["state"], "up_to_date");
    let backup = Repo(destination());
    linked.ok(&[
        "repo",
        "backup",
        backup.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    assert!(backup
        .ok(&["upstream", "show", "--as", "admin"])
        .contains("origin/main"));
    backup.ok(&["repo", "verify", "--as", "admin"]);
    client.ok(&["upstream", "unset", "--as", "admin"]);
    assert!(!linked
        .run(&["push", "--dry-run", "--as", "admin"])
        .status
        .success());
    client.ok(&["remote", "remove", "origin", "--as", "admin"]);
    client.ok(&[
        "remote",
        "set",
        "exchange",
        remote.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    client.ok(&["upstream", "set", "exchange", "main", "--as", "admin"]);
    assert_eq!(preview(&linked)["state"], "up_to_date");
}

#[test]
fn saved_relative_remote_and_current_line_selection_survive_cwd_changes() {
    let (_seed, remote) = seed_remote();
    let client = clone_native(&remote, "main");
    let relative = format!("../{}", remote.0.file_name().unwrap().to_str().unwrap());
    client.ok(&["remote", "set", "exchange", &relative, "--as", "admin"]);
    client.ok(&["line", "create", "feature", "--as", "admin"]);
    client.ok(&[
        "workspace",
        "start",
        "feature-work",
        "--target",
        "feature",
        "--as",
        "admin",
    ]);
    client.ok(&[
        "upstream",
        "set",
        "exchange",
        "feature-branch",
        "--as",
        "admin",
    ]);
    let report = preview(&client);
    assert_eq!(report["line"], "feature");
    assert_eq!(report["branch"], "feature-branch");
    assert_eq!(report["state"], "new_branch");
    let nested = client.0.join("subdirectory");
    fs::create_dir(&nested).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rgit"))
        .current_dir(&nested)
        .args(["--output", "json", "push", "--dry-run", "--as", "admin"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["records"][0]["data"], report);
    // A one-shot branch override does not rewrite the upstream.
    let before = saved_records(&client);
    client.ok(&["push", "--branch", "one-shot", "--as", "admin"]);
    assert!(client
        .ok(&["upstream", "show", "--as", "admin"])
        .contains("exchange/feature-branch"));
    assert_eq!(before, saved_records(&client)); // inherited commits were already bound
    git(&remote, &["rev-parse", "refs/heads/one-shot"]);
    // Transport failures and restricted previews do not publish native records.
    client.ok(&[
        "remote",
        "set",
        "broken",
        client.0.to_str().unwrap(),
        "--as",
        "admin",
    ]);
    let before = saved_records(&client);
    for args in [
        vec![
            "--output",
            "json",
            "push",
            "broken",
            "--dry-run",
            "--as",
            "admin",
        ],
        vec!["--output", "json", "push", "--dry-run"],
    ] {
        let output = client.run(&args);
        assert!(!output.status.success());
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["ok"], false);
        assert!(error["records"].as_array().unwrap().is_empty());
        assert_eq!(before, saved_records(&client));
    }
}

#[test]
fn push_guard_rejects_a_line_advanced_by_another_worktree_and_preview_is_bounded() {
    let (_seed, remote) = seed_remote();
    let alice = clone_native(&remote, "main");
    let linked = Repo(destination());
    alice.ok(&[
        "worktree",
        "add",
        linked.0.to_str().unwrap(),
        "--name",
        "second",
        "--from",
        "main",
        "--as",
        "admin",
    ]);
    alice.ok(&["workspace", "start", "first"]);
    for index in 0..3 {
        fs::write(alice.0.join("first.txt"), format!("checkpoint {index}")).unwrap();
        alice.ok(&["snapshot"]);
    }
    alice.ok(&["line", "integrate", "--as", "admin"]);
    let approved = preview(&alice);
    let expected = approved["snapshot_id"].as_str().unwrap();
    for limit in [0, 1] {
        let report: serde_json::Value = serde_json::from_str(&alice.ok(&[
            "--output",
            "json",
            "push",
            "--dry-run",
            "--max-commits",
            &limit.to_string(),
            "--as",
            "admin",
        ]))
        .unwrap();
        let data = &report["records"][0]["data"];
        assert_eq!(data["commits"].as_array().unwrap().len(), limit);
        assert_eq!(data["commits_truncated"], true);
        assert_eq!(data["commits_total"], approved["ahead"]);
        assert_eq!(data["local_commit"], approved["local_commit"]);
    }
    fs::write(linked.0.join("second.txt"), "other agent").unwrap();
    linked.ok(&["snapshot"]);
    linked.ok(&["line", "integrate", "--as", "admin"]);
    let before = saved_records(&alice);
    let remote_head = git(&remote, &["rev-parse", "main"]);
    let refusal = alice.run(&[
        "--output",
        "json",
        "push",
        "--expect-snapshot",
        expected,
        "--as",
        "admin",
    ]);
    assert!(!refusal.status.success());
    let error: serde_json::Value = serde_json::from_slice(&refusal.stdout).unwrap();
    assert_eq!(error["error"]["kind"], "stale_snapshot");
    assert_eq!(error["records"], serde_json::json!([]));
    assert_eq!(before, saved_records(&alice));
    assert_eq!(remote_head, git(&remote, &["rev-parse", "main"]));
    let refreshed = preview(&alice);
    alice.ok(&[
        "push",
        "--expect-snapshot",
        refreshed["snapshot_id"].as_str().unwrap(),
        "--as",
        "admin",
    ]);
    assert_eq!(
        String::from_utf8(git(&remote, &["rev-parse", "main"]))
            .unwrap()
            .trim(),
        refreshed["local_commit"].as_str().unwrap()
    );
    // Retrying the exact approved snapshot is idempotent.
    let before = saved_records(&alice);
    alice.ok(&[
        "push",
        "--expect-snapshot",
        refreshed["snapshot_id"].as_str().unwrap(),
        "--as",
        "admin",
    ]);
    assert_eq!(before, saved_records(&alice));
    assert_eq!(
        alice
            .run(&["push", "--max-commits", "1", "--as", "admin"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        alice
            .run(&[
                "push",
                "--dry-run",
                "--max-commits",
                "1001",
                "--as",
                "admin"
            ])
            .status
            .code(),
        Some(2)
    );
}
