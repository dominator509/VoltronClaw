//! voltron-ironclaw-adapter — Signed-skill capability-manifest pipeline.
//!
//! Provides [`IronclawManifestVerifier`], a concrete implementation of
//! [`ManifestVerifier`] that cryptographically verifies Ed25519-signed
//! skill manifests before execution.
//!
//! # Architecture
//!
//! ```text
//! IronclawManifestVerifier
//!   ├── ManifestRegistry   — stores known capability manifests
//!   ├── RevocationRegistry — tracks revoked manifests
//!   └── Ed25519            — signature verification (ed25519-dalek)
//! ```
//!
//! When wired into [`AgentRuntime`] via the builder, every skill
//! dispatch is gated by a call to [`IronclawManifestVerifier::verify`].
//! Skills without a valid signed manifest are rejected with a
//! [`VerificationError`].
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use voltron_core::ManifestVerifier;
//! use voltron_ironclaw_adapter::{
//!     IronclawManifestVerifier, ManifestRegistry, RevocationRegistry,
//! };
//!
//! let registry = ManifestRegistry::new();
//! let revocations = RevocationRegistry::new();
//! let verifier = Arc::new(
//!     IronclawManifestVerifier::new(registry, revocations),
//! ) as Arc<dyn ManifestVerifier>;
//! ```

pub mod verifier;

pub use verifier::{IronclawManifestVerifier, ManifestRegistry, RevocationRegistry};
