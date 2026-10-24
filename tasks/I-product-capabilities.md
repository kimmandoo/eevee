# I — Additional product capabilities

Planned after the verified v0.1.29 release on 2026-10-24; none of these features is claimed shipped or authorized for implementation by this roadmap update. Existing context/tool and acceptance work stays in [F](F-context-tools.md) and [H](H-provider-acceptance.md). Reuse the current executor, writer, approvals and receipt authority; [design rules](../docs/DESIGN_RULES.md) remain binding.

Verification for each task: targeted consumer-visible regressions and an actual disposable CLI/TUI journey. Reuse bounded native coverage profiles; do not multiply every case across every architecture or test internal wiring. Live model interpretation requires separate authorization.

### GP01 — Complete offline data snapshot

- [ ] Create and inspect a complete pre-upgrade backup without modifying its source.
- **Priority:** 1. **Needs:** —. **Builds on:** E03, P02, T01C, TC03A.
- **Gap:** Readable task export is not a complete database, journal and artifact backup; upgrades currently require the user to back up the whole data directory.
- **Change:** Explicit offline snapshot command, exclusive ownership with settled writers, create-only destination and versioned manifest. Include owned database, journals, outputs, artifacts and archived tasks; inspect integrity read-only. Disclose private app-owned content and any secrets before copying.
- **Accept:** Snapshot a task with a draft, run, receipt and artifact; verify identities and all member digests. Live owner, destination collision, links/escaping paths, changed/missing/truncated members and unknown manifest versions refuse truthfully. Cancellation or disk-full never advertises a complete backup; source bytes remain unchanged.
- **Watch out:** Do not migrate or repair the source to make its pre-upgrade backup. No external credential-store access, cloud sync, scheduling or new encryption subsystem.

### GP02 — Restore verified snapshot into a fresh data directory

- [ ] Recover a complete verified backup without overwriting current work or replaying operations.
- **Priority:** 1. **Needs:** GP01. **Builds on:** E02, E03, P02, P04B.
- **Change:** Review origin, format, privacy and a nonexisting destination; validate before publishing a usable restored directory. Preserve identities, preferences, drafts and retained evidence. Restore neither active requests nor usable grants.
- **Accept:** Launch against a restored snapshot and inspect the original task/draft/run/receipt/artifact with zero dispatch. Corruption, links/traversal, incompatible reader, interrupted restore and destination collision leave the backup and current data unchanged. Unknown historical outcomes remain unknown.
- **Watch out:** No merge, in-place downgrade, project relocation, credential import or portable provider-session claim. Reuse current recovery and older-writer refusal.

### GP03 — Reviewed regular-file deletion

- [ ] Delete one exact regular file through the existing reviewed executor.
- **Priority:** 1. **Needs:** —. **Builds on:** A03, C01, R02.
- **Gap:** Current provider write operations support creation/replacement, not a first-class exact-file deletion contract.
- **Change:** Extend native and controlled Codex operation handling with frozen source identity/content and explicit irreversible-effect review. Retain project scope, writer ownership, protected-path and exclusion policies; unsupported adapters report that limit.
- **Accept:** Actual deny/cancel leaves bytes intact; allow removes only the reviewed file and records the actual result. Source/ancestor changes, links/reparse points, protected data and scope changes refuse before effect. Exercise both supported routes and stale-source boundaries.
- **Watch out:** No directory/recursive deletion, shell escape, batch grant or fictional rollback. Interrupted outcomes follow the existing unknown-outcome contract.

### GP04 — Reviewed same-project file rename

- [ ] Move one regular file to an exact unoccupied destination after review.
- **Priority:** 2. **Needs:** —. **Builds on:** A03, C01, R02.
- **Change:** Freeze source bytes/identity, destination and parent identities; use existing writer ownership and consent. Preserve literal content and supported metadata. Reject unsupported case-only/cross-volume moves before any effect.
- **Accept:** Native and controlled Codex journeys cover allow, cancel, changed source, changed ancestor and newly occupied destination. Never overwrite a destination or affect unrelated dirty files; journal actual effects and invalidate source freshness accurately.
- **Watch out:** Independent of GP03; no directory move, import rewriting, automatic rename tracking or claimed rollback after partial/unknown effects.

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
