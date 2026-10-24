# F — Context, token economy and tool calling

Local deterministic optimizer only: no optimizer-initiated network, hosted summarization, embeddings, telemetry or tokenizer downloads. Keep exact Eevee-sent bytes, labelled estimates, provider-reported usage and billing distinct; unknown stays unknown, never zero. Tool discovery, paging and caching apply only to documented adapter capabilities and Eevee-owned execution. T05 was dropped on 2026-10-14.

The context/tool milestone and Wave3–4 observation cache, reviewed plans, failed-step recovery and impact-directed checks are complete; specifications and scoped proof are in [COMPLETED.md](COMPLETED.md). The separately authorized paired quality benchmark remains below.

### Q03 — Optimization quality benchmark

- [ ] Prove the optimizations reduce measured work without weaker answers.
- **Priority:** 2. **Needs:** —. **Builds on:** T01C, T02B, T03B, T04, TC01B, TC03B, TC02, TC04, TC05, D01.
- **Change:** Paired baseline/optimized journeys (bug fix, explanation, noisy log, resume) with fixed model/settings/repo, cold and warm caches separated, and all optimizer costs included.
- **Accept:** ≥3 paired trials per journey with raw samples, correctness, time, turns, tools and usage; the same functional/permission checks; regressions published.
- **Watch out:** Live runs are paid and need explicit user approval.
