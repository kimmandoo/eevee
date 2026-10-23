# F — Context, token economy and tool calling

Local deterministic optimizer only: no optimizer-initiated network, hosted summarization, embeddings, telemetry or tokenizer downloads. Keep exact Eevee-sent bytes, labelled estimates, provider-reported usage and billing distinct; unknown stays unknown, never zero. Tool discovery, paging and caching apply only to documented adapter capabilities and Eevee-owned execution. T05 was dropped on 2026-10-14.

Attachment chips, exact previews, provider-usage reconciliation, same-turn comparison and the ten-task context/tool milestone are complete; specifications and scoped proof are in [COMPLETED.md](COMPLETED.md). Observation caching, reviewed plans, failed-step recovery, impact-directed checks and paired quality benchmarks remain separate.

### TC04 — Observation cache

- [ ] Reuse unchanged read observations honestly.
- **Priority:** 2. **Needs:** —. **Builds on:** C02, TC03A.
- **Change:** Cache allowlisted reads/searches keyed by operation version, args, project and fingerprints; hits are labelled with their original time.
- **Accept:** A repeat runs no worker; changing content, ignore rules, tool version or a link boundary invalidates; other projects never share; executions avoided are compared against hashing cost.
- **Watch out:** Shell, tests, builds and network reads are non-cacheable. A hit is never a fresh test.

### TC02 — Reviewed tool plans

- [ ] Execute a visible typed plan with fewer round trips.
- **Priority:** 2. **Needs:** —. **Builds on:** R02, A03, TC01B.
- **Change:** A bounded dependency graph with validated args and effects. Only enforceably read-only independent calls run in parallel; writes and unknown effects are serialized; every effectful call is reviewed.
- **Accept:** Independent reads overlap and writes never do; a failed prerequisite blocks its descendants; cancellation starts no queued node; counts and time are compared with serial execution.
- **Watch out:** A batch is not blanket consent.

### TC05 — Failed-step recovery

- [ ] Recover a failed plan without repeating confirmed mutations.
- **Priority:** 2. **Needs:** TC02. **Builds on:** R02, TC03A.
- **Change:** Persist node args, fingerprints, results and certainty; preview resuming failed/unstarted nodes only.
- **Accept:** A confirmed write is not repeated; a lost mutation response blocks until reconciled; stale approvals start nothing.
- **Watch out:** Never promise exactly-once execution across crashes.

### D01 — Impact-directed verification plan

- [ ] Recommend the checks most likely to catch a change's defects.
- **Priority:** 2. **Needs:** TC02. **Builds on:** C01, R01, PK05A, T02B.
- **Change:** Use changed symbols, local dependencies, test mappings and existing Dex failure records to propose a minimal check set with reasons, through Jolteon's review.
- **Accept:** The caller's test catches a changed callee; a shared-config change triggers a broader plan; targeted passes are labelled targeted.
- **Watch out:** A recommendation is neither proof of completeness nor permission to run.

### Q03 — Optimization quality benchmark

- [ ] Prove the optimizations reduce measured work without weaker answers.
- **Priority:** 2. **Needs:** TC02, TC04, TC05, D01. **Builds on:** T01C, T02B, T03B, T04, TC01B, TC03B.
- **Change:** Paired baseline/optimized journeys (bug fix, explanation, noisy log, resume) with fixed model/settings/repo, cold and warm caches separated, and all optimizer costs included.
- **Accept:** ≥3 paired trials per journey with raw samples, correctness, time, turns, tools and usage; the same functional/permission checks; regressions published.
- **Watch out:** Live runs are paid and need explicit user approval.
