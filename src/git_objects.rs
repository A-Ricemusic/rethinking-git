use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CommitMetadata {
    pub object_id: String,
    pub object_format: String,
    pub raw_commit: Vec<u8>,
}

pub(super) struct ParsedCommit {
    pub tree: String,
    pub parents: Vec<String>,
    pub message: Vec<u8>,
    pub timestamp: u64,
}

impl CommitMetadata {
    pub fn parse(&self) -> Result<ParsedCommit> {
        if object_id("commit", &self.raw_commit, &self.object_format)? != self.object_id {
            bail!("Git commit provenance digest mismatch");
        }
        let split = self
            .raw_commit
            .windows(2)
            .position(|pair| pair == b"\n\n")
            .context("Git commit has no header terminator")?;
        let mut tree = None;
        let mut parents = Vec::new();
        let mut timestamp = None;
        for line in self.raw_commit[..split].split(|byte| *byte == b'\n') {
            if let Some(id) = line.strip_prefix(b"tree ") {
                if tree.is_some() {
                    bail!("duplicate Git tree header");
                }
                tree = Some(parse_id(id, &self.object_format)?);
            }
            if let Some(id) = line.strip_prefix(b"parent ") {
                parents.push(parse_id(id, &self.object_format)?);
            }
            if let Some(committer) = line.strip_prefix(b"committer ") {
                let stamp = committer
                    .rsplit(|b| *b == b' ')
                    .nth(1)
                    .context("Git committer timestamp missing")?;
                let stamp: i64 = std::str::from_utf8(stamp)?.parse()?;
                timestamp = Some(stamp.max(0) as u64);
            }
        }
        Ok(ParsedCommit {
            tree: tree.context("Git tree header missing")?,
            parents,
            message: self.raw_commit[split + 2..].to_vec(),
            timestamp: timestamp.context("Git committer missing")?,
        })
    }
}

pub(super) fn parse_id(bytes: &[u8], format: &str) -> Result<String> {
    let length = match format {
        "sha1" => 40,
        "sha256" => 64,
        _ => bail!("unsupported Git object format"),
    };
    if bytes.len() != length
        || !bytes
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        bail!("invalid Git object identifier");
    }
    Ok(std::str::from_utf8(bytes)?.to_string())
}

pub(super) fn object_id(kind: &str, bytes: &[u8], format: &str) -> Result<String> {
    let header = format!("{kind} {}\0", bytes.len());
    match format {
        "sha1" => {
            let mut hash = sha1::Sha1::new();
            hash.update(header.as_bytes());
            hash.update(bytes);
            Ok(hex::encode(hash.finalize()))
        }
        "sha256" => {
            let mut hash = Sha256::new();
            hash.update(header.as_bytes());
            hash.update(bytes);
            Ok(hex::encode(hash.finalize()))
        }
        _ => bail!("unsupported Git object format"),
    }
}

pub(super) struct Object {
    pub id: String,
    pub kind: &'static str,
    pub bytes: Vec<u8>,
}
enum Entry {
    File { id: String, executable: bool },
    Directory(BTreeMap<String, Entry>),
}

fn insert(
    directory: &mut BTreeMap<String, Entry>,
    parts: &[&str],
    id: String,
    executable: bool,
) -> Result<()> {
    if parts.len() == 1 {
        if directory
            .insert(parts[0].to_string(), Entry::File { id, executable })
            .is_some()
        {
            bail!("duplicate Git tree path");
        }
    } else {
        let child = directory
            .entry(parts[0].to_string())
            .or_insert_with(|| Entry::Directory(BTreeMap::new()));
        let Entry::Directory(child) = child else {
            bail!("Git tree path collision");
        };
        insert(child, &parts[1..], id, executable)?;
    }
    Ok(())
}

fn encode_tree(
    directory: BTreeMap<String, Entry>,
    format: &str,
    objects: &mut Vec<Object>,
) -> Result<String> {
    let mut entries = Vec::new();
    for (name, entry) in directory {
        let (mode, id, directory) = match entry {
            Entry::File { id, executable } => {
                (if executable { "100755" } else { "100644" }, id, false)
            }
            Entry::Directory(child) => ("40000", encode_tree(child, format, objects)?, true),
        };
        let mut sort_key = name.as_bytes().to_vec();
        if directory {
            sort_key.push(b'/');
        }
        entries.push((sort_key, name, mode, id));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut bytes = Vec::new();
    for (_, name, mode, id) in entries {
        bytes.extend_from_slice(mode.as_bytes());
        bytes.push(b' ');
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        bytes.extend(hex::decode(id)?);
    }
    let id = object_id("tree", &bytes, format)?;
    objects.push(Object {
        id: id.clone(),
        kind: "tree",
        bytes,
    });
    Ok(id)
}

pub(super) fn tree(
    repo: &Repo,
    files: &[FileEntry],
    format: &str,
) -> Result<(String, Vec<Object>)> {
    let mut directory = BTreeMap::new();
    let mut objects = Vec::new();
    for file in files {
        transaction::validate_working_key(&file.path)?;
        let parts: Vec<_> = file.path.split('/').collect();
        if parts.len() > 256 {
            bail!("Git path nesting exceeds supported depth");
        }
        let path = repo.path(&["blobs", &file.hash]);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("unsafe blob while building Git tree");
        }
        let bytes = fs::read(path)?;
        if hash_bytes(&bytes) != file.hash || bytes.len() as u64 != file.bytes {
            bail!("blob changed while building Git tree");
        }
        let id = object_id("blob", &bytes, format)?;
        insert(&mut directory, &parts, id.clone(), file.executable)?;
        objects.push(Object {
            id,
            kind: "blob",
            bytes,
        });
    }
    let tree = encode_tree(directory, format, &mut objects)?;
    Ok((tree, objects))
}
