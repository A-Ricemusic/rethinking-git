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
