//! Streaming I/O for private, server-owned continuity files. Never truncates the
//! previous file until the replacement is completely written and synced.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

use sha2::{Digest, Sha256};

pub(super) struct HashReader<R> {
    inner: R,
    hash: Sha256,
}

impl<R> HashReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
        }
    }

    pub fn finish(self) -> String {
        format!("{:x}", self.hash.finalize())
    }
}

impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        self.hash.update(&buffer[..count]);
        Ok(count)
    }
}

pub(super) struct HashWriter<W> {
    inner: W,
    hash: Sha256,
}

impl<W: Write> HashWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hash: Sha256::new(),
        }
    }

    pub fn finish(mut self) -> std::io::Result<String> {
        self.inner.flush()?;
        Ok(format!("{:x}", self.hash.finalize()))
    }
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let count = self.inner.write(buffer)?;
        self.hash.update(&buffer[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(super) fn write_private_atomic<T>(
    path: &Path,
    write: impl FnOnce(&mut File) -> Result<T, String>,
) -> Result<T, String> {
    let parent = path
        .parent()
        .ok_or_else(|| super::failure("missing file directory"))?;
    let temporary = parent.join(format!(".continuity-{}.tmp", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(super::io_failure)?;
        let value = write(&mut file)?;
        file.sync_all().map_err(super::io_failure)?;
        drop(file);
        std::fs::rename(&temporary, path).map_err(super::io_failure)?;
        super::sync_parent(path)?;
        Ok(value)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_replacement_preserves_original_and_cleans_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, b"original").unwrap();
        let result: Result<(), String> = write_private_atomic(&path, |file| {
            file.write_all(b"incomplete").unwrap();
            Err("simulated write failure".into())
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn streaming_hash_matches_file_and_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let expected = write_private_atomic(&path, |file| {
            let mut writer = HashWriter::new(std::io::BufWriter::new(file));
            writer
                .write_all(b"{\"records\":[]}")
                .map_err(super::super::io_failure)?;
            writer.finish().map_err(super::super::io_failure)
        })
        .unwrap();
        let mut reader = HashReader::new(File::open(&path).unwrap());
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(reader.finish(), expected);
        assert_eq!(expected, format!("{:x}", Sha256::digest(&bytes)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
