use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

pub const PARTIAL_WINDOW: u64 = 4096;
pub const PARTIAL_THRESHOLD: u64 = 65_536;
const BLOCK_SIZE: usize = 1 << 20;

pub type DigestKey = Vec<u8>;

pub trait DigestHasher: Send + Sync {
    /// Hash the entire content of the file at `path`.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from opening or reading the file.
    fn digest(&self, path: &Path) -> io::Result<DigestKey>;

    fn name(&self) -> &'static str;

    /// Hook for hashers that may be unavailable at runtime (e.g. a library
    /// consumer's `DigestHasher` gated on hardware support or an optional
    /// dependency). The pipeline short-circuits on `false` with
    /// `UnsupportedAlgo` instead of treating every file as unreadable.
    /// The built-in hashers are always available.
    fn available(&self) -> bool {
        true
    }
}

pub struct Xxh3Hasher;

impl DigestHasher for Xxh3Hasher {
    fn digest(&self, path: &Path) -> io::Result<DigestKey> {
        let mut file = File::open(path)?;
        let mut hasher = Xxh3::new();
        let mut buf = vec![0u8; BLOCK_SIZE];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.digest128().to_be_bytes().to_vec())
    }

    fn name(&self) -> &'static str {
        "xxh3"
    }
}

pub struct Sha256Hasher;

impl DigestHasher for Sha256Hasher {
    fn digest(&self, path: &Path) -> io::Result<DigestKey> {
        use sha2::{Digest, Sha256};
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; BLOCK_SIZE];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().to_vec())
    }

    fn name(&self) -> &'static str {
        "sha256"
    }
}

pub enum FullHashStrategy {
    Digest(Box<dyn DigestHasher>),
    Bytewise,
}

impl FullHashStrategy {
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            FullHashStrategy::Digest(d) => d.name(),
            FullHashStrategy::Bytewise => "bytewise",
        }
    }

    #[must_use]
    pub fn xxh3() -> Self {
        FullHashStrategy::Digest(Box::new(Xxh3Hasher))
    }

    #[must_use]
    pub fn sha256() -> Self {
        FullHashStrategy::Digest(Box::new(Sha256Hasher))
    }
}

/// Hash the first and last [`PARTIAL_WINDOW`] bytes of a file. Callers must
/// only pass files of at least [`PARTIAL_THRESHOLD`] bytes, so the two
/// windows never overlap.
///
/// # Errors
///
/// Returns any I/O error from opening, seeking, or reading the file —
/// including a file shrunk below `size` since it was stat'ed.
pub fn partial_xxh3(path: &Path, size: u64) -> io::Result<DigestKey> {
    debug_assert!(size >= PARTIAL_THRESHOLD);
    let mut file = File::open(path)?;
    let mut hasher = Xxh3::new();
    // PARTIAL_WINDOW is 4 KiB; the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    let mut buf = vec![0u8; PARTIAL_WINDOW as usize];

    file.read_exact(&mut buf)?;
    hasher.update(&buf);

    file.seek(SeekFrom::Start(size - PARTIAL_WINDOW))?;
    file.read_exact(&mut buf)?;
    hasher.update(&buf);

    Ok(hasher.digest128().to_be_bytes().to_vec())
}

#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(bytes: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn xxh3_digest_is_deterministic() {
        let f = write_temp(b"hello world");
        let h1 = Xxh3Hasher.digest(f.path()).unwrap();
        let h2 = Xxh3Hasher.digest(f.path()).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn xxh3_distinguishes_content() {
        let a = write_temp(b"hello");
        let b = write_temp(b"hellp");
        assert_ne!(
            Xxh3Hasher.digest(a.path()).unwrap(),
            Xxh3Hasher.digest(b.path()).unwrap()
        );
    }

    #[test]
    fn sha256_digest_is_deterministic_and_well_known() {
        // SHA-256("hello world") = b94d27b9934d3e08a52e52d7da7dabfa...
        let f = write_temp(b"hello world");
        let h = Sha256Hasher.digest(f.path()).unwrap();
        assert_eq!(h.len(), 32);
        assert_eq!(
            hex(&h),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        // Re-run gives same result.
        let h2 = Sha256Hasher.digest(f.path()).unwrap();
        assert_eq!(h, h2);
    }

    #[test]
    fn sha256_distinguishes_content() {
        let a = write_temp(b"hello");
        let b = write_temp(b"hellp");
        assert_ne!(
            Sha256Hasher.digest(a.path()).unwrap(),
            Sha256Hasher.digest(b.path()).unwrap()
        );
    }

    #[test]
    fn partial_xxh3_separates_head_difference() {
        let mut a = vec![0u8; usize::try_from(PARTIAL_THRESHOLD).unwrap()];
        let mut b = a.clone();
        a[0] = 1;
        let fa = write_temp(&a);
        let fb = write_temp(&b);
        let ha = partial_xxh3(fa.path(), a.len() as u64).unwrap();
        let hb = partial_xxh3(fb.path(), b.len() as u64).unwrap();
        assert_ne!(ha, hb);
        // Same files compare equal:
        b[0] = 1;
        let fb2 = write_temp(&b);
        let hb2 = partial_xxh3(fb2.path(), b.len() as u64).unwrap();
        assert_eq!(ha, hb2);
    }

    #[test]
    fn hex_encoding_round_trip() {
        assert_eq!(hex(&[0x00, 0xff, 0xab]), "00ffab");
    }
}
