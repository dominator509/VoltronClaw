use std::collections::HashMap;
use std::collections::HashSet;

use voltron_core::{
    CapabilityManifest, ManifestVerifier, SignedManifest, VerificationError,
};

// ── ManifestRegistry ───────────────────────────────────────────────

/// Registry of known capability manifests, keyed by skill name.
///
/// Used by [`IronclawManifestVerifier`] to look up the expected
/// manifest for a given skill before executing it.
#[derive(Debug, Default)]
pub struct ManifestRegistry {
    manifests: HashMap<String, CapabilityManifest>,
}

impl ManifestRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a manifest for a skill.
    pub fn register(&mut self, manifest: CapabilityManifest) {
        let name = manifest.skill_name.clone();
        self.manifests.insert(name, manifest);
    }

    /// Look up a manifest by skill name.
    pub fn lookup(&self, skill_name: &str) -> Option<&CapabilityManifest> {
        self.manifests.get(skill_name)
    }
}

// ── RevocationRegistry ─────────────────────────────────────────────

/// Tracks revoked skill manifests by skill name.
///
/// When a manifest is revoked, all execution attempts for that skill
/// are rejected with [`VerificationError::Revoked`].
#[derive(Debug, Default)]
pub struct RevocationRegistry {
    revoked: HashSet<String>,
}

impl RevocationRegistry {
    /// Create an empty revocation set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a skill as revoked.
    pub fn revoke(&mut self, skill_name: &str) {
        self.revoked.insert(skill_name.to_string());
    }

    /// Un-revoke a skill.
    pub fn unrevoke(&mut self, skill_name: &str) {
        self.revoked.remove(skill_name);
    }

    /// Check whether a skill is revoked.
    pub fn check(&self, skill_name: &str) -> bool {
        self.revoked.contains(skill_name)
    }
}

// ── IronclawManifestVerifier ───────────────────────────────────────

/// Concrete [`ManifestVerifier`] that performs Ed25519 verification,
/// expiry checking, content hash validation, permission checks, and
/// revocation checks.
///
/// The verifier holds its own registry of signed manifests keyed by
/// skill name, allowing lookup by name via [`verify_skill_by_name`].
pub struct IronclawManifestVerifier {
    /// Known signed manifests (keyed by skill_name).
    signed_manifests: HashMap<String, SignedManifest>,
    /// Expected capability manifests for hash/permission checks.
    manifest_registry: ManifestRegistry,
    /// Revoked skill names.
    revocation_registry: RevocationRegistry,
}

impl IronclawManifestVerifier {
    /// Create a new verifier with the given registries.
    pub fn new(
        manifest_registry: ManifestRegistry,
        revocation_registry: RevocationRegistry,
    ) -> Self {
        Self {
            signed_manifests: HashMap::new(),
            manifest_registry,
            revocation_registry,
        }
    }

    /// Register a signed manifest for a skill.
    ///
    /// This is how signed manifests are loaded from a manifest directory
    /// and wired into the verifier.
    pub fn register_signed(&mut self, signed: SignedManifest) {
        let name = signed.manifest.skill_name.clone();
        self.signed_manifests.insert(name, signed);
    }
}

impl ManifestVerifier for IronclawManifestVerifier {
    fn verify_manifest(
        &self,
        signed: &SignedManifest,
    ) -> Result<CapabilityManifest, VerificationError> {
        let skill_name = &signed.manifest.skill_name;

        // 1. Check revocation status
        if self.revocation_registry.check(skill_name) {
            return Err(VerificationError::Revoked {
                skill_name: skill_name.clone(),
            });
        }

        // 2. Verify Ed25519 signature
        if let Err(_e) = signed.verify() {
            return Err(VerificationError::InvalidSignature {
                skill_name: skill_name.clone(),
            });
        }

        // 3. Check expiry
        if let Some(expires_at) = signed.manifest.expires_at {
            if chrono::Utc::now() > expires_at {
                return Err(VerificationError::Expired {
                    skill_name: skill_name.clone(),
                    expired_at: expires_at.to_rfc3339(),
                });
            }
        }

        // 4. Verify content hash against registered manifest
        if let Some(registered) = self.manifest_registry.lookup(skill_name) {
            if !signed.manifest.verify_hash(&registered.content_hash) {
                return Err(VerificationError::HashMismatch {
                    skill_name: skill_name.clone(),
                });
            }
        }

        // 5. Check permissions (every declared permission must be in the registered manifest)
        if let Some(registered) = self.manifest_registry.lookup(skill_name) {
            for perm in &signed.manifest.permissions {
                if !registered.permissions.contains(perm) {
                    return Err(VerificationError::PermissionViolation {
                        skill_name: skill_name.clone(),
                        permission: perm.clone(),
                    });
                }
            }
        }

        Ok(signed.manifest.clone())
    }

    fn verify_skill_by_name(
        &self,
        skill_name: &str,
    ) -> Result<CapabilityManifest, VerificationError> {
        let signed = self.signed_manifests.get(skill_name).ok_or_else(|| {
            VerificationError::InvalidSignature {
                skill_name: skill_name.to_string(),
            }
        })?;
        self.verify_manifest(signed)
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use voltron_core::{CapabilityManifest, SignedManifest};

    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use rand::RngCore;

    /// Helper: create a valid signing key and sign a manifest.
    fn make_signed(
        skill_name: &str,
        permissions: Vec<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        content_hash: [u8; 32],
    ) -> (SignedManifest, SigningKey, [u8; 32]) {
        let mut secret_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let public_key = signing_key.verifying_key().to_bytes();

        let manifest = CapabilityManifest {
            skill_name: skill_name.to_string(),
            version: semver::Version::new(1, 0, 0),
            permissions,
            content_hash,
            expires_at,
            metadata: serde_json::json!({}),
        };

        let canonical = manifest.canonical_bytes();
        use ed25519_dalek::Signer;
        let signature = signing_key.sign(&canonical).to_bytes().to_vec();

        let signed = SignedManifest {
            manifest,
            signature,
            public_key,
        };

        (signed, signing_key, public_key)
    }

    // ── Test 1: Valid manifest + valid signature → PASS ─────────────

    #[test]
    fn test_valid_manifest_passes() {
        let hash = [0u8; 32];
        let (signed, _sk, _pk) = make_signed("test_skill", vec![], None, hash);

        let mut registry = ManifestRegistry::new();
        registry.register(CapabilityManifest {
            skill_name: "test_skill".to_string(),
            version: semver::Version::new(1, 0, 0),
            permissions: vec![],
            content_hash: hash,
            expires_at: None,
            metadata: serde_json::json!({}),
        });

        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        let result = verifier.verify_manifest(&signed);
        assert!(result.is_ok(), "Expected OK, got: {:?}", result.err());
    }

    // ── Test 2: Wrong signature → InvalidSignature ──────────────────

    #[test]
    fn test_wrong_signature_fails() {
        let hash = [0u8; 32];
        let (mut signed, _sk, _pk) = make_signed("test_skill", vec![], None, hash);

        // Tamper with the signature
        if let Some(b) = signed.signature.last_mut() {
            *b = b.wrapping_add(1);
        }

        let registry = ManifestRegistry::new();
        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        let result = verifier.verify_manifest(&signed);
        assert!(
            matches!(result, Err(VerificationError::InvalidSignature { .. })),
            "Expected InvalidSignature, got: {:?}",
            result
        );
    }

    // ── Test 3: Expired manifest → Expired ──────────────────────────

    #[test]
    fn test_expired_manifest_fails() {
        let hash = [0u8; 32];
        let past = chrono::Utc::now() - chrono::Duration::hours(1);
        let (signed, _sk, _pk) = make_signed("old_skill", vec![], Some(past), hash);

        let registry = ManifestRegistry::new();
        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        let result = verifier.verify_manifest(&signed);
        assert!(
            matches!(result, Err(VerificationError::Expired { .. })),
            "Expected Expired, got: {:?}",
            result
        );
    }

    // ── Test 4: Tampered content (hash mismatch) → HashMismatch ─────

    #[test]
    fn test_hash_mismatch_fails() {
        let declared_hash = [0u8; 32];
        let actual_hash = [1u8; 32]; // different!
        let (signed, _sk, _pk) = make_signed("test_skill", vec![], None, declared_hash);

        let mut registry = ManifestRegistry::new();
        registry.register(CapabilityManifest {
            skill_name: "test_skill".to_string(),
            version: semver::Version::new(1, 0, 0),
            permissions: vec![],
            content_hash: actual_hash,
            expires_at: None,
            metadata: serde_json::json!({}),
        });

        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        let result = verifier.verify_manifest(&signed);
        assert!(
            matches!(result, Err(VerificationError::HashMismatch { .. })),
            "Expected HashMismatch, got: {:?}",
            result
        );
    }

    // ── Test 5: Missing manifest for skill → permissions unknown ────

    #[test]
    fn test_missing_manifest_for_skill() {
        let hash = [0u8; 32];
        let (signed, _sk, _pk) = make_signed("unknown_skill", vec![], None, hash);

        // Empty registry — no manifest registered for "unknown_skill"
        let registry = ManifestRegistry::new();
        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        // Should still pass signature+expiry checks since those don't require registry,
        // but hash check is skipped when no registry entry exists
        let result = verifier.verify_manifest(&signed);
        assert!(result.is_ok(), "Expected OK (no registry = no hash check), got: {:?}", result.err());
    }

    // ── Test 6: Revoked manifest → Revoked ──────────────────────────

    #[test]
    fn test_revoked_manifest_fails() {
        let hash = [0u8; 32];
        let (signed, _sk, _pk) = make_signed("revoked_skill", vec![], None, hash);

        let mut revocations = RevocationRegistry::new();
        revocations.revoke("revoked_skill");

        let registry = ManifestRegistry::new();
        let verifier = IronclawManifestVerifier::new(registry, revocations);
        let result = verifier.verify_manifest(&signed);
        assert!(
            matches!(result, Err(VerificationError::Revoked { .. })),
            "Expected Revoked, got: {:?}",
            result
        );
    }

    // ── Test 7: Permission violation ────────────────────────────────

    #[test]
    fn test_permission_violation_fails() {
        let hash = [0u8; 32];
        let (signed, _sk, _pk) = make_signed(
            "perm_skill",
            vec!["fs:read".into(), "net:connect".into()],
            None,
            hash,
        );

        let mut registry = ManifestRegistry::new();
        // Registered manifest only allows "fs:read"
        registry.register(CapabilityManifest {
            skill_name: "perm_skill".to_string(),
            version: semver::Version::new(1, 0, 0),
            permissions: vec!["fs:read".into()],
            content_hash: hash,
            expires_at: None,
            metadata: serde_json::json!({}),
        });

        let verifier = IronclawManifestVerifier::new(registry, RevocationRegistry::new());
        let result = verifier.verify_manifest(&signed);
        assert!(
            matches!(result, Err(VerificationError::PermissionViolation { .. })),
            "Expected PermissionViolation, got: {:?}",
            result
        );
    }

    // ── Test 8: Revocation + un-revoke ──────────────────────────────

    #[test]
    fn test_unrevoke_works() {
        let hash = [0u8; 32];
        let (signed, _sk, _pk) = make_signed("temp_revoked", vec![], None, hash);

        let mut revocations = RevocationRegistry::new();
        revocations.revoke("temp_revoked");
        revocations.unrevoke("temp_revoked");

        let mut registry = ManifestRegistry::new();
        registry.register(CapabilityManifest {
            skill_name: "temp_revoked".to_string(),
            version: semver::Version::new(1, 0, 0),
            permissions: vec![],
            content_hash: hash,
            expires_at: None,
            metadata: serde_json::json!({}),
        });

        let verifier = IronclawManifestVerifier::new(registry, revocations);
        let result = verifier.verify_manifest(&signed);
        assert!(result.is_ok(), "Expected OK after unrevoke, got: {:?}", result.err());
    }
}
