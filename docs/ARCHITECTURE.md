# Architecture

> Version: 0.1.0  
> Status: Phase 0 scaffold

## Workspace Layout

```
voltron-claw/
├── Cargo.toml                       # workspace root
├── rust-toolchain.toml              # pin stable >= 1.78
├── LICENSE                          # Apache-2.0
├── NOTICE                           # attributions
├── README.md
├── deny.toml                        # cargo-deny config
├── .github/workflows/ci.yml
├── docs/                            # all project documentation
├── logs/                            # build logs per phase
├── src/                             # binary entry point
│   └── main.rs
├── crates/                          # original code
│   ├── voltron-core/                # trait definitions + error types
│   ├── voltron-providers/           # LLMProvider impls
│   ├── voltron-memory/              # MemoryStore impls
│   ├── voltron-skills/              # SkillExecutor + skills
│   ├── voltron-channels/            # ChannelAdapter impls
│   ├── voltron-audit/               # AuditSink impls
│   └── voltron-runtime/             # agent loop
└── third_party/                     # borrowed code (Phase 2+)
```

## Crate Dependency Graph

```
voltron-runtime
  ├── voltron-core
  ├── voltron-providers → voltron-core
  ├── voltron-memory → voltron-core
  ├── voltron-skills → voltron-core
  ├── voltron-channels → voltron-core
  └── voltron-audit → voltron-core

src/main.rs → voltron-runtime
```

All crates depend on `voltron-core` for traits and types.
`voltron-runtime` depends on all five implementation crates.

## Core Traits

Defined in `voltron-core/src/traits.rs`:

- **LLMProvider** — async `generate(messages, tools) -> Result<Response, VoltronError>`
- **MemoryStore** — async `put`, `get`, `search`, `delete` with `MemoryRecord`
- **SkillExecutor** — async `execute(skill_id, args) -> Result<SkillResult, VoltronError>`
- **ChannelAdapter** — async `recv() -> Stream<Message>`, async `send(Message)`
- **AuditSink** — sync `append(AuditEntry)`

All traits use `#[async_trait]`. Error type: `VoltronError` (enum via `thiserror`).

## Dependency Policy

Permissive-only (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, MPL-2.0, Unlicense, CC0).
Enforced by `deny.toml`. See `docs/LICENSE_STRATEGY.md` for full policy.

## Third-Party Code

Borrowed components live in `/third_party/<name>/` with their own LICENSE and
PROVENANCE.md. Adapter crates wrap them behind `voltron-core` traits.
Never mixed into original source directories.
