# SPEC_ANCHOR

> Version: 0.1.0  
> Scope: Phases 0–2  
> Status: DRAFT — awaiting Phase 1 trait definitions

## Project Mission

Voltron Claw is a greenfield Rust-native composite agent that combines proven
algorithms from existing open-source claws with novel security and architecture
layers no current project ships.

## Phase 0 — Foundation

- Workspace scaffold with 7 crates
- License enforcement (cargo-deny)
- CI/CD pipeline (github-actions)
- Documentation framework

## Phase 1 — Core Traits + Minimum Viable Loop

Five core traits locked in `crates/voltron-core/src/traits.rs`:

- `LLMProvider` — async generate(messages, tools) -> Response
- `MemoryStore` — async put, get, search, delete with MemoryRecord
- `SkillExecutor` — async execute(skill_id, args) -> SkillResult with SkillManifest
- `ChannelAdapter` — async recv() -> Stream<Message>, async send(Message)
- `AuditSink` — sync append(AuditEntry)

All return `Result<T, VoltronError>` with `VoltronError` via `thiserror`.

Traits are `#[async_trait]` with `Send + Sync` bounds.

Phase 1 also delivers working impls for each trait and a minimal agent loop
in `voltron-runtime`.

## Phase 2 — First Borrowed Component Intake

Operator selects a component from §3.4 of the handoff prompt. Full license
audit (§3 of LICENSE_STRATEGY.md), fenced placement in `/third_party/`,
adapter crate behind `voltron-core` traits.

## Out of Scope (this prompt)

- Trinity-anchored congruence layer
- Encrypted memory composite (SQLCipher + AES-256-GCM + Argon2id)
- Two-role internal split
- Tokenization layer (PII/PHI redaction)
- Append-only HMAC-chained audit log
- TEE deployment target
- Additional borrowed components beyond Phase 2

## Spec Lock

No Rust code shall be written until the implementing trait or module
is specified in this document and marked `SPEC_ANCHOR_REVIEWED.flag`
in the `/docs/` directory.
