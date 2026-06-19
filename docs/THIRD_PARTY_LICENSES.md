# Third-Party Licenses

> Live registry of every borrowed open-source component.  
> Append-only. One entry per component.  
> Schema per LICENSE_STRATEGY.md §11.

---

## Registry

### Moltis

- **Source repository:** https://github.com/moltis-org/moltis
- **Commit hash borrowed from:** `48c9a41926d095173654030a4a87baf236792b19`
- **Retrieval date:** 2026-06-19
- **License:** MIT
- **License file path in our repo:** /third_party/moltis/LICENSE
- **Files containing borrowed code:** TBD — adapter crate in /crates/voltron-moltis-adapter/
- **Borrowed scope:** full module (agent runtime)
- **Modifications made:** None — pinned commit, no local changes
- **Attribution location:** NOTICE file at distribution root
- **Patent grant (if Apache-2.0):** N/A (MIT)
- **Audit PR:** awaiting merge
- **Auditor:** Deziray (license audit), Ip Man (placement)

---

## Schema (for future entries)

Each entry must contain:

- **Source repository:** URL
- **Commit hash borrowed from:** SHA
- **Retrieval date:** ISO-8601
- **License:** SPDX identifier
- **License file path in our repo:** /third_party/<component>/LICENSE
- **Files containing borrowed code:** list
- **Borrowed scope:** "full module" | "algorithm only" | "API surface" | etc.
- **Modifications made:** summary
- **Attribution location:** where copyright notice lives in distribution
- **Patent grant (if Apache-2.0):** Yes / N/A
- **Audit PR:** #<number>
- **Auditor:** name
