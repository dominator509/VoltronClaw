use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Data types ────────────────────────────────────────────────────

/// A signed-capability manifest describing what permissions a skill
/// requires and attesting to its content integrity.
///
/// This struct is serialised to deterministic CBOR for Ed25519 signing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityManifest {
    /// Unique skill name (e.g., "filesystem::read").
    pub skill_name: String,
    /// Semantic version of the manifest / skill interface.
    pub version: semver::Version,
    /// Declared permissions (e.g., "fs:read", "net:connect").
    #[serde(default)]
    pub permissions: Vec<String>,
    /// SHA-256 content hash of the skill implementation (32 bytes).
    pub content_hash: [u8; 32],
    /// Optional expiry timestamp. `None` means the manifest does not
    /// expire.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Arbitrary metadata carried by the manifest.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl CapabilityManifest {
    /// Serialise the manifest to deterministic CBOR bytes for signing.
    ///
    /// Uses [`ciborium`] with canonical (deterministic) CBOR encoding
    /// so the same logical manifest always produces the same byte
    /// sequence.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .expect("deterministic CBOR serialisation of CapabilityManifest");
        buf
    }

    /// Verify that the given 32-byte hash matches this manifest's
    /// `content_hash` field.
    pub fn verify_hash(&self, expected: &[u8; 32]) -> bool {
        &self.content_hash == expected
    }
}

/// A [`CapabilityManifest`] wrapped with an Ed25519 signature and the
/// public key that produced it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedManifest {
    /// The capability manifest being attested.
    pub manifest: CapabilityManifest,
    /// Ed25519 signature over `manifest.canonical_bytes()`.
    #[serde(with = "serde_bytes")]
    pub signature: Vec<u8>,
    /// Ed25519 public key used to verify the signature.
    pub public_key: [u8; 32],
}

impl SignedManifest {
    /// Verify the Ed25519 signature against the manifest's canonical
    /// bytes.
    ///
    /// Returns `Ok(())` if the signature is valid, or a
    /// [`SignatureError`] (re-exported from `ed25519_dalek`).
    pub fn verify(&self) -> Result<(), ed25519_dalek::SignatureError> {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        let verifying_key = VerifyingKey::from_bytes(&self.public_key)?;
        let sig = Signature::from_slice(&self.signature)?;
        verifying_key.verify(&self.manifest.canonical_bytes(), &sig)
    }
}

// ── Verification error ─────────────────────────────────────────────

/// Errors that can occur during capability-manifest verification.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum VerificationError {
    /// Ed25519 signature does not match the manifest bytes.
    #[error("invalid signature for manifest '{skill_name}'")]
    InvalidSignature { skill_name: String },

    /// The manifest's `expires_at` timestamp is in the past.
    #[error("manifest for '{skill_name}' expired at {expired_at}")]
    Expired {
        skill_name: String,
        expired_at: String,
    },

    /// The manifest content hash does not match the computed hash.
    #[error("content hash mismatch for manifest '{skill_name}'")]
    HashMismatch { skill_name: String },

    /// The manifest requests a permission not granted by its policy.
    #[error("permission violation: skill '{skill_name}' lacks permission '{permission}'")]
    PermissionViolation {
        skill_name: String,
        permission: String,
    },

    /// The skill's manifest has been revoked.
    #[error("manifest for skill '{skill_name}' is revoked")]
    Revoked { skill_name: String },
}

// ── ManifestVerifier trait ─────────────────────────────────────────

/// Verifier for signed skill capability manifests.
///
/// Implementations check cryptographic signatures, expiry, content
/// integrity, permissions, and revocation status before allowing a
/// skill to execute.
///
/// This trait is **optional** — runtimes without a verifier wired in
/// execute all skills unconditionally.
pub trait ManifestVerifier: Send + Sync {
    /// Verify a signed capability manifest.
    ///
    /// Returns the verified [`CapabilityManifest`] on success, or a
    /// [`VerificationError`] describing the failure.
    fn verify_manifest(
        &self,
        signed: &SignedManifest,
    ) -> Result<CapabilityManifest, VerificationError>;

    /// Verify a skill by name, looking up the signed manifest internally.
    ///
    /// The default implementation returns [`VerificationError::InvalidSignature`]
    /// — implementors should override this to perform the lookup from their
    /// internal registry.
    fn verify_skill_by_name(&self, _skill_name: &str) -> Result<CapabilityManifest, VerificationError> {
        Err(VerificationError::InvalidSignature {
            skill_name: _skill_name.to_string(),
        })
    }
}
