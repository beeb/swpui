use std::{
    fs,
    io::{BufReader, Read},
    path::Path,
};

use sha2::{Digest as _, Sha256};

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
pub struct FileHash([u8; 32]);

impl From<[u8; 32]> for FileHash {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl FileHash {
    /// Hash the contents of a file.
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let file = fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        Ok(Self::digest(&mut reader))
    }

    /// Hash the bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let hash: [u8; 32] = Sha256::digest(bytes).into();
        Self(hash)
    }

    /// Hash the bytes produced by a reader.
    pub fn digest<R: Read>(content: &mut R) -> Self {
        let mut hasher = Sha256::new();
        let mut buf = [0; 1024];
        while let Ok(size) = content.read(&mut buf) {
            if size == 0 {
                break;
            }
            hasher.update(&buf[0..size]);
        }
        Self(hasher.finalize().into())
    }

    /// Check whether the hash matches the current contents of a file.
    pub fn matches(&self, path: impl AsRef<Path>) -> anyhow::Result<bool> {
        Ok(&Self::new(path)? == self)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn stale_file_detected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "original content").unwrap();
        let hash = FileHash::new(&path).unwrap();

        // modify the file externally
        fs::write(&path, "modified content").unwrap();

        assert!(!hash.matches(&path).unwrap());
    }

    #[test]
    fn fresh_file_not_stale() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "original content").unwrap();
        let hash = FileHash::new(&path).unwrap();

        assert!(hash.matches(&path).unwrap());
    }
}
