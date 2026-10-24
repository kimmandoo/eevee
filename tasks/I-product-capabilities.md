# I — Additional product capabilities

GP01–GP04 completed in Waves1–2; specifications and scoped proof are in [COMPLETED.md](COMPLETED.md). The three remaining capabilities below are planned only. Reuse the current executor, writer, approvals and receipt authority; [design rules](../docs/DESIGN_RULES.md) remain binding.

Verification for each task: targeted consumer-visible regressions and an actual disposable CLI/TUI journey. Reuse bounded native coverage profiles; do not multiply every case across every architecture or test internal wiring. Live model interpretation requires separate authorization.

### GP05 — Project search of retained task facts and evidence summaries

- [ ] Find retained work across tasks in the current project without network access.
- **Priority:** 2. **Needs:** —. **Builds on:** E02, E03, R02, S02, T01C.
- **Gap:** Existing task-scoped journal, execution, source and artifact search does not provide cross-task retrieval.
- **Change:** Bounded cancellable literal search over explicitly named fields: goals, criteria, decisions, approved notes, retained provider-journal text and run summaries. Attribute task, kind, time and excerpt; offer an archived filter and guarded navigation to original evidence. Label incomplete coverage.
- **Accept:** Search two current-project tasks plus one foreign-project task across archive/restart. Foreign content appears in neither snippets nor counts. Purge invalidates indexed material; opening a result preserves dirty drafts and ownership guards. Observe zero network or implicit context selection.
- **Watch out:** No embeddings, generated summaries, reconstruction of process-local conversations or purged bodies, cloud index or automatic attachment.

### GP06 — Reviewed screenshot input on a supported native route

- [ ] Explicitly attach one bounded PNG/JPEG to a documented image-capable native Responses route.
- **Priority:** 2. **Needs:** —. **Builds on:** C02, C03A, T01A, T01C.
- **Gap:** Current native messages and approved source capture are text-only; binary-source rejection is not image support.
- **Change:** Validate format, compressed/decoded bounds, dimensions and immutable identity. Show disclosure, bytes, selected route capability and retention/purge policy. Respect containment, exclusions, no-link policy and request-body limits. Unknown/unsupported capability refuses rather than silently dropping the image.
- **Accept:** Protocol evidence contains the exact approved image bytes; receipt distinguishes actual serialized bytes from unknown image tokens. Changed/deleted source invalidates dispatch; removal/cancel sends nothing. Exercise restart, quota and purge. Actual model interpretation remains a separate authorized live row.
- **Watch out:** Images can contain secrets that cannot reliably be redacted. Do not claim visual review from a hash when the terminal cannot display the image. No URL fetch, automatic clipboard capture, OCR, PDF/audio/video or all-adapter expansion.

### GP07 — Reviewed selected local Git commit

- [ ] Commit one explicit whole-file changeset locally without silently including other work.
- **Priority:** 3. **Needs:** —. **Builds on:** C01, R01, R02, A03.
- **Gap:** Current repository integration intentionally observes Git read-only. This task requires a scoped design-contract amendment before implementation; this plan does not change that contract.
- **Change:** Ordinary supported repository and initially clean index only. Freeze selected paths/content, HEAD, index, message, identity and configuration effects. Review and bind the exact commit operation to existing consent. Hooks, filters, signing and helpers must be enforceably excluded with disclosure or cause refusal.
- **Accept:** In a disposable repository with two dirty files, commit only the selected file and retain the other unchanged. Existing staged changes, merge state and HEAD/source/index/configuration drift refuse. Record actual commit ID and honest partial index/ref outcomes; a commit is not verification of its content.
- **Watch out:** No push, authentication, branch/worktree management, rebase, conflict resolution, automatic retry/reset or claimed rollback. File approval must not silently authorize repository code execution.
