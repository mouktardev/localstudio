use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Files up to this size are hashed in full; larger files use a documented
/// head+tail+length marker so imports stay fast.
const HASH_FULL_CAP: u64 = 64 * 1024 * 1024;
const HASH_PARTIAL_CHUNK: u64 = 8 * 1024 * 1024;

/// Content hash used for duplicate detection. `partial:`-prefixed values are
/// not whole-file hashes and are only ever compared with other partial hashes.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut hasher = Sha256::new();

    if len <= HASH_FULL_CAP {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    } else {
        let mut head = vec![0u8; HASH_PARTIAL_CHUNK as usize];
        file.read_exact(&mut head)?;
        hasher.update(&head);

        file.seek(SeekFrom::End(-(HASH_PARTIAL_CHUNK as i64)))?;
        let mut tail = vec![0u8; HASH_PARTIAL_CHUNK as usize];
        file.read_exact(&mut tail)?;
        hasher.update(&tail);
        hasher.update(len.to_le_bytes());

        Ok(format!("partial:{:x}", hasher.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_consistently() {
        let dir = tempfile::tempdir().expect("tempdir");
        let small = dir.path().join("small.bin");
        std::fs::write(&small, b"hello").expect("write");
        let hash = hash_file(&small).expect("hash");
        assert_eq!(hash.len(), 64, "full sha256 hex");
        assert!(!hash.starts_with("partial:"));

        let other = dir.path().join("other.bin");
        std::fs::write(&other, b"hello").expect("write");
        assert_eq!(hash_file(&other).unwrap(), hash);
        std::fs::write(&other, b"world").expect("write");
        assert_ne!(hash_file(&other).unwrap(), hash);
    }
}
