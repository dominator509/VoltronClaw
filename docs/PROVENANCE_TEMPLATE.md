# PROVENANCE Template

> This template must be completed for every component placed in `/third_party/<name>/`.

---

# PROVENANCE — <Component Name>

## Source

- **Original repository:** <URL>
- **Commit hash:** <SHA>
- **Retrieval date:** <ISO-8601>
- **Retrieved by:** <name or agent>

## License

- **Declared license:** <SPDX identifier>
- **LICENSE file present in source:** Yes / No
- **License file at retrieval commit:** <path in source repo>
- **License copy in our repo:** /third_party/<component>/LICENSE

## Intake Audit

- [ ] License on whitelist (LICENSE_STRATEGY.md §2)
- [ ] No copyleft in transitive deps
- [ ] Copyright notices preserved
- [ ] THIRD_PARTY_LICENSES.md entry created
- [ ] IP_POSTURE_MATRIX.md updated
- [ ] cargo-deny check clean
- [ ] Adapter crate created behind voltron-core trait

## Modifications

- <List all changes made to the borrowed code, with rationale>

## Attribution

- Original copyright notice preserved in: <file paths>
- Attribution included in: <NOTICE / README / distribution>

## Patent Caveats

- Patent grant applies: Yes (Apache-2.0) / No (MIT/BSD/etc)
- Known patents covering this code: <none / list>

## Sign-off

- Auditor: <name>
- Date: <ISO-8601>
