# VOLTRON_LINK — Voltron Claw Coordination Buffer
# DeepSeek Cache Tier: PREFIX-STABLE (no timestamps, fixed slots, surgical edits only)
# EDIT POLICY: edit_file only. Never write_file on this buffer. No dates, no From: sigs.
# Single source of truth for all three agents. ACTIVE_PHASE is authoritative.

## [SYSTEM_STATE]
ACTIVE_PHASE=PHASE_3
ACTIVE_STEP=7.1_IRONCLAW_IMPLEMENT
BUILD_MODE=GREENFIELD
BLOCKED=FALSE
BLOCK_REASON=

## [ALFRED_ORCHESTRATOR]
TASK=ORCHESTRATING: Phase 3 — IronClaw signed-skill / capability-manifest pipeline. Step 7.1: Ip Man assigned to implement voltron-ironclaw-adapter crate. Full spec in IP_MAN_CODER slot.
AUDIT_ASSIGNMENT=Deziray — audit after Ip Man completes implementation and tests pass
STATUS=ORCHESTRATING — Ip Man active

## [IP_MAN_CODER]
CURRENT_TASK=PHASE_3_IRONCLAW: Implement voltron-ironclaw-adapter crate. Signed-skill / capability-manifest pipeline requiring cryptographic verification of skill capabilities before execution. GREENFIELD implementation (no external repo — IronClaw is operator-owned).
IMPLEMENTED_CRATES=voltron-providers, voltron-memory, voltron-skills, voltron-channels, voltron-audit, src/main.rs, voltron-runtime, voltron-moltis-adapter
STATUS=ACTIVE
BLOCKERS=None
FINDINGS=
SPEC=
  Create `crates/voltron-ironclaw-adapter/` with standard Cargo.toml depending on voltron-core (path = "../voltron-core").

  MODULE 1 — CapabilityManifest (src/manifest.rs):
    Struct with fields: skill_name: String, version: semver::Version, permissions: Vec<String>, content_hash: [u8; 32], expires_at: Option<chrono::DateTime<chrono::Utc>>, metadata: serde_json::Value.
    Methods: canonical_bytes() -> Vec<u8> (deterministic CBOR serialization for signing), verify_hash(expected: &[u8; 32]) -> bool.

  MODULE 2 — SignedManifest (src/signed.rs):
    Struct with fields: manifest: CapabilityManifest, signature: Vec<u8>, public_key: [u8; 32].
    Uses ed25519-dalek crate for Ed25519 signature verification. Add dependency: ed25519-dalek = "2".
    Method: verify() -> Result<(), SignatureError>.

  MODULE 3 — ManifestVerifier trait (in voltron-core, added to src/lib.rs):
    pub trait ManifestVerifier: Send + Sync {
        fn verify_manifest(&self, signed: &SignedManifest) -> Result<CapabilityManifest, VerificationError>;
    }
    Also define VerificationError enum (InvalidSignature, Expired, HashMismatch, PermissionViolation, Revoked).

  MODULE 4 — IronclawManifestVerifier (src/verifier.rs):
    Implements ManifestVerifier. Verifies Ed25519 signature, checks expiry, validates content hash, checks permissions.
    Integration point: wraps existing skill manifests — calls self.manifest_registry.lookup(&signed.manifest.skill_name) and self.revocation_registry.check(&signed.manifest.skill_name).

  MODULE 5 — Integration with SkillExecutor (in voltron-skills or voltron-runtime):
    Add an optional ManifestVerifier to LocalSkillExecutor or AgentRuntime. When set, verify_manifest() is called before execute() dispatches a skill. Unsigned/invalid skills return VerificationError and produce an audit entry.
    The verifier is optional (core traits unchanged) — gates are active only when a verifier is wired in.

  MODULE 6 — Tests (src/tests.rs or tests/):
    - Valid manifest + valid signature → PASS
    - Valid manifest + wrong signature → SignatureError
    - Expired manifest → Expired error
    - Tampered content (hash mismatch) → HashMismatch error
    - Missing manifest for skill → verification failure
    - Integration: LocalSkillExecutor with verifier rejects unsigned skill call

  INTEGRATION GOAL: main.rs gains `--ironclaw-manifest-dir` flag. When set, AgentRuntime::builder().manifest_verifier(verifier) wires IronClaw into the run loop. Skills without signed manifests are rejected. Skills with valid manifests execute normally.

  COMMIT after implementation: `feat(ironclaw): implement signed-skill capability-manifest pipeline`

## [DEZIRAY_AUDITOR]
CURRENT_AUDIT=PENDING: Phase 3 IronClaw voltron-ironclaw-adapter — Ip Man implementing signed-skill pipeline. Audit after code committed and tests pass.
PHASE_1_AUDIT=COMPLETE — all 7 crates audited, 5 remediation items verified passing, 57+ tests confirmed
PHASE_2_AUDIT=COMPLETE — crates/voltron-moltis-adapter/ audited, 18/18 tests, PASS verdict
STATUS=STANDBY — awaiting Ip Man's IronClaw implementation for audit
FINDINGS=
  [AUDIT-6.6:PASS] Moltis adapter crate audit complete. Four criteria verified:
    (1) Trait conformance — MoltisChannel impl ChannelAdapter, MoltisSkillBridge impl SkillExecutor, MoltisAuditRelay impl AuditSink. All signatures correct, all three object-safe (compile-time check at L1057). MoltisAgentRuntime bundles all three with clean builder pattern. PASS.
    (2) Workspace registration — Cargo.toml depends on voltron-core via path dependency. Root workspace members includes crates/voltron-moltis-adapter. Workspace deps (tokio, serde, async-trait, etc.) used correctly. PASS.
    (3) Unit tests — 18 tests covering channel (memory mode, inject/recv, send, second-recv-empty), skill (list_create success+missing, list_search, offer_respond accept+decline+missing, unknown, mock, manifests, manifest lookup), audit relay append, runtime builder (memory+NATS fallback), object safety. All 18 tests present. PASS.
    (4) No unsafe or panic — Zero unsafe blocks. All 16 unwrap() calls are in #[cfg(test)] only. Production code uses proper error handling (map_err, timeout, match, unwrap_or_else fallback). PASS.
    Verdict: MOLTIS_ADAPTER_APPROVED. Step 6.6 complete, ready for step advancement.
  [F1:CRITICAL] src/main.rs `_model_override` stubbed — `--model` CLI flag and config `model` field captured (L87-90) but never passed to `DeepSeekProvider::from_env()` or `OpenAIProvider::from_env()`. Both call `Self::new(&api_key, None)`, defaulting to `DEEPSEEK_CHAT`/`GPT_4O_MINI`. `.with_model()` method exists but unused. ASSIGNEE=IP_MAN
  [F2:MEDIUM] voltron-runtime/lib.rs `uuid_v4()` (L517-534) uses timestamp arithmetic instead of RFC 4122 random bytes — collision risk under high throughput. Should use `uuid` crate. ASSIGNEE=IP_MAN
  [F3:LOW] voltron-runtime/lib.rs custom `iso_now()` (L536-579) hand-rolled date math — works for common cases but has edge-case risk (leap year boundaries). ASSIGNEE=IP_MAN
  [F4:LOW] No integration test for `run_loop()` — only `process_message()` unit tests exist. ASSIGNEE=IP_MAN
  [F5:LOW] Tool iteration exhaustion at `max_tool_iterations` silently drops last iteration's tool results. ASSIGNEE=IP_MAN
  [AUDIT-6.2:PASS] Moltis license audit — APPREVED_FOR_INTAKE. ASSIGNEE=DEZIRAY
  [AUDIT-6.4:PASS] Third-party placement audit — all docs updated. ASSIGNEE=DEZIRAY

## [ACK_MATRIX]
ACK_IP_MAN=FALSE
ACK_DEZIRAY=FALSE
NEEDS_ALFRED=FALSE