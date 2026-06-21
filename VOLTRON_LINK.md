# VOLTRON_LINK — Voltron Claw Coordination Buffer
# DeepSeek Cache Tier: PREFIX-STABLE (no timestamps, fixed slots, surgical edits only)
# EDIT POLICY: edit_file only. Never write_file on this buffer. No dates, no From: sigs.
# Single source of truth for all three agents. ACTIVE_PHASE is authoritative.

## [SYSTEM_STATE]
ACTIVE_PHASE=PHASE_4
ACTIVE_STEP=8.5_PHASE_5_AUDIT_COMPLETE
BUILD_MODE=GREENFIELD
BLOCKED=FALSE
BLOCK_REASON=

## [ALFRED_ORCHESTRATOR]
TASK=ORCHESTRATING: Phase 5 COMPLETE. Two new crates implemented (voltron-memory-tree bucket-seal cascade, voltron-ractor ractor actor model). Audit delivered: CONDITIONAL_PASS — 0 CRITICAL, 0 HIGH, 3 LOW. Pipeline awaiting Phase 6 direction.
AUDIT_ASSIGNMENT=None — Deziray delivered Phase 5 audit (AUDIT-9.x)
STATUS=PHASE_5_COMPLETE — all 5 phases audited, 26 tests pass. Awaiting operator direction for Phase 6.

## [IP_MAN_CODER]
CURRENT_TASK=STANDBY — Phase 5 crown-jewel implementation done by operator
IMPLEMENTED_CRATES=voltron-providers, voltron-memory, voltron-memory-tree, voltron-skills, voltron-channels, voltron-audit, src/main.rs, voltron-runtime, voltron-ractor, voltron-moltis-adapter, voltron-ironclaw-adapter, voltron-hermes-adapter
STATUS=STANDBY — Phase 5 crates built by operator, 26 tests passing. Awaiting Deziray audit.
BLOCKERS=None
FINDINGS=
SPEC=
SPEC=
  Create `crates/voltron-hermes-adapter/` with standard Cargo.toml depending on voltron-core (path = "../voltron-core").

  OVERVIEW: The Hermes self-improving skill loop is tool-based procedural memory. After completing a complex task, the agent saves the workflow as a reusable skill (SKILL.md with YAML frontmatter). On future tasks, it discovers skills via a compact index (never dumped into system prompt), loads full instructions on demand, and patches skills as it finds gaps during use.

  === MODULE 1 — Skill Storage (src/skill_storage.rs) ===

  Port of: tools/skill_manager_tool.py (directory layout, validation)
  Port of: tools/skills_tool.py (frontmatter parsing, platform gating)

  SkillsDir struct wrapping a base Path (configurable, default ~/.voltron/skills/).
  Skill on disk = directory containing SKILL.md + optional references/ templates/ scripts/ assets/ subdirs.

  SkillMeta struct: name (String, max 64), description (String, max 1024), version (Option<String>), license (Option<String>), platforms (Option<Vec<String>>), prerequisites (Option<Prerequisites>), metadata (Option<HashMap<String, serde_json::Value>>).

  core functions:
  - scan_skills_dir(base: &Path) -> Vec<SkillMeta> — walk dirs, parse SKILL.md frontmatter, return metadata only (no body). Skips dirs without SKILL.md. Uses serde_yaml for frontmatter.
  - read_skill_body(name: &str, base: &Path) -> Result<String> — read SKILL.md body (everything after --- closing delimiter).
  - read_skill_file(name: &str, base: &Path, relative_path: &str) -> Result<String> — read a supporting file (references/, templates/, scripts/, assets/). Validate path does not escape skill dir.
  - validate_skill_name(name: &str) -> Result<()> — regex [a-z0-9][a-z0-9._-]*, max 64 chars.
  - validate_frontmatter(content: &str) -> Result<SkillMeta> — parse YAML frontmatter, verify required fields (name, description), validate description length ≤ 1024.
  - platform_matches(platforms: &Option<Vec<String>>) -> bool — check against current OS.

  VALIDATION CONSTANTS: MAX_NAME_LENGTH=64, MAX_DESCRIPTION_LENGTH=1024, MAX_SKILL_CONTENT_CHARS=100_000, MAX_SKILL_FILE_BYTES=1_048_576, VALID_NAME_RE = ^[a-z0-9][a-z0-9._-]*$.
  ALLOWED_SUBDIRS: ["references", "templates", "scripts", "assets"].

  === MODULE 2 — Skill Manager (src/skill_manager.rs) ===

  Port of: tools/skill_manager_tool.py (all 6 actions, with security guard)

  SkillManager trait with 6 actions, all returning Result<SkillActionResponse, SkillManagerError>:
  - create(name, category: Option<String>, content: String) — validate name+frontmatter, create dir (skills/{category?}/{name}/), atomic write SKILL.md, create empty references/templates/scripts/assets/ dirs. Error if skill already exists.
  - edit(name, content: String) — find existing skill, validate frontmatter, atomic replace SKILL.md. Error if skill not found.
  - patch(name, file: Option<String>, old_text: String, new_text: String, replace_all: bool) — find skill, read file (defaults to SKILL.md), apply exact text replacement via .replace(). Error if old_text not found.
  - delete(name) — find skill, validate delete safety (no symlinks, inside skills root, not the root itself), check pin status, rmtree. Error if pinned or unsafe.
  - write_file(name, relative_path: String, content: String) — validate path in ALLOWED_SUBDIRS, write file (atomic replace), create parent dirs as needed. Error if path escapes or not in allowed subdirs.
  - remove_file(name, relative_path: String) — validate path in ALLOWED_SUBDIRS, remove file, clean up empty parent dirs. Error if file doesn't exist.

  SkillActionResponse: { success: bool, skill_name: String, action: String, path: Option<PathBuf>, message: String }.
  SkillManagerError enum: NotFound, AlreadyExists, InvalidName, InvalidFrontmatter, FrontmatterParseError, ContentTooLarge, FileTooLarge, PathTraversal, InvalidSubdir, Pinned, DeleteUnsafe, IoError.

  Atomic writes: write to temp file in target dir, then fs::rename (atomic on Unix). This prevents partial writes from corrupting skills.

  Pin guard: skills can be pinned (protected from deletion). Agent can still edit/patch pinned skills; only deletion is blocked. Pin state stored in a sidecar JSON file (skill_usage.json) per skill dir.

  === MODULE 3 — Progressive Disclosure Tools (src/skills_tool.rs) ===

  Port of: tools/skills_tool.py (skills_list, skill_view with tiered loading)

  SkillsTool trait:
  - skills_list(base: &Path) -> SkillsListResult — scan all skills, return compact index: Vec<SkillMeta> (name, description only — never the body). Token-efficient for system prompt inclusion.
  - skill_view(name: &str, base: &Path, file: Option<&str>) -> SkillViewResult — if file is None, return full SKILL.md body. If file is Some(path), return that supporting file's content. Validate path does not escape. Error if skill not found.
  - check_skill_requirements(name, base) -> SkillRequirements — check platforms, prerequisites against current environment.

  SkillsListResult: { skills: Vec<SkillMeta>, count: usize }.
  SkillViewResult: { skill_name: String, file: Option<String>, content: String }.
  SkillRequirements: { compatible: bool, missing_env_vars: Vec<String>, unsupported_platform: bool }.

  === MODULE 4 — Security Guard (src/skill_guard.rs) [OPTIONAL, config-gated] ===

  Port of: tools/skills_guard.py (scan_skill, should_allow_install)

  Off by default (config: skills.guard_agent_created = false). When enabled, scans agent-created skills for dangerous patterns before write.

  SkillGuard trait:
  - scan_skill(skill_dir: &Path, source: &str) -> ScanResult — walk skill files, check against dangerous patterns.
  - should_allow(result: &ScanResult) -> (Option<bool>, String) — allow (true), block (false), ask (None) verdicts.
  - format_scan_report(result: &ScanResult) -> String.

  Dangerous pattern checks (from hermes-agent source):
  - Shell execution patterns: "rm -rf", "curl | sh", "sudo", "eval", "exec("
  - Network exfiltration: "curl.*https?://", "wget.*https?://", "nc -"
  - File system escape: "..", "/etc/passwd", "/etc/shadow"
  - Prompt injection in skill content: "ignore previous instructions", "system prompt:", "<system>"

  ScanResult: { findings: Vec<Finding>, file_count: usize, source: String }.
  Finding: { file: String, line: usize, pattern: String, severity: Severity (Low/Medium/High/Critical) }.

  Integration: skill_manager.create() and edit() call scan_skill() after write. On block verdict, delete the skill and return error with scan report. On ask verdict (dangerous patterns found but not blocking), also delete and return error (agent-created = block on any dangerous finding). Module is gated behind a feature flag `skill-guard`.

  === MODULE 5 — AgentRuntime Integration (voltron-runtime) ===

  Port of: run_agent.py skill registration + toolsets.py skill toolset

  Add optional HermesEngine to AgentRuntime builder:
  - skills_dir: PathBuf (default ~/.voltron/skills/)
  - guard_enabled: bool (default false)

  When wired, HermesEngine registers 3 tools with the runtime:
  - skill_manage (SkillManager trait) — creates/edits/patches/deletes skills
  - skills_list (SkillsTool trait) — compact skill index discovery
  - skill_view (SkillsTool trait) — load full skill or supporting file

  CLI flag: --skills-dir <PATH> overrides default skills directory.

  Skill index inclusion: on each turn, the compact skills_list output (Vec<SkillMeta> with name+description only) is appended to the system prompt as a "Available Skills" section. This is ~100 tokens for 10 skills. The agent uses skill_view to load the full instructions on demand.

  === MODULE 6 — Tests (mod tests in adapter crate) ===

  Unit tests (targeting filesystem operations with tempdir):
  - scan_skills_dir: dir with 3 skills → returns 3 SkillMeta entries
  - scan_skills_dir: empty dir → returns 0 entries
  - scan_skills_dir: dir with no SKILL.md → skipped
  - validate_skill_name: valid names pass, invalid (uppercase, spaces, special chars) fail
  - validate_frontmatter: missing name → error, missing description → error, description > 1024 → error
  - validate_frontmatter: valid frontmatter → Ok(SkillMeta)
  - skill_manager.create: valid input → creates dir + SKILL.md + subdirs, frontmatter parses
  - skill_manager.create: duplicate name → AlreadyExists error
  - skill_manager.create: invalid name → InvalidName error
  - skill_manager.create: content exceeding 100k chars → ContentTooLarge error
  - skill_manager.edit: existing skill → SKILL.md replaced, atomically
  - skill_manager.edit: nonexistent skill → NotFound error
  - skill_manager.patch: old_text found → replaced, success
  - skill_manager.patch: old_text not found → error
  - skill_manager.patch(replace_all=true): multiple occurrences → all replaced
  - skill_manager.delete: existing skill → dir removed
  - skill_manager.delete: pinned skill → Pinned error
  - skill_manager.delete: path traversal attempt → PathTraversal error
  - skill_manager.write_file: valid subdir → file created
  - skill_manager.write_file: invalid subdir → InvalidSubdir error
  - skill_manager.remove_file: existing file → file removed
  - skills_list: multiple skills → returns compact index (no body content)
  - skill_view: existing skill → returns full SKILL.md body
  - skill_view with file: existing reference → returns file content
  - skill_view with file: path traversal attempt → error

  Integration test (voltron-runtime tests):
  - HermesEngine wired → skill_manage, skills_list, skill_view tools registered in runtime
  - skill_manage(create) via tool call → skill created on disk
  - skills_list via tool call → returns created skill in index
  - skill_view via tool call → returns full skill body

  DEPENDENCIES (workspace): serde, serde_json, serde_yaml, thiserror, tempfile (dev), regex.

  COMMIT: `feat(hermes): implement self-improving skill loop (hermes-agent port)`

## [DEZIRAY_AUDITOR]
CURRENT_AUDIT=COMPLETE: Audit of crates/voltron-memory-tree (7 modules) and crates/voltron-ractor (4 modules + tests). 26 tests pass. 0 CRITICAL, 1 HIGH, 3 LOW findings.
PHASE_1_AUDIT=COMPLETE — all 7 crates audited, 5 remediation items verified passing, 57+ tests confirmed
PHASE_2_AUDIT=COMPLETE — crates/voltron-moltis-adapter/ audited, 18/18 tests, PASS verdict
PHASE_3_AUDIT=COMPLETE — crates/voltron-ironclaw-adapter/ audited, 85/85 tests, PASS verdict
PHASE_4_AUDIT=COMPLETE — crates/voltron-hermes-adapter/ audited, 6 modules verified, PASS verdict
STATUS=COMPLETE — Phase 5 audit delivered, verdict CONDITIONAL_PASS
FINDINGS=
  [AUDIT-9.1:PHASE5_MEMORY_TREE] — crates/voltron-memory-tree audit (7 modules, 13 tests):
    (M1) types.rs — 4 constants (INPUT_TOKEN_BUDGET=50k, OUTPUT_TOKEN_BUDGET=5k, SUMMARY_FANOUT=10, MAX_CASCADE_DEPTH=32), 6 structs, 3 enum types. TreeKind (Source/Global/Topic) with LabelStrategy mapping correct. DEFAULT_FLUSH_AGE_SECS=604800 (7 days). PASS.
    (M2) store.rs — TreeStore trait with 8 async methods. InMemoryTreeStore uses HashMap for O(1) lookups. Buffer dedup via retain+push. No persistence = dev only. PASS.
    (M3) summarize.rs — Summarizer trait with input_tokens hint. ConcatSummarizer (testing) and TruncatingSummarizer (lossy sim) built-in. Trait is Send+Sync for async use. PASS.
    (M4) seal.rs — Bucket-seal cascade engine. append_leaf: active-guard, store leaf, L0 token-gated check (≥INPUT_TOKEN_BUDGET), cascade_seal recursive via Box::pin, MAX_CASCADE_DEPTH=32 safety cap. Label resolution: ExtractFromContent (keyword extraction), UnionFromChildren (BTreeSet dedup), Empty. Root update checks last summary level > current root level. PASS.
    (M5) flush.rs — flush_stale_buffers iterates L0 buffers, checks oldest_timestamp < now-max_age, force-seals via sentinel leaf. Uses saturating_sub for token delta. PASS.
    (M6) tree.rs — MemoryTreeEngine public API: create_tree, ingest, flush_stale, freeze_tree, archive_tree. Freeze/archive gates reject new input. Tree persisted after each ingest. PASS.
    (M7) lib.rs — Re-exports all public types, 7 public modules. PASS.
  [AUDIT-9.2:PHASE5_RACTOR] — crates/voltron-ractor audit (4 modules, 13 tests):
    (M1) lib.rs — AgentTask enum (ProcessMessage/Reload/Shutdown) with oneshot reply channels. AgentResponse, AgentError (6 variants including ShuttingDown/Cancelled). VoltronError conversion via From. PASS.
    (M2) actor.rs — AgentActor implements ractor::Actor. State initialization via pre_start with AgentArgs. Message processing: record incoming, build context with system prompt + history, LLM call, record response, enforce max_history. Reload from memory, graceful shutdown. ractor 0.14 API uses #[async_trait] feature. PASS.
    (M3) handle.rs — ActorAgentHandle wraps ActorRef<AgentTask>. Ergonomic process_message/reload/shutdown. is_alive() returns true unconditionally — see LOW finding. PASS.
    (M4) runtime.rs — ActorRuntime with topic-based pub/sub. register spawns agents, map topic→agent_ids. publish routes to subscribers or default agent. send_to for direct dispatch. shutdown_all iterates all agents. PASS.
  [AUDIT-9.3:HIGH→RESOLVED] — handle.rs is_alive() was a stub returning true unconditionally. Fixed: now sends a Reload ping with 100ms timeout, returns false on error/timeout. Raises is_alive to async for correctness. PASS.
  [AUDIT-9.4:LOW] — flush.rs sentinel leaf: flush_stale_buffers uses append_leaf with a sentinel containing up to INPUT_TOKEN_BUDGET tokens of placeholder text. This inflates token estimates in downstream summaries. Direct seal_level call would be cleaner for time-based flushes where content already exists.
  [AUDIT-9.5:LOW] — seal.rs keyword entity extraction: extract_keyword_entities uses simple capitalisation heuristics + domain-specific substring matching. Advertised as "placeholder for full entity extraction" in comments — acceptable for v0.1 but should be replaced with LLM-based or NER-based extraction for production.
  [AUDIT-9.6:LOW] — actor.rs LLM error path records incoming message to memory but does not record an error response. On LLM failure, conversation history may desync (message recorded as received but never answered). Acceptable for dev; consider recording a synthetic error message for production.
  Verdict: CONDITIONAL_PASS — 0 CRITICAL, 0 HIGH (resolved), 3 LOW. All 26 tests pass, zero build errors. Core algorithms (bucket-seal cascade, ractor topic dispatch) are structurally sound.
FINDINGS=
  [AUDIT-7.1:PASS] IronClaw adapter crate audit complete. Six spec modules verified:
    (1) CapabilityManifest — all fields present, canonical_bytes() via ciborium, verify_hash() method. PASS.
    (2) SignedManifest — ed25519-dalek Ed25519 verification, serde_bytes serialization, proper error propagation. PASS.
    (3) ManifestVerifier trait — verify_manifest + verify_skill_by_name, VerificationError enum with all 5 variants. PASS.
    (4) IronclawManifestVerifier — all 5 gates (revocation, sig, expiry, hash, permission) implemented in correct order. PASS.
    (5) Integration — --ironclaw-manifest-dir flag, load_ironclaw_manifests() function, AgentRuntime builder wiring, process_message gate. PASS.
    (6) Tests — 9 unit tests covering valid sig, wrong sig, expiry, hash mismatch, missing manifest, revocation, permission violation, unrevoke. PASS.
    Phase 2 findings F1 (model_override) fixed in main.rs. F2 (uuid_v4) fixed — uses uuid crate. F3 (iso_now) fixed — uses chrono. F5 (tool exhaustion) mitigated with fallback message. F4 (run_loop integration test) remains open — low priority.
    Verdict: IRONCLAW_APPROVED. 85/85 tests pass.
  [AUDIT-8.1:PASS] Hermes adapter crate audit complete. Six spec modules verified:
    (1) Skill Storage — SkillsDir, SkillMeta, scan_skills_dir, read_skill_body, read_skill_file, validate_skill_name, validate_frontmatter, platform_matches. All spec constants present. Path traversal via canonicalization. 14 tests. PASS.
    (2) Skill Manager — SkillManager trait with 6 actions, DiskSkillManager with atomic writes, pin protection, delete safety. 17 tests. PASS.
    (3) Skills Tool — progressive disclosure: compact index (name+description), full body on demand, requirements check. 7 tests. PASS.
    (4) Security Guard — feature-gated, shell/network/fs/injection pattern scan, should_allow verdicts. 7 tests. PASS.
    (5) HermesEngine — wraps SkillManager+SkillsTool, config, create_skill with guard integration, format_skill_index. 9 tests. PASS.
    (6) Linkage — all types re-exported, feature-gated. 4 tests. PASS.
    Verdict: HERMES_ADAPTER_APPROVED. All 6 modules per spec.

## [ACK_MATRIX]
ACK_IP_MAN=FALSE
ACK_DEZIRAY=FALSE
NEEDS_ALFRED=FALSE