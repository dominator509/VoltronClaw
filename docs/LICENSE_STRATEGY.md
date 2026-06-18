---
title: LICENSE_STRATEGY.md
version: 1.0.0
status: DRAFT — awaiting human + IP-counsel review
scope: Project-agnostic license, patent, trade-secret, and trademark posture for a Rust-based composite agent ("Voltron Claw" or any best-of-breed derivative project)
---

# LICENSE_STRATEGY.md

> ⚠️ **NOT LEGAL ADVICE.** This document is an engineering-grade strategy framework, not a substitute for an IP attorney. Before filing any patent, asserting any IP right, or shipping a commercial product derived from third-party open-source code, obtain written counsel from a licensed IP attorney in your jurisdiction. The cost of a single consultation is trivial relative to the cost of getting this wrong.

---

## 0. Purpose

This document defines the **license, patent, trade-secret, and trademark posture** for the project. It is the single source of truth for:

- Which third-party open-source components may be incorporated, and under what conditions
- What the project may and may not do with respect to copyright, patent, trademark, and trade secrets
- The audit, attribution, and record-keeping obligations every contributor must meet
- The defensive vs. offensive IP stance the project will take
- The workflow gates that must pass before any third-party code is merged

Every contributor — human or agentic — MUST read and comply with this document. Violations are project-blocking events.

---

## 1. Guiding Principles

1. **Compose, do not stitch.** Borrowed components live in clearly-bounded modules with documented provenance. Never copy fragments without attribution.
2. **Permissive-only intake.** Only MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, MPL-2.0 (file-level only), and Unlicense / CC0 sources are eligible. Anything copyleft (GPL, LGPL, AGPL) is prohibited from intake without written exception.
3. **Patent the novel, not the borrowed.** Borrowed inventions are prior art. Only genuinely novel additions invented by the project are candidates for patent filing — and only after counsel review.
4. **Apache §3 is a live tripwire.** The project will not initiate patent litigation against any party for code that overlaps with Apache-2.0 ancestry without explicit counsel sign-off, because doing so terminates all Apache-2.0 grants automatically.
5. **Defensive posture by default.** Patents, if filed, are filed for defensive use, cross-licensing, and patent-pool participation — not for offensive assertion.
6. **Trade secret + trademark + execution are the moat.** Patents are a secondary layer. The primary moats are proprietary code that is never published, a strong trademark on the product name, and execution velocity.
7. **Auditability beats cleverness.** Every borrowed line, every license decision, every patent question gets logged. Future-us will thank present-us.

---

## 2. License Intake Whitelist & Blacklist

### ✅ Whitelisted (intake permitted under standard process)

| License | Notes |
|---|---|
| MIT | Standard. Preserve copyright notice and license text in any file containing borrowed code. |
| Apache-2.0 | Preferred over MIT when both are available because of the explicit patent grant. Subject to §3 defensive-termination awareness. |
| BSD-2-Clause / BSD-3-Clause | Same posture as MIT. BSD-3-Clause "no endorsement" clause is fine — we don't claim endorsement anyway. |
| ISC | Functionally MIT-equivalent. |
| Zlib | Acceptable; minimal obligations. |
| MPL-2.0 | **File-level copyleft only.** Modified MPL files remain MPL and must be redistributable in source form. Acceptable for utility modules but not for crown-jewel code. |
| Unlicense / CC0 / Public Domain | Acceptable, but verify provenance — many "public domain" claims are legally questionable outside the US. |

### ❌ Blacklisted (intake forbidden without written exception)

| License | Reason |
|---|---|
| GPL-2.0 / GPL-3.0 | Viral copyleft — would force the entire derived binary to GPL. |
| LGPL-2.1 / LGPL-3.0 | Dynamic-linking exception is workable but creates compliance burden and limits distribution flexibility. |
| AGPL-3.0 | **Network-trigger copyleft.** Even running it on a server triggers source-disclosure obligations. Catastrophic for SaaS or hosted agent deployments. |
| SSPL (MongoDB-style) | Server-side viral, even for managed services. |
| BUSL (Business Source License) | Time-bombed; converts later but restrictive in the interim. Read the specific Use Limitation. |
| Commons Clause additions | Strips commercial use rights from otherwise-permissive licenses. |
| No LICENSE file present | Default copyright applies = no usage rights. Treat as forbidden until a license is confirmed in writing from the author. |
| Custom / "source-available" / "fair use" / "ethical source" | Require individual counsel review. Default-deny. |

### ⚠️ Conditional / Counsel Required

- **Dual-licensed projects** (e.g., "MIT or GPL"): we always take the permissive option, but document the choice explicitly.
- **CLA-required projects** where we want to contribute back: we will sign CLAs only after counsel review.
- **License changes mid-project** (e.g., Elastic → SSPL, Redis → SSPL): we pin to the last permissive-licensed commit hash and document the fork point.

---

## 3. Pre-Intake Audit Checklist

Before any third-party code is merged into the project, the merging contributor MUST complete and attach the following checklist to the pull request:

- [ ] LICENSE file present in the source repo at the commit hash being borrowed from
- [ ] License is on the §2 whitelist (or written exception is attached)
- [ ] Commit hash + repo URL + retrieval date logged in `THIRD_PARTY_LICENSES.md`
- [ ] Original copyright notice preserved in every file containing borrowed code
- [ ] Original LICENSE text included in `/third_party/<component>/LICENSE`
- [ ] `cargo-deny check licenses` passes with no new findings
- [ ] `cargo-deny check advisories` passes with no new findings
- [ ] `cargo-deny check bans` passes with no new findings
- [ ] Transitive dependencies audited (run `cargo tree -e features` and verify each new transitive license against the whitelist)
- [ ] No GPL/AGPL/SSPL/BUSL/Commons-Clause/no-license code in the transitive graph
- [ ] Patent-grant clause noted if source is Apache-2.0 (see §6)
- [ ] If the borrowed code embodies an algorithm or method, note any known patents covering it in `PRIOR_ART_REGISTRY.md`
- [ ] Contributor signs the audit attestation in the PR description

PRs that lack a completed checklist MUST NOT be merged.

---

## 4. What the Project May Do (Copyright)

Under MIT/Apache-2.0 intake, the project MAY:

- ✅ Copy, modify, and integrate the code into the project
- ✅ Combine it with proprietary code we wrote ourselves
- ✅ Relicense the *combined work* under any license of our choosing, including proprietary, **as long as the borrowed files retain their original notices and the original license text travels with them**
- ✅ Sell, license, host, or SaaS-deploy the resulting product commercially
- ✅ Keep our novel additions closed-source and proprietary
- ✅ Build a competing commercial product that displaces the original
- ✅ Fork and continue development if the upstream is abandoned

---

## 5. What the Project May NOT Do (Copyright + Estoppel)

The project MUST NOT:

- ❌ Strip, alter, or obscure copyright notices in borrowed files
- ❌ Remove or modify the borrowed code's LICENSE text
- ❌ Claim the original authors endorse, sponsor, or are affiliated with our product
- ❌ Use the original project's trademark or logo in our branding without separate trademark permission
- ❌ Misrepresent the provenance of borrowed code as original work
- ❌ Sue an upstream author for using the code they originally published (implied-license / estoppel doctrines)
- ❌ Re-publish borrowed code under a more restrictive license than the original while implying it is the original work

---

## 6. Apache-2.0 §3 — Defensive Termination Tripwire

If any Apache-2.0 code is incorporated into the project (likely — Hermes, NemoClaw, and many Rust crates are Apache-2.0), the project becomes subject to **Apache License §3 defensive termination**:

> If the project (or any successor) institutes patent litigation against any entity alleging that any Apache-2.0-licensed Work or Contribution constitutes direct or contributory patent infringement, **all Apache-2.0 patent licenses granted to the project terminate as of the date the litigation is filed.**

**Operational consequences:**

1. The project will not file or threaten patent litigation against any party for patents that read on any Apache-2.0 code in our dependency graph, **without prior written sign-off from IP counsel**.
2. Before any patent assertion, counsel must produce an "Apache-2.0 exposure analysis" showing which dependencies would be affected and what the fallback (re-implement, replace, accept termination) would be.
3. This effectively means the project's Apache-2.0 components are **a defensive shield** — they discourage offensive patent assertion by raising the cost.
4. This is a feature, not a bug, of our license posture. It aligns with the §1.5 defensive-by-default principle.

---

## 7. Patent Strategy

### 7.1 What Cannot Be Patented

The following are **prior art** and cannot be patented by this project:

- Any algorithm, method, or architecture embodied in a published open-source repository we borrowed from, as of its publication date
- Any technique disclosed in a published paper, conference talk, blog post, or public specification
- Anything documented in any of our own public commits or releases (filing more than 12 months after public disclosure forfeits US patent rights; most jurisdictions are stricter — "absolute novelty")

Filing a patent on prior art is **invalid on its face** and exposes the project to invalidation, sanctions, and reputational damage.

### 7.2 What May Be Patentable

Genuinely novel additions invented by the project — meaning **not previously disclosed anywhere** — may be patentable if they meet all four tests:

- **Novelty** — no prior public disclosure
- **Non-obviousness** — a skilled engineer would not trivially combine known techniques to reach this
- **Utility** — concrete, demonstrable, beneficial function
- **Patent-eligible subject matter** — not an "abstract idea" under *Alice Corp v. CLS Bank* (US) or its equivalents elsewhere. Most pure-software inventions face high §101 hurdles in the US; tying the invention to concrete technical improvements (latency, memory, security guarantees, hardware integration) improves eligibility.

Candidate categories worth a counsel conversation (project-specific examples; substitute your own):

- Novel composite architectures combining encryption + key hierarchy + tokenization in a single agent-memory substrate, *if* the specific composition is not in prior art
- Novel multi-role internal-state-management mechanisms that demonstrably prevent a specific class of failure (drift, collusion, hallucination), *if* the mechanism itself is non-obvious
- Novel attestation/verification chains specific to agentic skill execution
- Hardware-tied innovations (TEE integration patterns, HSM-bound key hierarchies) where the hardware coupling strengthens §101 eligibility

### 7.3 Patent Workflow Gates

Before any provisional or non-provisional patent application is filed:

- [ ] Invention disclosed in writing to the project's IP lead (template: `INVENTION_DISCLOSURE.md`)
- [ ] Prior-art search performed (Google Patents, USPTO PatFT, EPO Espacenet, Lens.org, GitHub code search, arXiv) and findings logged
- [ ] Internal novelty review: at least one engineer independent of the inventor confirms they cannot find prior art
- [ ] Public-disclosure timeline audit (have we already publicly disclosed this? If yes, US 1-year clock starts — most other jurisdictions are already forfeited)
- [ ] Apache-2.0 §3 exposure analysis (which dependencies' patent grants would be affected if we ever asserted this)
- [ ] IP counsel engaged and confirms patentability worth pursuing
- [ ] Defensive vs. offensive intent documented and approved by project leadership
- [ ] Decision logged in `PATENT_REGISTRY.md`

### 7.4 Defensive Posture

The project's default posture is **defensive**:

- File patents only on inventions where a defensive holding produces clear value (cross-licensing leverage, deterrence against trolls, patent-pool eligibility)
- Consider joining the **Open Invention Network (OIN)** patent non-aggression community
- Consider committing eligible patents to the **Patent Commons** or making non-assertion pledges (Tesla-style, Red Hat-style)
- Document non-assertion pledges in `PATENT_NON_ASSERTION_PLEDGE.md` if adopted

The project will not initiate offensive patent litigation absent extraordinary circumstances, board-level approval, and counsel sign-off.

---

## 8. Trade-Secret Strategy

For most software inventions in the current legal climate (post-*Alice*), **trade secrets often outperform patents.** Trade-secret protection requires:

1. The information has independent economic value from being secret
2. The holder takes **reasonable measures** to keep it secret
3. The information is not generally known or readily ascertainable

### 8.1 Reasonable Measures (mandatory)

- Source repository for crown-jewel code is private; access is logged and least-privilege
- Contributors sign confidentiality agreements before access
- Build artifacts containing trade-secret logic are not shipped in raw form (consider obfuscation, server-side execution, or hardware-bound execution where feasible)
- Internal documentation explicitly marked `// TRADE SECRET — DO NOT DISCLOSE` where relevant
- Departing contributors are reminded in writing of continuing confidentiality obligations
- Trade-secret inventory maintained in `TRADE_SECRET_REGISTRY.md` (itself confidential)

### 8.2 What Belongs as Trade Secret vs. Patent

| Characteristic | Lean Patent | Lean Trade Secret |
|---|---|---|
| Easy to reverse-engineer from the product? | ✅ Patent (because secrecy will fail) | ❌ |
| Hard or impossible to reverse-engineer? | ❌ | ✅ Trade secret (because secrecy can hold) |
| Innovation cycles fast (12-24 months)? | ❌ (patents take 2-4 years to grant) | ✅ Trade secret |
| Need investor/acquirer-visible IP? | ✅ Patent | ⚠️ Document carefully |
| Subject-matter eligibility is shaky under §101? | ❌ | ✅ Trade secret |
| Want to deter copycats publicly? | ✅ Patent | ❌ |

---

## 9. Trademark Strategy

The product name and logo are likely the **most enforceable and longest-lived IP asset** the project will own.

- Reserve the product name as a trademark in primary jurisdictions (US, EU, target commercial markets) before public launch
- Conduct a clearance search before committing to a name (USPTO TESS, EUIPO eSearch, common-law searches)
- Use the ™ symbol from first use; ® only after registration grants
- Defend the trademark consistently — failure to defend can lead to genericide
- Maintain a `TRADEMARK_REGISTRY.md` listing marks, jurisdictions, registration numbers, renewal dates
- Project will not use third-party trademarks (NVIDIA, original claw names, etc.) in our branding except in nominative-fair-use contexts ("compatible with X," "imported from Y") with counsel review

---

## 10. Per-Component IP Posture Matrix (Template)

Maintain this table in `IP_POSTURE_MATRIX.md`. Update on every component intake.

| Component | Source License | Borrowed? | License Obligations Met? | Patent Risk | Trade-Secret Layer? | Patent Candidate? |
|---|---|---|---|---|---|---|
| Trait shell | <license> | Yes / No / Original | ✅ / ❌ | Low / Med / High | Y/N | Y/N |
| Sandbox layer | <license> | Yes / No / Original | ✅ / ❌ | Low / Med / High | Y/N | Y/N |
| Self-improving loop | <license> | Yes / No / Original | ✅ / ❌ | Low / Med / High | Y/N | Y/N |
| Memory architecture | <license> | Yes / No / Original | ✅ / ❌ | Low / Med / High | Y/N | Y/N |
| Skill-signing pipeline | <license> | Yes / No / Original | ✅ / ❌ | Low / Med / High | Y/N | Y/N |
| Encrypted-memory stack | Original | N/A | N/A | Low | Y | Candidate |
| Multi-role internal split | Original | N/A | N/A | Low | Y | Candidate |
| Tokenization layer | Original | N/A | N/A | Low | Y | Candidate |
| TEE deployment target | Original | N/A | N/A | Med (TEE-vendor patents) | Y | Counsel-required |
| Audit chain | Original | N/A | N/A | Low | Y | Possibly |

---

## 11. THIRD_PARTY_LICENSES.md (Template / Embedded Spec)

The project MUST maintain a `THIRD_PARTY_LICENSES.md` at the repo root with one entry per borrowed component. Minimum schema:

```
### <Component Name>

- **Source repository:** <URL>
- **Commit hash borrowed from:** <sha>
- **Retrieval date:** <ISO-8601>
- **License:** <SPDX identifier>
- **License file path in our repo:** /third_party/<component>/LICENSE
- **Files containing borrowed code:** <list>
- **Borrowed scope:** <"full module," "algorithm only," "API surface," etc.>
- **Modifications made:** <summary>
- **Attribution location:** <where the copyright notice lives in our distribution>
- **Patent grant (if Apache-2.0):** Yes / N/A
- **Audit PR:** #<pr number>
- **Auditor:** <name>
```

This file is **non-optional.** Missing or incomplete entries are a release-blocking defect.

---

## 12. CONTRIBUTING.md Companion Requirements

The project's `CONTRIBUTING.md` MUST require every contributor to:

- Agree that their contributions are licensed under the project's outbound license (typically Apache-2.0 or MIT for the OSS portions)
- Sign the project's Developer Certificate of Origin (DCO) or CLA (whichever the project adopts — document choice in `GOVERNANCE.md`)
- Disclose any third-party code they are introducing and confirm it passes §3 audit
- Disclose any patents they hold or have applied for that may read on their contribution
- Confirm they have authority to contribute (not contributing employer's code without permission)

---

## 13. Outbound Licensing Strategy

The project will publish under a **dual-track outbound license**:

- **OSS Core:** Apache-2.0 (preferred for the explicit patent grant) or MIT. Choose at project inception and document in `LICENSE`.
- **Enterprise Add-ons / Commercial Distribution:** proprietary license, optionally with an open-core split where crown-jewel features (encrypted-memory composite, multi-role substrate, attestation chains) stay proprietary.
- **SaaS / Hosted Service:** proprietary terms of service; never AGPL upstream, never SSPL upstream — both would compromise our ability to host.

Document the outbound choice in `LICENSE`, `LICENSE.commercial` (if dual-licensed), and `README.md`.

---

## 14. Red Flags & Failure Modes

| Red Flag | Why It's Bad | Mitigation |
|---|---|---|
| Copy-pasted code with no provenance log | Copyright infringement risk; impossible to audit | Reject in code review; require provenance |
| Borrowing from "no LICENSE" repo | No usage rights granted; default copyright applies | Forbidden; require licensed alternative |
| Patent application on borrowed algorithm | Invalid patent + sanctions risk + reputation damage | Pre-filing prior-art search + counsel review |
| Apache §3 trigger without exposure analysis | Catastrophic loss of dependency patent grants | Counsel sign-off required before any patent assertion |
| Trade secret published in a blog post | Loss of trade-secret protection in one event | Pre-publish review for crown-jewel docs |
| Trademark used without clearance | Infringement suit + forced rebrand | Clearance search before adoption |
| Contributor brings employer's code | Risk of employer claiming ownership | DCO/CLA + disclosure questionnaire |
| Mixed copyleft snuck in via transitive dep | Whole binary becomes copyleft | `cargo-deny check licenses` in CI |
| Out-of-date `THIRD_PARTY_LICENSES.md` | Compliance failure at release | CI check: every borrowed file must reference a registered component |

---

## 15. Required Tooling

The project's CI pipeline MUST run, on every PR and every release:

- `cargo-deny check licenses` — license whitelist enforcement
- `cargo-deny check advisories` — RustSec advisory check
- `cargo-deny check bans` — banned-crate enforcement
- `cargo-deny check sources` — source-registry enforcement
- `cargo about generate` — produces a human-readable license inventory for inclusion in releases
- License-header presence check — script verifies every source file has the project's SPDX-License-Identifier header
- `THIRD_PARTY_LICENSES.md` freshness check — fails the build if any `/third_party/` directory exists without a corresponding registry entry

Optional but recommended:

- `scancode-toolkit` periodic deep scan
- `fossology` for high-risk releases
- `licensee` for cross-validation of detected licenses

---

## 16. Governance & Review Cadence

- **Every PR introducing third-party code:** §3 audit checklist + maintainer review
- **Every quarter:** full sweep of `THIRD_PARTY_LICENSES.md` against current source repos to detect upstream license changes (rare but consequential — see Elastic, Redis, MongoDB precedents)
- **Every release:** generate and ship a `NOTICE` or `ACKNOWLEDGEMENTS` file with all required attributions
- **Annually:** IP counsel review of trademark renewals, patent portfolio (if any), trade-secret inventory, and overall posture
- **On any patent-filing decision:** mandatory counsel engagement before filing

---

## 17. Document Maintenance

- This document is the single source of truth for license, patent, trade-secret, and trademark policy
- Changes require approval from the project's IP lead and at least one other maintainer
- Material changes require IP counsel sign-off
- Version history is preserved in version control; do not edit history
- Cross-link from `README.md`, `CONTRIBUTING.md`, `GOVERNANCE.md`

---

## 18. Companion Documents (To Be Created)

- `THIRD_PARTY_LICENSES.md` — the live registry of every borrowed component
- `IP_POSTURE_MATRIX.md` — per-component posture (see §10)
- `PATENT_REGISTRY.md` — filed and pending patents, including non-assertion pledges
- `PATENT_NON_ASSERTION_PLEDGE.md` — public pledge text, if adopted
- `PRIOR_ART_REGISTRY.md` — known prior art relevant to our work
- `INVENTION_DISCLOSURE.md` — template for disclosing potential patentable inventions internally
- `TRADE_SECRET_REGISTRY.md` — confidential; lists what is held as trade secret
- `TRADEMARK_REGISTRY.md` — marks, jurisdictions, renewals
- `CONTRIBUTING.md` — contributor agreements and audit obligations
- `GOVERNANCE.md` — decision rights, including IP decisions

---

## 19. Sign-Off

This document does not take effect until:

- [ ] Reviewed by project IP lead
- [ ] Reviewed by at least one other maintainer
- [ ] Reviewed by external IP counsel (recommended before any commercial release)
- [ ] Approved and merged into the project's main branch
- [ ] Cross-linked from `README.md`, `CONTRIBUTING.md`, `GOVERNANCE.md`

Gaps, ambiguities, and open questions should be logged in `LICENSE_STRATEGY_GAPS.md` with clear closure instructions for downstream agents and counsel.

---

*End of LICENSE_STRATEGY.md v1.0.0*
