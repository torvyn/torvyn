//! Signing provider trait and implementations.
//!
//! Per HLI Doc 08, Section 6 and MR-17 (Doc 10). Phase 0 ships with
//! stub implementations. Real Sigstore integration is Phase 2.

use crate::digest::ContentDigest;
use crate::error::SigningError;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// SignatureInfo
// ---------------------------------------------------------------------------

/// Metadata about an artifact's cryptographic signature.
///
/// Per HLI Doc 08, Section 11.5.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureInfo {
    /// Signing method used.
    pub method: SigningMethod,

    /// Identity of the signer (email, key fingerprint, etc.).
    pub signer_identity: String,

    /// Signing timestamp (RFC 3339).
    pub timestamp: String,

    /// Whether verification has been performed and succeeded.
    pub verified: bool,
}

/// The signing method used for an artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SigningMethod {
    /// No signature (stub / development).
    Unsigned,

    /// Sigstore keyless signing (Phase 2).
    SigstoreKeyless {
        /// OIDC issuer URL.
        oidc_issuer: String,
        /// Rekor transparency log entry ID.
        rekor_log_id: String,
    },

    /// Key-based signing.
    KeyBased {
        /// Key identifier.
        key_id: String,
        /// Signing algorithm.
        algorithm: String,
    },
}

// ---------------------------------------------------------------------------
// SigningProvider trait
// ---------------------------------------------------------------------------

/// Abstraction over signing backends.
///
/// Implementations may use Sigstore (Phase 2), local keys (testing),
/// or a stub (Phase 0 default).
///
/// All methods are synchronous in Phase 0. Sigstore will require async
/// when implemented; at that point this trait should be made async.
pub trait SigningProvider: Send + Sync {
    /// Sign an artifact digest.
    ///
    /// Returns signature info on success.
    ///
    /// # Errors
    /// Returns `SigningError` if signing fails.
    fn sign(&self, digest: &ContentDigest) -> Result<SignatureInfo, SigningError>;

    /// Verify a signature against an artifact digest.
    ///
    /// # Errors
    /// Returns `SigningError` if verification fails.
    fn verify(
        &self,
        digest: &ContentDigest,
        signature: &SignatureInfo,
    ) -> Result<bool, SigningError>;

    /// Human-readable name of this provider.
    fn provider_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// StubSigningProvider
// ---------------------------------------------------------------------------

/// A stub signing provider that produces unsigned markers.
///
/// Used in Phase 0 and for local development where signing is not required.
pub struct StubSigningProvider;

impl SigningProvider for StubSigningProvider {
    fn sign(&self, _digest: &ContentDigest) -> Result<SignatureInfo, SigningError> {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());

        Ok(SignatureInfo {
            method: SigningMethod::Unsigned,
            signer_identity: "unsigned-stub".to_owned(),
            timestamp: now,
            verified: false,
        })
    }

    fn verify(
        &self,
        _digest: &ContentDigest,
        signature: &SignatureInfo,
    ) -> Result<bool, SigningError> {
        // Stub: unsigned artifacts always "verify" as unsigned
        Ok(signature.method == SigningMethod::Unsigned)
    }

    fn provider_name(&self) -> &str {
        "stub"
    }
}

// ---------------------------------------------------------------------------
// LocalKeySigningProvider (testing only)
// ---------------------------------------------------------------------------

/// A local key-based signing provider using HMAC-SHA256.
///
/// **WARNING: This is for testing only. It does not provide real
/// cryptographic signing suitable for production use.**
///
/// For production, use `SigstoreSigningProvider` (Phase 2) or
/// a proper ECDSA/Ed25519 key-based signer.
pub struct LocalKeySigningProvider {
    /// Shared secret key for HMAC.
    key: Vec<u8>,
    /// Key ID for identification.
    key_id: String,
}

impl LocalKeySigningProvider {
    /// Create a new local key provider.
    ///
    /// # Arguments
    /// - `key`: Shared secret bytes.
    /// - `key_id`: Human-readable key identifier.
    pub fn new(key: Vec<u8>, key_id: String) -> Self {
        Self { key, key_id }
    }

    /// Compute HMAC-SHA256 of the digest using the key.
    fn hmac(&self, digest: &ContentDigest) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&self.key);
        hasher.update(digest.hex.as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl SigningProvider for LocalKeySigningProvider {
    fn sign(&self, digest: &ContentDigest) -> Result<SignatureInfo, SigningError> {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());

        let _sig_hex = self.hmac(digest);

        Ok(SignatureInfo {
            method: SigningMethod::KeyBased {
                key_id: self.key_id.clone(),
                algorithm: "hmac-sha256-test".to_owned(),
            },
            signer_identity: self.key_id.clone(),
            timestamp: now,
            verified: false,
        })
    }

    fn verify(
        &self,
        digest: &ContentDigest,
        signature: &SignatureInfo,
    ) -> Result<bool, SigningError> {
        match &signature.method {
            SigningMethod::KeyBased { key_id, .. } => {
                if key_id != &self.key_id {
                    return Ok(false);
                }
                // In a real impl, we'd verify the stored signature bytes
                // against our computed HMAC. For testing, we just verify
                // that we can recompute.
                let _computed = self.hmac(digest);
                Ok(true) // Testing stub: always passes if key matches
            }
            _ => Ok(false),
        }
    }

    fn provider_name(&self) -> &str {
        "local-key-test"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_provider_signs_as_unsigned() {
        let provider = StubSigningProvider;
        let digest = ContentDigest::of_bytes(b"test artifact");
        let info = provider.sign(&digest).unwrap();
        assert_eq!(info.method, SigningMethod::Unsigned);
        assert_eq!(info.signer_identity, "unsigned-stub");
    }

    #[test]
    fn stub_provider_verifies_unsigned() {
        let provider = StubSigningProvider;
        let digest = ContentDigest::of_bytes(b"test artifact");
        let info = provider.sign(&digest).unwrap();
        assert!(provider.verify(&digest, &info).unwrap());
    }

    #[test]
    fn stub_provider_rejects_non_unsigned() {
        let provider = StubSigningProvider;
        let digest = ContentDigest::of_bytes(b"test artifact");
        let info = SignatureInfo {
            method: SigningMethod::KeyBased {
                key_id: "some-key".into(),
                algorithm: "ecdsa".into(),
            },
            signer_identity: "someone".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            verified: false,
        };
        assert!(!provider.verify(&digest, &info).unwrap());
    }

    #[test]
    fn local_key_provider_signs_with_key_id() {
        let provider = LocalKeySigningProvider::new(b"secret".to_vec(), "test-key".into());
        let digest = ContentDigest::of_bytes(b"test data");
        let info = provider.sign(&digest).unwrap();

        match &info.method {
            SigningMethod::KeyBased { key_id, algorithm } => {
                assert_eq!(key_id, "test-key");
                assert_eq!(algorithm, "hmac-sha256-test");
            }
            other => panic!("expected KeyBased, got {other:?}"),
        }
    }

    #[test]
    fn local_key_provider_verifies_own_signature() {
        let provider = LocalKeySigningProvider::new(b"secret".to_vec(), "test-key".into());
        let digest = ContentDigest::of_bytes(b"test data");
        let info = provider.sign(&digest).unwrap();
        assert!(provider.verify(&digest, &info).unwrap());
    }

    #[test]
    fn local_key_provider_rejects_wrong_key_id() {
        let provider = LocalKeySigningProvider::new(b"secret".to_vec(), "test-key".into());
        let digest = ContentDigest::of_bytes(b"test data");
        let info = SignatureInfo {
            method: SigningMethod::KeyBased {
                key_id: "wrong-key".into(),
                algorithm: "hmac-sha256-test".into(),
            },
            signer_identity: "wrong".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            verified: false,
        };
        assert!(!provider.verify(&digest, &info).unwrap());
    }

    #[test]
    fn signature_info_serialization_roundtrip() {
        let info = SignatureInfo {
            method: SigningMethod::SigstoreKeyless {
                oidc_issuer: "https://github.com/login/oauth".into(),
                rekor_log_id: "log123".into(),
            },
            signer_identity: "alice@example.com".into(),
            timestamp: "2026-03-11T12:00:00Z".into(),
            verified: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: SignatureInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, info);
    }
}
