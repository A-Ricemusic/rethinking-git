//! Streaming content identities for inspection paths that do not need file bytes.
use super::*;
use std::io::{ErrorKind, Read, Write};

pub(super) fn digest(reader: impl Read) -> Result<(String, u64)> {
    digest_with(reader, |_| {})
}

pub(super) fn digest_with(
    reader: impl Read,
    mut visit: impl FnMut(&[u8]),
) -> Result<(String, u64)> {
    digest_fallible(reader, |bytes| {
        visit(bytes);
        Ok(())
    })
}

pub(super) fn copy_digest(reader: impl Read, mut writer: impl Write) -> Result<(String, u64)> {
    digest_fallible(reader, |bytes| {
        writer
            .write_all(bytes)
            .context("failed to spool blob contents")
    })
}

fn digest_fallible(
    mut reader: impl Read,
    mut visit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<(String, u64)> {
    let mut hash = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).context("failed to hash file contents"),
        };
        length = length
            .checked_add(count as u64)
            .context("file length overflow")?;
        hash.update(&buffer[..count]);
        visit(&buffer[..count])?;
    }
    Ok((hex::encode(hash.finalize()), length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    struct ShortReads {
        remaining: Vec<u8>,
        interrupted: bool,
        fail: bool,
    }
    impl Read for ShortReads {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            if self.remaining.is_empty() {
                return if self.fail {
                    Err(io::ErrorKind::PermissionDenied.into())
                } else {
                    Ok(0)
                };
            }
            let count = output.len().min(7).min(self.remaining.len());
            output[..count].copy_from_slice(&self.remaining[..count]);
            self.remaining.drain(..count);
            Ok(count)
        }
    }

    #[test]
    fn empty_short_and_interrupted_reads_match_content_identity() {
        for bytes in [Vec::new(), b"binary\0contents\xffwith short reads".to_vec()] {
            let expected = (hash_bytes(&bytes), bytes.len() as u64);
            assert_eq!(
                digest(ShortReads {
                    remaining: bytes,
                    interrupted: false,
                    fail: false
                })
                .unwrap(),
                expected
            );
        }
    }

    #[test]
    fn read_failure_never_returns_a_partial_content_identity() {
        assert!(digest(ShortReads {
            remaining: b"prefix".to_vec(),
            interrupted: false,
            fail: true
        })
        .is_err());
    }

    #[test]
    fn copying_handles_short_reads_and_reports_write_failures() {
        let bytes = b"binary\0contents\xff".to_vec();
        let mut written = Vec::new();
        assert_eq!(
            copy_digest(
                ShortReads {
                    remaining: bytes.clone(),
                    interrupted: false,
                    fail: false
                },
                &mut written
            )
            .unwrap(),
            (hash_bytes(&bytes), bytes.len() as u64)
        );
        assert_eq!(written, bytes);
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::WriteZero.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(copy_digest(&bytes[..], FailingWriter).is_err());
    }
}
