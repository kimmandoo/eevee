# F — Context, token economy and tool calling

Local deterministic optimizer only: no optimizer-initiated network, hosted summarization, embeddings, telemetry or tokenizer downloads. Keep exact Eevee-sent bytes, labelled estimates, provider-reported usage and billing distinct; unknown stays unknown, never zero. Tool discovery, paging and caching apply only to documented adapter capabilities and Eevee-owned execution. T05 was dropped on 2026-10-14.

### C03A — Attachment chips

- [x] Show and remove `@` attachments as chips.
- **Priority:** 1. **Needs:** —. **Builds on:** C02, A02.
- **Change:** Compact removable file/range chips in the composer, keyboard-navigable, restored after a restart.
- **Accept:** Attach a file and a range, remove one by keyboard, restart, and recover the rest. Long Unicode names keep remove/inspect visible.
- **Watch out:** Excluded secrets and out-of-root links never become chips.

### C03B — Chip preview, staleness and budget

- [x] Inspect exactly what each chip sends.
- **Priority:** 1. **Needs:** C03A. **Builds on:** C02.
- **Change:** Per-chip content preview, stale/unavailable markers, exact bytes and a labelled token estimate; reuse dispatch revalidation.
- **Accept:** A source change goes stale before send; a missing capture blocks assembly; displayed bytes equal the serialized request.
- **Watch out:** Never imply control over provider-added reads/history.

### T01A — Provider usage reconciliation

- [x] Record provider-reported usage per turn without double counting.
- **Priority:** 1. **Needs:** C03B. **Builds on:** A02, R02.
- **Change:** Store input/output/cache/reasoning usage from supported events; delta, final and repeated events count once. Name the model and tokenizer.
- **Accept:** Two real turns and a resume reconcile against provider events.
- **Status:** Verified three explicitly authorized read-only Codex 0.147.0 turns, including two actual owned-session resumes, on macOS arm64 on 2026-10-21. Persisted response counters matched reported events and resumed session deltas without overlapping sums. Unreported model/tokenizer metadata remained unknown; billing stayed provider-owned. SDK revision-5 contract/policy evidence is separate from live SDK conformance.
- **Watch out:** Unavailable prices produce no invented cost; opaque provider input is not attributed to selected files.

### T01B — Before/after selection comparison

- [ ] Compare two context selections for the same turn.
- **Priority:** 2. **Needs:** T01A. **Builds on:** —.
- **Change:** Show the receipt delta between the current and a previous selection.
- **Accept:** Removing a chip shows the exact byte and estimate drop per section.

### T01C — Durable receipt identity

- [ ] Keep receipts across restarts and respect purge.
- **Priority:** 2. **Needs:** T01A. **Builds on:** E03.
- **Change:** Persist receipts with their request identity.
- **Accept:** A receipt survives a restart; purge removes its source bytes but keeps counts according to E03.

### T02A — Ranked context with reasons under a budget

- [ ] Assemble a smaller packet with visible reasons and omissions.
- **Priority:** 1. **Needs:** T01A. **Builds on:** C02.
- **Change:** From goal, diff and pins, rank candidate excerpts (text-only, labelled), dedup overlaps, and show reasons and omissions under a user budget. Overflow requires a decision.
- **Accept:** In a multi-file bug fixture, the defect, caller and check are included with fewer tokens than the full-file baseline (one named tokenizer). Editing a source after preview stops dispatch.
- **Watch out:** Never drop mandatory instructions or pins silently.

### T02B — Symbol-aware excerpts

- [ ] Prefer definitions, signatures and direct references.
- **Priority:** 2. **Needs:** T02A. **Builds on:** —.
- **Change:** Use available local language tooling for symbols; unsupported syntax falls back to labelled text ranges.
- **Accept:** A seeded cross-file requirement stays discoverable; unsupported files are labelled, never given fabricated symbols.

### T03A — Cross-entry-point dedup

- [ ] Send a capture once even when it arrives through several entry points.
- **Priority:** 2. **Needs:** T02A. **Builds on:** C02.
- **Change:** Extend the shipped in-packet dedup (`same_bytes_as`, hoisted `source_root`) across attachments, memory, handoff facts and tool-result references.
- **Accept:** The same capture through three entry points yields fewer tokens and every citation resolves; identical text from distinct sources is not conflated.

### T03B — Local revision and tokenization cache

- [ ] Cache assembly and tokenization by content and configuration identity.
- **Priority:** 2. **Needs:** T03A. **Builds on:** —.
- **Change:** A persisted revision index; a changed source, policy or tokenizer invalidates entries.
- **Accept:** Invalidation works for each cause with zero network access.
- **Watch out:** A local cache hit never implies provider retention or a billing discount.

### T04 — Evidence-linked compaction

- [ ] Continue long work with a compact local ledger.
- **Priority:** 2. **Needs:** T01A. **Builds on:** C02, E02, R02.
- **Change:** A deterministic packet of requirements, approved decisions, references, unresolved hypotheses and next action, with inferences kept separate from observations; preview before any request.
- **Accept:** An offline resume with conflicting decisions, a failed check and a changed source keeps the latest decision and the failure; cancel changes nothing; processing cost and packet size are measured.
- **Watch out:** No model summarization; an old approval is never a new grant.

### TC01A — Local tool catalog search

- [ ] Search stored metadata of explicitly trusted tools.
- **Priority:** 1. **Needs:** —. **Builds on:** A01, C02.
- **Change:** A local catalog (populated by a separate reviewed setup action) with namespaced identity, trust origin and declared effects, and local ranking.
- **Accept:** In a 100-tool fixture, the intended tool ranks first; collisions and malicious descriptions are handled without executing anything.
- **Watch out:** Relevance is not authorization; a read-only hint is not a sandbox.

### TC01B — Lazy schema exposure to adapters

- [ ] Send only the selected tool schemas where the adapter supports it.
- **Priority:** 1. **Needs:** TC01A. **Builds on:** A02.
- **Change:** Load exact versioned schemas for supported adapters; unsupported providers report that lazy exposure is unavailable.
- **Accept:** Fewer schema tokens than the full catalog, with one real compatible round trip. A changed schema or disconnected server requires setup.
- **Watch out:** No hidden schema fetch and no credential scraping.

### TC03A — Paged tool-result artifacts

- [ ] Store tool output once and serve exact pages.
- **Priority:** 1. **Needs:** TC01A. **Builds on:** E03, R02.
- **Change:** Immutable artifact identities with an exit summary, total/captured/omitted sizes, a preview and range retrieval. Inherits isolation, redaction, quota and purge.
- **Accept:** For a 10 MiB log, errors outside the preview are retrieved by page; wrong-task, purged and stale handles are refused.

### TC03B — Search within artifacts

- [ ] Find the relevant slice instead of paging blindly.
- **Priority:** 1. **Needs:** TC03A, EV10A. **Builds on:** —.
- **Change:** Exact search plus parsed failure locations (reusing EV10A).
- **Accept:** Search finds the errors with much less input than the full log; a quota-limited run reports discarded bytes; instruction-like output cannot approve calls.

### TC04 — Observation cache

- [ ] Reuse unchanged read observations honestly.
- **Priority:** 2. **Needs:** TC03A. **Builds on:** C02.
- **Change:** Cache allowlisted reads/searches keyed by operation version, args, project and fingerprints; hits are labelled with their original time.
- **Accept:** A repeat runs no worker; changing content, ignore rules, tool version or a link boundary invalidates; other projects never share; executions avoided are compared against hashing cost.
- **Watch out:** Shell, tests, builds and network reads are non-cacheable. A hit is never a fresh test.

### TC02 — Reviewed tool plans

- [ ] Execute a visible typed plan with fewer round trips.
- **Priority:** 2. **Needs:** TC01B. **Builds on:** R02, A03.
- **Change:** A bounded dependency graph with validated args and effects. Only enforceably read-only independent calls run in parallel; writes and unknown effects are serialized; every effectful call is reviewed.
- **Accept:** Independent reads overlap and writes never do; a failed prerequisite blocks its descendants; cancellation starts no queued node; counts and time are compared with serial execution.
- **Watch out:** A batch is not blanket consent.

### TC05 — Failed-step recovery

- [ ] Recover a failed plan without repeating confirmed mutations.
- **Priority:** 2. **Needs:** TC02, TC03A. **Builds on:** R02.
- **Change:** Persist node args, fingerprints, results and certainty; preview resuming failed/unstarted nodes only.
- **Accept:** A confirmed write is not repeated; a lost mutation response blocks until reconciled; stale approvals start nothing.
- **Watch out:** Never promise exactly-once execution across crashes.

### D01 — Impact-directed verification plan

- [ ] Recommend the checks most likely to catch a change's defects.
- **Priority:** 2. **Needs:** T02B, TC02. **Builds on:** C01, R01.
- **Change:** Use changed symbols, local dependencies, test mappings and prior failures (PK05A when available) to propose a minimal check set with reasons, through Jolteon's review.
- **Accept:** The caller's test catches a changed callee; a shared-config change triggers a broader plan; targeted passes are labelled targeted.
- **Watch out:** A recommendation is neither proof of completeness nor permission to run.

### Q03 — Optimization quality benchmark

- [ ] Prove the optimizations reduce measured work without weaker answers.
- **Priority:** 2. **Needs:** T01C, T02B, T03B, T04, TC01B, TC02, TC03B, TC04, TC05, D01.
- **Change:** Paired baseline/optimized journeys (bug fix, explanation, noisy log, resume) with fixed model/settings/repo, cold and warm caches separated, and all optimizer costs included.
- **Accept:** ≥3 paired trials per journey with raw samples, correctness, time, turns, tools and usage; the same functional/permission checks; regressions published.
- **Watch out:** Live runs are paid and need explicit user approval.
