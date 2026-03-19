//! SHA-256 digest computation for content-addressable storage.
//!
//! All digests in Torvyn packaging use SHA-256, formatted as
//! `sha256:{hex_lowercase}`. This matches the OCI content-addressable
//! storage convention.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// The result of a SHA-256 digest computation.
///
/// # Invariants
/// - The `hex` field is always 64 lowercase hex characters.
/// - The `prefixed` field is always `sha256:{hex}`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContentDigest {
    /// Raw hex string (64 characters, lowercase).
    pub hex: String,
    /// OCI-style prefixed string: `sha256:{hex}`.
    pub prefixed: String,
}

impl ContentDigest {
    /// Compute the SHA-256 digest of a byte slice.
    ///
    /// // WARM PATH — called per layer during pack/unpack.
    /// Allocation: one String for the hex output.
    pub fn of_bytes(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let hex = hex::encode(result);
        let prefixed = format!("sha256:{hex}");
        Self { hex, prefixed }
    }

    /// Compute the SHA-256 digest of a file on disk.
    ///
    /// Reads the file in 64 KiB chunks to avoid loading the entire
    /// file into memory (important for large Wasm binaries).
    ///
    /// // WARM PATH — called per layer during pack.
    ///
    /// # Errors
    /// Returns an I/O error if the file cannot be read.
    pub fn of_file(path: &Path) -> Result<Self, std::io::Error> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536]; // 64 KiB read buffer
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let result = hasher.finalize();
        let hex = hex::encode(result);
        let prefixed = format!("sha256:{hex}");
        Ok(Self { hex, prefixed })
    }

    /// Parse a prefixed digest string (`sha256:{hex}`).
    ///
    /// Returns `None` if the format is invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix("sha256:")?;
        if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self {
            hex: hex.to_lowercase(),
            prefixed: format!("sha256:{}", hex.to_lowercase()),
        })
    }

    /// Verify that a byte slice matches this digest.
    ///
    /// // WARM PATH — called during pull verification.
    pub fn verify(&self, data: &[u8]) -> bool {
        Self::of_bytes(data).hex == self.hex
    }
}

impl std::fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.prefixed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_of_empty_is_known_hash() {
        let d = ContentDigest::of_bytes(b"");
        // SHA-256 of empty string is well-known.
        assert_eq!(
            d.hex,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert!(d.prefixed.starts_with("sha256:"));
    }

    #[test]
    fn digest_of_hello_world() {
        let d = ContentDigest::of_bytes(b"hello world");
        assert_eq!(d.hex.len(), 64);
        assert!(d.prefixed.starts_with("sha256:"));
    }

    #[test]
    fn same_input_produces_same_digest() {
        let a = ContentDigest::of_bytes(b"test data");
        let b = ContentDigest::of_bytes(b"test data");
        assert_eq!(a, b);
    }

    #[test]
    fn different_input_produces_different_digest() {
        let a = ContentDigest::of_bytes(b"data a");
        let b = ContentDigest::of_bytes(b"data b");
        assert_ne!(a, b);
    }

    #[test]
    fn verify_correct_data_passes() {
        let d = ContentDigest::of_bytes(b"verify me");
        assert!(d.verify(b"verify me"));
    }

    #[test]
    fn verify_wrong_data_fails() {
        let d = ContentDigest::of_bytes(b"original");
        assert!(!d.verify(b"tampered"));
    }

    #[test]
    fn parse_valid_prefixed_digest() {
        let d = ContentDigest::of_bytes(b"hello");
        let parsed = ContentDigest::parse(&d.prefixed).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn parse_invalid_prefix_returns_none() {
        assert!(ContentDigest::parse("md5:abc123").is_none());
    }

    #[test]
    fn parse_wrong_length_returns_none() {
        assert!(ContentDigest::parse("sha256:abc").is_none());
    }

    #[test]
    fn digest_of_file_matches_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        let data = b"file content for hashing";
        std::fs::write(&path, data).unwrap();

        let from_file = ContentDigest::of_file(&path).unwrap();
        let from_bytes = ContentDigest::of_bytes(data);
        assert_eq!(from_file, from_bytes);
    }

    #[test]
    fn digest_of_file_not_found_is_error() {
        let result = ContentDigest::of_file(Path::new("/nonexistent/file.bin"));
        assert!(result.is_err());
    }
}
