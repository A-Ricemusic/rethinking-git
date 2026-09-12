use super::*;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

pub(super) struct Rules {
    root: PathBuf,
    tracked: BTreeSet<String>,
    matchers: BTreeMap<PathBuf, Gitignore>,
    ignored_directories: BTreeSet<PathBuf>,
}

impl Rules {
    pub fn new(root: &Path, tracked: BTreeSet<String>) -> Self {
        Self {
            root: root.to_path_buf(),
            tracked,
            matchers: BTreeMap::new(),
            ignored_directories: BTreeSet::new(),
        }
    }

    pub fn includes(&mut self, path: &Path, directory: bool) -> Result<bool> {
        let relative = repository_relative_path(path.strip_prefix(&self.root)?)?;
        let parent = path.parent().context("working path has no parent")?;
        let inherited = self.ignored_directories.contains(parent);
        let mut ancestors: Vec<_> = parent
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&self.root))
            .collect();
        ancestors.reverse();
        let mut ignored = false;
        for ancestor in ancestors {
            if !self.matchers.contains_key(ancestor) {
                let mut builder = GitignoreBuilder::new(ancestor);
                let ignore_path = ancestor.join(".gitignore");
                match fs::symlink_metadata(&ignore_path) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                        bail!(".gitignore must be a regular file")
                    }
                    Ok(_) => {
                        if let Some(error) = builder.add(&ignore_path) {
                            return Err(error).context("failed to load .gitignore rules");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("failed to inspect .gitignore"),
                }
                self.matchers.insert(
                    ancestor.to_path_buf(),
                    builder.build().context("invalid .gitignore rules")?,
                );
            }
            let matched = self.matchers[ancestor].matched(path, directory);
            if !matched.is_none() {
                ignored = matched.is_ignore();
            }
        }
        ignored |= inherited;
        if directory && ignored {
            self.ignored_directories.insert(path.to_path_buf());
        }
        if !ignored {
            return Ok(true);
        }
        if directory {
            let prefix = format!("{relative}/");
            Ok(self
                .tracked
                .range(prefix.clone()..)
                .next()
                .is_some_and(|path| path.starts_with(&prefix)))
        } else {
            Ok(self.tracked.contains(&relative))
        }
    }
}
