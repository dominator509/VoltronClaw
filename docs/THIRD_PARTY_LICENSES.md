# Third-Party Licenses

> Live registry of every borrowed open-source component.  
> Append-only. One entry per component.  
> Schema per LICENSE_STRATEGY.md §11.

---

## Registry

*No third-party components have been ingested as of Phase 0.*

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
