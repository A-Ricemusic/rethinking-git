use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use uuid::Uuid;

struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let repo = Self(std::env::temp_dir().join(format!("rgit-clone-{}", Uuid::new_v4())));
        fs::create_dir(&repo.0).unwrap();
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

fn destination() -> PathBuf {
    std::env::temp_dir().join(format!("rgit-clone-destination-{}", Uuid::new_v4()))
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

fn commit(repo: &Repo) {
    git(repo, &["add", "."]);
    git(
        repo,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "commit",
            "-m",
            "source",
        ],
    );
}

#[test]
fn clone_works_outside_a_native_repository_and_materializes_saved_files() {
    let caller = Repo::new();
    let remote = Repo::new();
    git(&remote, &["init", "-b", "main"]);
    fs::write(remote.0.join("source.txt"), "source bytes").unwrap();
    commit(&remote);
    let target = Repo(destination());
    caller.ok(&[
        "git",
        "clone",
        remote.0.to_str().unwrap(),
        target.0.to_str().unwrap(),
        "--domain",
        "public",
    ]);
    assert_eq!(
        fs::read(target.0.join("source.txt")).unwrap(),
        b"source bytes"
    );
    assert!(!target.0.join(".rgit/clone-request.json").exists());
    target.ok(&["repo", "verify", "--as", "admin"]);
    assert!(target.ok(&["status"]).contains("clean"));
    assert!(!caller
        .run(&[
            "git",
            "clone",
            remote.0.to_str().unwrap(),
            target.0.to_str().unwrap()
        ])
        .status
        .success());
    assert_eq!(
        fs::read(target.0.join("source.txt")).unwrap(),
        b"source bytes"
    );
}

#[test]
fn resume_requires_matching_request_and_preserves_files_created_after_failure() {
    let caller = Repo::new();
    let remote = Repo::new();
    git(&remote, &["init", "-b", "main"]);
    let target = Repo(destination());
    let args = [
        "git",
        "clone",
        remote.0.to_str().unwrap(),
        target.0.to_str().unwrap(),
        "--domain",
        "public",
    ];
    assert!(!caller.run(&args).status.success());
    assert!(target.0.join(".rgit/clone-request.json").exists());
    fs::write(remote.0.join("source.txt"), "remote bytes").unwrap();
    commit(&remote);
    assert!(!caller
        .run(&[
            "git",
            "clone",
            remote.0.to_str().unwrap(),
            target.0.to_str().unwrap(),
            "--resume",
            "--branch",
            "different",
            "--domain",
            "public"
        ])
        .status
        .success());
    fs::write(target.0.join("source.txt"), "user's later file").unwrap();
    let mut resume = args.to_vec();
    resume.push("--resume");
    assert!(!caller.run(&resume).status.success());
    assert_eq!(
        fs::read(target.0.join("source.txt")).unwrap(),
        b"user's later file"
    );
    assert!(target.0.join(".rgit/clone-request.json").exists());
    // The user intentionally moves their file aside before retrying.
    fs::rename(target.0.join("source.txt"), target.0.join("preserved.txt")).unwrap();
    caller.ok(&resume);
    assert_eq!(
        fs::read(target.0.join("source.txt")).unwrap(),
        b"remote bytes"
    );
    assert_eq!(
        fs::read(target.0.join("preserved.txt")).unwrap(),
        b"user's later file"
    );
    assert!(!target.0.join(".rgit/clone-request.json").exists());
    target.ok(&["repo", "verify", "--as", "admin"]);
}
