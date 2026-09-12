//! Bounded-memory capture of regular-file blobs before publishing metadata.
use super::*;
use std::io::Read;

pub(super) fn blob(repo: &Repo, reader: impl Read) -> Result<(String, u64)> {
    let directory = repo.path(&["blobs"]);
    let metadata = fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("blob directory must be a regular directory");
    }
    let temporary = directory.join(format!(".rgit-publish-{}", Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        let identity = blob_io::copy_digest(reader, &mut file)?;
        file.sync_all()?;
        let destination = directory.join(&identity.0);
        // A hard link admits the complete, synced bytes without replacing an
        // existing object. The command lock serializes cooperating CLI writers.
        match fs::hard_link(&temporary, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stored =
                    verify::open_blob(repo, &identity.0).context("failed to verify stored blob")?;
                if !equal_contents(stored, fs::File::open(&temporary)?)? {
                    bail!("stored blob failed verification; snapshot was not published");
                }
            }
            Err(error) => return Err(error).context("failed to publish captured blob"),
        }
        #[cfg(unix)]
        fs::File::open(&directory)?.sync_all()?;
        Ok(identity)
    })();
    drop(file);
    // An interruption may leave an ignored publication temporary, never a
    // partial object under its content address. Ordinary errors clean it up.
    let cleanup = fs::remove_file(&temporary);
    let identity = result?;
    cleanup.context("failed to remove blob publication temporary")?;
    Ok(identity)
}

fn equal_contents(mut left: impl Read, mut right: impl Read) -> Result<bool> {
    let mut left_bytes = [0_u8; 65_536];
    let mut right_bytes = [0_u8; 65_536];
    loop {
        let count = match right.read(&mut right_bytes) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            return match left.read_exact(&mut left_bytes[..1]) {
                Ok(()) => Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(true),
                Err(error) => Err(error.into()),
            };
        }
        match left.read_exact(&mut left_bytes[..count]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        if left_bytes[..count] != right_bytes[..count] {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_capture_removes_temporary_without_admitting_a_partial_blob() {
        struct FailedRead(bool);
        impl Read for FailedRead {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                assert!(bytes.len() <= 65_536);
                if self.0 {
                    return Err(std::io::ErrorKind::PermissionDenied.into());
                }
                self.0 = true;
                bytes.fill(42);
                Ok(bytes.len())
            }
        }
        let root = std::env::temp_dir().join(format!("rgit-capture-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        {
            let repo = initialize_repo(root.clone()).unwrap();
            assert!(blob(&repo, FailedRead(false)).is_err());
            assert_eq!(fs::read_dir(repo.path(&["blobs"])).unwrap().count(), 0);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exact_comparison_checks_both_lengths_and_chunk_boundaries() {
        let bytes = vec![42_u8; 131_073];
        assert!(equal_contents(&bytes[..], &bytes[..]).unwrap());
        assert!(!equal_contents(&bytes[..bytes.len() - 1], &bytes[..]).unwrap());
        assert!(!equal_contents(&bytes[..], &bytes[..bytes.len() - 1]).unwrap());
        let mut changed = bytes.clone();
        changed[65_536] = 41;
        assert!(!equal_contents(&bytes[..], &changed[..]).unwrap());
        assert!(equal_contents(&b""[..], &b""[..]).unwrap());
    }
}
