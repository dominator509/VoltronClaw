# Third-Party Components

This directory holds borrowed open-source components, each in its own
subdirectory with its original LICENSE and a PROVENANCE.md file.

## Fencing Policy

- Each component lives in its own `/third_party/<name>/` directory.
- Original LICENSE text and copyright notices are preserved.
- Borrowed code is never mixed into `/src/` or `/crates/voltron-*/`.
- Adapter crates in `/crates/voltron-<name>-adapter/` wrap the borrowed
  code behind `voltron-core` traits without leaking borrowed types.

## Intake Process

See [docs/LICENSE_STRATEGY.md](../docs/LICENSE_STRATEGY.md) §3 and the
pre-intake audit checklist.
