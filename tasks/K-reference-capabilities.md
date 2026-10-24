# K — Reference-derived capabilities

Requested on 2026-10-24 after v0.1.29. These six tasks adapt useful behavior from the read-only `../oh-my-pi` reference at commit `f979f4639777858de28e6ecb99d577443abe0b7b`; they are planned, not implemented. Pinned upstream links identify the inspected reference, not proof that Eevee already supports it.

Existing [F](F-context-tools.md), [I](I-product-capabilities.md), [J](J-eevee-workflows.md) and [completed work](COMPLETED.md) retain their scopes. In particular, do not rebuild symbol excerpts, compaction, catalog search, artifact paging, memory, party/formation/relay, or the proposed delete/rename/search/screenshot/Git features. [Design rules](../docs/DESIGN_RULES.md) remain binding. Each implementation needs focused behavioral regressions and an actual disposable runtime journey through existing bounded CI profiles, not another full platform cross-product.

Reference inspection did not copy implementation or prompts. The reference root [MIT license](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/LICENSE) requires retained notices for copied substantial portions; any later code reuse must check the actual component and dependency licenses separately.

### RX01 — Snapshot-bound multi-hunk editing of one file

- [ ] Apply several exact replacements with one final-file review instead of repeated reads and writes.
- **Priority:** 2. **Needs:** —. **Builds on:** A03, C01, R02.
- **Reference:** [Original-snapshot hunk coordinates](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tools/edit.md#L47-L59) and [prepare/conflict handling](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tools/edit.md#L145-L154).
- **Eevee gap:** `src/providers/writes/tools.rs` accepts one `old_text`/`new_text` replacement against a full-file digest. This is a richer single-file edit representation, not TC02's execution plan or a multi-file transaction.
- **Change:** Extend the existing edit operation with at most32 literal replacements bound to one original full SHA-256. Resolve all spans against that baseline, reject overlap/ambiguity and construct one final proposal through the existing writer and consent. Retain the64-KiB final-file limit and actual adapter capability disclosure.
- **Accept:** Two separated edits land after one complete review with unrelated CRLF/Unicode bytes unchanged. A replacement's output cannot become another replacement's input. Empty/ambiguous anchors, overlap, oversized output, stale source/ancestor and cancellation cause no write. Exercise supported native and controlled-host routes with exact final bytes.
- **Watch out:** No short normalized tags, fuzzy stale recovery, last-edit-wins overlap, registers, automatic dialect selection or batch grants. GP03/GP04 still own deletion/rename.

### RX02 — Pre-write Rust syntax-regression disclosure

- [ ] See an introduced Rust parse failure before approving a file mutation.
- **Priority:** 2. **Needs:** —. **Builds on:** A03, T02B.
- **Reference:** [Edit syntax warnings and repair behavior](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tools/edit.md#L139-L143). Adapt detection, not its post-write warning or automatic repair semantics.
- **Eevee gap:** `src/workspace/context/symbols.rs` already uses a Rust parser for context; `src/providers/writes.rs` proposal preparation does not compare baseline/proposed parse outcomes.
- **Change:** Parse bounded frozen `.rs` baseline/proposal bytes locally off the input/render path; place the result in the existing file review. Distinguish newly invalid, pre-existing invalid, invalid-to-valid, parseable and unavailable/unsupported analysis. Creation has no fictional baseline. This is advisory disclosure, not an extra approval queue.
- **Accept:** A missing delimiter is visible before any write, including compact English/Korean review. Parseable means neither compiled nor type-correct nor verified. Changed proposal bytes invalidate the analysis; changed disk identity retains stale-write refusal. Cancel starts no subprocess, formatter, repair or model call and changes no file.
- **Watch out:** Never alter reviewed bytes automatically, run a background compiler or treat a parser result as a passing check. D01 retains verification planning.

### RX03 — Bounded Rust structural source search

- [ ] Find a code shape across source files, rather than only a filename, literal text or ranked symbol excerpt.
- **Priority:** 2. **Needs:** —. **Builds on:** C02, T02B, TC01A, TC01B.
- **Reference:** [Structural captures and output](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tools/ast-grep.md#L28-L51) and [scan limits](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tools/ast-grep.md#L94-L100).
- **Eevee gap:** Existing provider file discovery matches names; syntax-derived context ranks identifiers. Neither is a user-specified structural-pattern query.
- **Change:** Document a Rust-only expression-pattern grammar with named node captures and repeated-capture equality. Return deterministic project-relative ranges, captures, full-source identity and honest scan/parse omissions. Reuse local parser, discovery/privacy rules and existing adapter exposure; define hard candidate, scanned-byte, match and response-byte limits before implementation.
- **Accept:** Equivalent call shapes match across whitespace/newlines; lookalikes inside comments/strings do not. Invalid pattern, unsupported language, parse failure, complete no-match and incomplete scan remain distinct. Returned ranges resolve to their digest. Cancellation, exclusions, protected paths, links and limits do not leak content; paging is bound to the query/source snapshot and refuses changed sources.
- **Watch out:** No semantic name-binding claims, structural rewriting, automatic context selection, network grammar acquisition or unbounded project traversal. A partial scan is not proof of absence.

### RX04 — Parked future-request drafts without scheduling

- [ ] Keep several separate unsent follow-ups while a partner works, without automatically running them.
- **Priority:** 2. **Needs:** —. **Builds on:** E02, E03, UX01, PT05B.
- **Reference:** [Follow-up and dequeue editor actions](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/keybindings.md#L43-L45) and [queued follow-up implementation](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/packages/coding-agent/src/session/agent-session.ts#L8246-L8270).
- **Eevee gap:** The owned composer preserves one future Request during work; a multi-entry unsent draft tray is distinct from moves/templates, tool plans and partner relays.
- **Change:** Explicitly park, inspect, restore and discard bounded user-authored text entries scoped to project/task/partner. Define count/byte limits; use the existing durable writer and migration/quota contract. No provider/session/tool state accompanies entries. Clear the composer only after acknowledged parking of the exact revision.
- **Accept:** Park two drafts during a running request, let it finish, restart and switch owners: nothing sends. Restoring over a nonempty draft requires exact replacement review; cancellation preserves both texts/caret. Save failure retains recoverable content. Restore only stages text; ordinary explicit Send prepares current context and fresh authority.
- **Watch out:** Do not copy automatic queue draining, live steering, synthetic messages, template expansion at enqueue or restart replay. This is a draft tray, not a job scheduler; removing a partner/task follows existing privacy cleanup.

### RX05 — Reviewed external editing of the current Request

- [ ] Compose a long Request in a chosen external editor and review the returned text without sending it.
- **Priority:** 2. **Needs:** —. **Builds on:** UX01, P01, P02, R01.
- **Reference:** [External editor resolution, process and temporary-file lifecycle](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/packages/coding-agent/src/utils/external-editor.ts#L19-L87).
- **Eevee gap:** In-terminal editing and `src/platform/terminal.rs` terminal lending exist; a reviewed draft-to-editor round trip does not. Reuse those owners rather than a second runner.
- **Change:** Initially require committed global idle state. Review exact executable/argv and plaintext disclosure; use a private bounded temporary file, terminal lending/restoration and identity-checked return. Treat environment editor configuration as untrusted input, not shell syntax to execute implicitly. Show the returned candidate before replacing the owning draft.
- **Accept:** Real disposable editor journeys preserve UTF-8, CRLF and trailing newlines within existing limits. Cancellation, nonzero exit, missing executable, invalid/oversized output, unsafe replacement/link and stale owner/draft leave the original intact. Normal/error paths restore terminal modes; temp cleanup failure is reported. Importing text starts no request or context selection.
- **Watch out:** No full conversation/credential export, silent trimming or arbitrary shell interpolation. The editor is external, not sandboxed; it may create backups or network effects. Removing Eevee's temp file does not erase editor-owned copies. Secret handling/disclosure remains explicit.

### RX06 — Named landmarks on retained responses

- [ ] Jump to a user-labelled retained explanation without replaying or branching its request.
- **Priority:** 3. **Needs:** —. **Builds on:** E02, E03, R02, UX01.
- **Reference:** [Session labels and labelled navigation](https://github.com/can1357/oh-my-pi/blob/f979f4639777858de28e6ecb99d577443abe0b7b/docs/tree.md#L38-L79). Adapt labels only, not conversation-leaf switching.
- **Eevee gap:** Current response navigation has item/position anchors, but no durable user-named landmarks. GP05 is cross-task search and PK09 is contribution inspection; neither owns manual positional labels.
- **Change:** Bounded label metadata points to an existing durably retained response/journal identity and optional bounded position. List, rename/remove and jump within the current task using existing inspectors/writer. Show original request/partner attribution; do not persist process-local transcript bodies merely to support labels.
- **Accept:** Label/jump while another partner streams, resize/reflow, restart and purge. Draft/caret and paused-output position remain owned; missing/purged targets are unavailable or consistently removed, never reconstructed. No implicit owner switch, provider request, old question reopening or replay occurs.
- **Watch out:** A label is not approved memory, a decision, verification or context selection. No conversation tree, generated switch-summary, file rewind or execution rollback.

## Deliberately not imported

- Autonomous follow-up execution, auto-retry/model repair and blanket write permissions conflict with explicit per-request/per-effect authority.
- Full LSP server discovery/execution, server-originated edits and compiler-backed diagnostics need a separate reviewed process/configuration design; syntax analysis here does not imply those capabilities.
- Structural rewrite apply based on live reruns or match counts is not exact reviewed-byte approval. No AST mutation is included in RX03.
- Short normalized snapshot tags and fuzzy recovery do not replace Eevee's full-digest, original-byte identity.
- Existing context, memory, tools, party and portable workflows are reused, not counted again as new reference tasks.
