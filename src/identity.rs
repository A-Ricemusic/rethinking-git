use super::*;

pub(super) fn configured(repo: &Repo) -> Result<Option<String>> {
    let config: RepoConfig = read_json(repo, &repo.path(&["repo.json"]))?;
    if let Some(author) = &config.author {
        git_bridge::validate_author(author)?;
    }
    Ok(config.author)
}

pub(super) fn set(repo: &Repo, name: &str, email: &str) -> Result<()> {
    if name.trim().is_empty() || email.chars().any(char::is_whitespace) {
        bail!("identity requires a name and an email without whitespace");
    }
    let author = format!("{} <{}>", name.trim(), email);
    git_bridge::validate_author(&author)?;
    let mut config: RepoConfig = read_json(repo, &repo.path(&["repo.json"]))?;
    config.author = Some(author.clone());
    write_json(repo, &repo.path(&["repo.json"]), &config)?;
    record_operation(
        repo,
        OperationKind::SetIdentity,
        admin_policy(),
        format!("configured author `{author}` for future snapshots"),
        None,
    )?;
    println!("configured author: {author}");
    Ok(())
}

pub(super) fn show(repo: &Repo) -> Result<()> {
    match configured(repo)? {
        Some(author) => println!("author: {author}"),
        None => println!("no author configured; use identity set NAME EMAIL"),
    }
    Ok(())
}

pub(super) fn snapshot_author(snapshot: &Snapshot) -> Result<Option<String>> {
    if let Some(author) = &snapshot.author {
        return Ok(Some(author.clone()));
    }
    Ok(snapshot
        .git
        .as_ref()
        .map(|metadata| metadata.parse())
        .transpose()?
        .and_then(|parsed| parsed.author_identity)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}
