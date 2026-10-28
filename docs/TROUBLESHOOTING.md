# Troubleshooting

### [2026-10-28] v0.1.35 release upload failed without version changelog notes

- **Context / Symptom:** Native run 40638360962 passed at `ad5c634`, but release run 40640741692 failed at "Upload all assets before publishing"; no GitHub release was created.
- **Root Cause:** The upload step reads `## <version>` from CHANGELOG.md for release notes. Only dated `## 2026-10-28` entries existed for 0.1.35, so the split raised before `gh release create`.
- **Solution:** Added a `## 0.1.35` section, pushed main, waited for a fresh native run on that commit, then moved the unpublished `v0.1.35` tag to it and pushed again.
- **Prevention / Reference:** Add the `## <version>` changelog section together with the version bump, before tagging.

### [2026-10-28] Accepted handoffs and tool selections still blocked Requests after ordinary saves or switches

- **Context / Symptom:** After the provider-switch fix, two related paths still refused every later Send. An explicitly accepted same-task handoff failed with `Handoff task changed or belongs to another task; prepare and accept a fresh handoff` as soon as the composer saved the new Request (every Send does). A `/tools` selection made for one adapter returned `Finish catalog setup for this exact task/partner/adapter before dispatch` after switching provider, form partner or task, with no hint how to recover.
- **Root Cause:** Handoff admission compared the raw task revision, which advances on request-only saves even though the Request text is not part of the portable document. The tool catalog treated a selection owned by another adapter/task/partner as an unfinished setup instead of non-portable state.
- **Solution:** `validate_dispatch` admits a later revision only when the same task's goal, next action, criteria, derived portable facts/unresolved items and sources still match exactly; the document is never refreshed. Changed/foreign handoffs still fail closed and the refusal now names `/handoff-clear`. `tool_catalog::State::selection` excludes a foreign-owner selection (kept for its own owner) and errors only for the current owner's busy/failed setup.
- **Prevention / Reference:** Regressions `later_revisions_admit_only_unchanged_portable_facts`, `accepted_imports_stay_exact_across_provider_switch_and_request_saves`, `foreign_owner_selection_is_excluded_without_blocking_or_forgetting`. When adding new task-bound request state, decide whether a request-only save or provider switch should drop, exclude or block it; blocking needs an actionable recovery command.

### [2026-10-28] Provider switching blocked later Requests with stale handoff and empty JSON errors

- **Context / Symptom:** A Windows PowerShell user reported `Handoff task changed or belongs to another task; prepare and accept a fresh handoff` together with `Invalid frozen request JSON: EOF while parsing a value at line 1 column 0`. Both were reproduced by consumer regressions before the fix (`artifact://7`, `artifact://10`).
- **Root Cause:** Switching providers after an existing response/session implicitly selected a handoff at the old task revision, including when the switch was cancelled. Ordinary Request saving advanced that revision; strict handoff admission correctly refused it. Preparation therefore returned no payload, but receipt staging still parsed the empty body and appended EOF. This lifecycle code is platform-neutral, not a PowerShell JSON/encoding failure.
- **Solution:** Removed implicit handoff selection during provider switching; ordinary preparation already freezes the current committed task/context. Kept deliberately imported documents exact and their stale/foreign refusal unchanged. Blocked preparation now retires any earlier staged receipt without creating a new one; malformed unblocked requests still fail. Older builds can use `/handoff-clear` to deselect the stale document and explicitly Send again without deleting the task or export.
- **Prevention / Reference:** Rust1.90.0 full source gates passed1,841 tests/two ignored. Actual120×36/60×18 Implement journeys exercised cancelled/accepted provider switches, saved follow-ups, private/session restart and expiry inspection in `target/connection-fix-runtime-final`;18 exact loopback POSTs, no implicit sends. No native PowerShell or authenticated live-provider result was claimed.
- **Verification-fixture distinction:** The first runtime attempt stopped on a helper's blanket `Eevee`-name ban: the fixed `eevee_run_command` schema legitimately described “Eevee's owned runner.” The session-only driver applied that context-leak check to actual context rather than static tool descriptions, retained full unmodified wire bodies, and then passed all four complete Implement journeys. Production packets, permanent helpers, deadlines and canonical role/text/authentication checks were unchanged.

### [2026-10-28] Reopened account setup concealed an expired saved credential

- **Context / Symptom:** The user also reported connections disappearing on later visits. Session-only setup intentionally retained no credential after process exit; saved account access tokens also expired. Separately, Account → Advanced settings displayed no saved-source expiry reason. The consumer regression reproduced `source diagnostic must survive setup: None` (`artifact://11`).
- **Root Cause:** Authentication intentionally discarded refresh tokens and required explicit reconnect. Recovery setup copied nonsecret editable defaults without checking stored/JWT expiry, collapsed saved-source errors, and Account → Advanced then cleared its notice.
- **Solution:** Preserved safe private-source errors, reused grant expiry admission and existing JWT claims checks for setup diagnostics, and carried those notices into Account Advanced. Session-only remained the default; explicit private plaintext storage remained opt-in and did not extend access-token lifetime. No automatic renewal, account rotation, credential-store mutation or inference retry was added.
- **Prevention / Reference:** `/connect chatgpt` → Storage → SAVE private local plaintext before Browser/Device login retains only the access grant. Reconnect explicitly after expiry. Actual compact/wide isolated-HOME restarts proved session credentials absent, explicitly saved fixture credentials restored, zero startup replay, and visible expiry without network or saved-file mutation. Existing full-suite regression also covered JWT expiry and invalid stored JSON. The missing local `pyte` dependency was installed only in `/tmp/eevee-connection-fix-venv`; Rust LSP was restored by installing the missing rust-analyzer component and reloading it.

### [2026-10-28] An interrupted fixture socket read changed a round-limit result

- **Context / Symptom:** Native40574273330 passed Check/Linux-arm64/macOS×2/Windows. Linux-x86_64 failed `round_limit_refuses_the_terminal_call_set_without_executing_it`: the HTTP peer panicked at a raw read with `Os { code: 4, kind: Interrupted, message: "Interrupted system call" }`, so the consumer correctly observed transport Unknown rather than the expected deliberate round-limit refusal.
- **Root Cause:** The bounded test peer assumed socket accept/read never returned the operating system's Interrupted condition. Its peer failure prevented the planned terminal response; this was not a model-limit or production retry defect.
- **Solution:** Continued only interrupted accept/header/body-read system calls at the existing byte position and within the existing admission/deadline policy. EOF, other errors, body bounds, exact round counts and the original refusal/effect assertions stayed unchanged. No provider request was restarted or replayed and no production code changed. Original log: `target/ci-40574273330-failed.log`; source-gate results and rebuilt executable identity are recorded in the checkpoint.

### [2026-10-28] TLS-root settings blocked a cleartext loopback WebSocket

- **Context / Symptom:** Native40567052809 passed Check and both macOS jobs, but both Linux test suites refused plain loopback sockets with `Native WebSocket refuses custom ambient TLS trust; no trust-policy bypass or fallback`.
- **Root Cause:** The new transport checked ambient TLS-root variables before considering whether the frozen destination actually used TLS. Linux runner settings exposed that cleartext `ws://` cannot consume those trust roots.
- **Solution:** Limited that check to secure `wss://`, leaving exact destination admission, remote cleartext refusal and all proxy/secure-trust guards intact. Added a child-local environment regression that performs the real retained tool chain with unavailable TLS-root settings and separately requires secure scoped request refusal. Its focused regression passed; full corrected-source and native qualification remain recorded in the checkpoint.

### [2026-10-28] Windows held-response verification requested idle-only capture

- **Context / Symptom:** Native40567052809 Windows passed Rust/build/install/ConPTY8 and six workflow drivers. Conversation failed at its held request Enter with `native console output did not settle within the bounded capture window`; fixture cleanup then masked it as `loopback listener/worker cleanup exceeded deadline`.
- **Root Cause:** The Windows session's normal idle-only input capture was used while the controlled provider deliberately kept Receiving response active. Cleanup also left that owned HTTP gate held after an assertion.
- **Solution:** Used explicit non-idle input pumping only for the held request, future Unicode paste/caret/save and cancellation. Exact POST/visible-partial/SQL-owner/outcome/release predicates still determine settlement; idle defaults and deadlines did not change. Failure cleanup now releases the fixture's own gate. Original evidence is retained at `target/ci-40567052809/windows`.

### [2026-10-28] External uncertainty was confused with a live durable transport lease

- **Context / Symptom:** New postdispatch disconnect verification reported `opaque external effects were silently released` after real Unknown settlement.
- **Root Cause:** The fixture required the durable active-work row to remain byte-identical. Existing settlement correctly drained that transport lease while retaining the separate process-local unknown-effect admission barrier.
- **Solution:** Awaited actual lease settlement, then deliberately attempted Send with the retained future Request and proved refusal, zero new process/query/submission/lease, unchanged draft/caret and local-only diagnostic remediation. Both sizes across all five external routes passed20/20 in `target/followup-disconnect-final-d`; production lease policy did not change.

### [2026-10-28] Transport verification addressed the wrong terminal owner

- **Context / Symptom:** Advanced preflight remained on home, cancellation keys were consumed by neutral file review, independent hazards were blocked by retained diagnostics, and compact streaming inspection could be hidden by an evolution receipt or a later Plan.
- **Root Cause:** Fixtures assumed global key ownership, editable focus and latest-item visibility across distinct surfaces. A rendered file-diff card also used a semantic Failed outcome with a presentation-specific `[FILE]` label; matching the card word instead of its outcome rejected correct state.
- **Solution:** Explicitly opened prepared Review, Esc-deferred without answering before global cancellation, focused Request through the existing helper, dismissed only the known neutral receipt, paged to the actual public partial, isolated independent hazards without clearing diagnostics and explicitly reviewed ALL-owned-work shutdown. Outcome assertions read the semantic terminal status independent of card decoration. All20 WS and20 disconnect journeys plus four original-outcome continuations passed on frozen B; no timeout, privacy rule, grant or production outcome classification changed.

### [2026-10-28] Downloaded package verification used newer private source metadata

- **Context / Symptom:** Public0.1.33 assets and the first11 installation groups passed, then `installation_smoke.py` rejected its lifecycle fixture at `assert f'version = "{current}"' in manifest`.
- **Root Cause:** The downloaded executable was correctly0.1.33, while the private follow-up checkout had advanced Cargo metadata to0.1.34. The driver compiles a real older-version lifecycle fixture and correctly requires its source to match the package under test.
- **Solution:** Ran the complete public installation driver from the explicit detached qualified0.1.33 commit, without modifying its assertion or the package. All24 groups, including34 installed native terminal scenarios, passed in `target/release-033-install-matched`; removed the owned source worktree afterward.
- **Prevention / Reference:** Pin public-package lifecycle verification to the package's qualified source commit; do not substitute mutable follow-up metadata or manufacture a version string.

### [2026-10-28] Outcome cards changed transcript geometry and compact paging

- **Context / Symptom:** New local outcome cards caused `response header missing`; wide extraction dropped leading public characters, and compact artifact inspection at transcript start could no longer see a later final answer.
- **Root Cause:** The old helper derived the body column from the word Response inside metadata, assumed only provider-response card kinds and included mutable notice/partner chrome. The artifact fixture also assumed its final answer fit on the first page despite explicitly navigating to the start.
- **Solution:** Read semantic card kinds, navigation-owned terminal-cell bounds, the actual displayed row range and observed partner edge; reconstructed retained rows by absolute index without overlap or notice contamination. The artifact journey explicitly paged to the real final answer while retaining all source/receipt assertions. Both-size B artifact journeys passed, and native outcome ownership/reason/context/draft assertions reached their final safe-exit review. Exact source evidence remains under `target/followup-native-final-b`.
- **Prevention / Reference:** Terminal columns are cells, not string offsets; a word inside metadata is not a layout anchor. Explicit start-page inspection must not imply that a later answer is already visible.

### [2026-10-28] Valid streamed tool calls used sparse output indices

- **Context / Symptom:** Integrated WebSocket consumer regression `native_websocket_registered_item_lane_streams_real_tool_arguments` settled Unknown with `WebSocket duplicate/interleaved output item refused` before the actual tool continuation.
- **Root Cause:** The lane validator equated the next added index with the number of observed added events. A valid function call at index1 was rejected when index0 reasoning appeared only in the authoritative completion snapshot.
- **Solution:** Allowed sparse strictly increasing added indices while retaining bounded indices, duplicate identities/indices, registered lanes, response/sequence identity and authoritative snapshot checks. Extended the actual argument-delta→arguments-done→item-done→completion consumer path and rejection boundaries; all20 WebSocket regressions passed (`artifact://197`).
- **Prevention / Reference:** Observation order is not a complete output array. Validate lifecycle/identity against the final snapshot without requiring every nonpublic output to have streamed an added event.

### [2026-10-28] Partial-output fixtures expected an unbounded terminal token

- **Context / Symptom:** New abrupt-loss and cancellation regressions waited for an entire unterminated final word, so a correct credential-tail holdback prevented the expected observation and cancellation.
- **Root Cause:** The fixture treated transport chunk boundaries as safe public token boundaries. Ordinary streaming deliberately withholds an unresolved terminal token.
- **Solution:** Kept the required complete public-prefix assertions and supplied a bounded public line followed by an unresolved credential canary. Both interruption paths now also assert that the credential is never displayed, no completed public history is admitted and no additional request is sent. The20-case WebSocket suite passed without changing the production redactor or deadlines.

### [2026-10-28] Attachment persistence preceded the Request return frame

- **Context / Symptom:** Native run40547662468 passed Check/macOS×2/Linux-arm64/Windows. Linux-x86_64 compact context failed after the second source attachment with `refusing to dismiss unknown popup`, showing `Review draft discard`.
- **Root Cause:** The source row was durably committed before the terminal displayed the transition back to Request. The helper observed the SQL row, mistook the stale source-drawer frame for the current surface and sent Esc after the app had already returned; the normal draft-discard guard then correctly opened.
- **Solution:** `records_smoke.Journey.pin_range` now waits for the actual Request footer after the existing exact SQL capture acknowledgment before navigating. It neither dismisses a new confirmation nor changes capture/consent/HTTP predicates, deadlines or coverage. The full two-size context journey passed in `target/ci-40547662468-context-fixed`; original failure evidence remains under `target/ci-40547662468/linux-x86_64`.
- **Prevention / Reference:** Durable acknowledgment and rendered navigation acknowledgment are different boundaries. Wait for both rather than sending another Escape or treating silence as settlement.

### [2026-10-28] Nested context sends still forced idle-only capture during inference

- **Context / Symptom:** Native run40540661883 passed Check and four native targets, including Windows. macOS-arm64's compact context journey failed in inherited `moves_smoke.Journey.send_request` with `native console output did not settle within the bounded capture window`; the wide journey passed. Retained evidence is `target/ci-40540661883/macos-arm64`.
- **Root Cause:** The context-only session subclass forwarded every send with `settle=True`, overriding the shared session's asynchronous POSIX default. The earlier explicit async fix covered its direct source-tool path but not inherited request senders; legitimate Connecting animation could therefore fail before the existing HTTP/answer/lease predicates ran.
- **Solution:** Removed the behaviorless forwarding subclass and used the existing `SurfaceSession` directly. No quiet deadline, retry, coverage, expected response or authority check changed. The actual two-size context journey passed in `target/ci-40540661883-context-fixed`.
- **Prevention / Reference:** Keep idle-only capture opt-in at appropriate call sites; do not use terminal silence as evidence that an active request settled.

### [2026-10-28] Rust language-server shim resolved without its installed component

- **Context / Symptom:** Exported-symbol reference lookup reported `LSP server exited unexpectedly (code 0)`, although status said rust-analyzer was configured. Direct version inspection reported `Unknown binary 'rust-analyzer' in official toolchain 'stable-aarch64-apple-darwin'`.
- **Root Cause:** PATH resolved the rustup shim, but the selected stable toolchain lacked its rust-analyzer component; the device also cached the initialization failure.
- **Solution:** Installed only the missing stable rust-analyzer component, reloaded the server and refreshed the source snapshot after startup indexing. A real Preview reference query then returned75 references. The project's qualification toolchain remained Rust1.90.0; no global default toolchain or product setting changed.

### [2026-10-28] Real Ghostty capture initially showed setup or monochrome cell art

- **Context / Symptom:** A fresh isolated Ghostty1.3.1 launch first showed `No image / Images: setup`; after artwork installation it showed a monochrome character approximation rather than native pixels. The old README screenshot had explicitly disabled images and reconstructed text cells.
- **Root Cause:** The isolated HOME had no pinned artwork cache. Automation color/locale/multiplexer environment settings also retained the launch's monochrome/ASCII graphics guards even inside a genuinely graphics-capable terminal.
- **Solution:** Used the normal explicit artwork downloader into the isolated cache, which SHA256-verified all nine pinned originals. Launched the actual binary in a separate real Ghostty window with its own `xterm-ghostty`/truecolor environment, removed automation-only NO_COLOR/locale/multiplexer overrides and selected UTF-8. Visually inspected the actual native Eevee pixels and captured only that owned window with macOS `screencapture`; no program guard was disabled and no image overlay was fabricated.
- **Prevention / Reference:** A text-only PTY parser cannot prove graphics. Check actual artwork setup, native protocol/cell metrics and the terminal's real color/locale settings; document screenshot artwork separately from image-free software packages.

### [2026-10-28] Complete Codex plan snapshots lost their public suffix

- **Context / Symptom:** All four controlled Codex progress journeys reached the malformed-plan transcript check but could never inspect its final marker. Evidence is `target/unassigned-progress-d2-codex-native/codex-120x36-en/en-malformed-transcript-screens.json`.
- **Root Cause:** `turn/plan/updated` delivers a whole structured snapshot, but its reserved plan item used the ordinary incomplete-message redactor path. That path correctly holds a possible credential suffix for future bytes; no final plan-item event flushed the already-complete snapshot.
- **Solution:** Routed only the reserved complete plan snapshot through the existing final text redactor. Kept its evolving item incomplete, ordinary message-delta holds unchanged and provider progress unverified. Added a consumer regression covering the public closing suffix, secret removal, Korean final-step text and continued holdback for an ordinary message; a new frozen candidate and full feature matrices are required.
- **Prevention / Reference:** Distinguish a complete notification's text boundary from task/item completion. Do not relax redaction or fabricate completion merely to expose a tail.

### [2026-10-28] Taskless SDK discovery was rejected before model metadata

- **Context / Symptom:** The controlled fresh-connection SDK journey loaded the module but never reached `accountInfo` or created the metadata query; `unassigned-journey-claude-fixed-debug-c/wire.json` retained the failure.
- **Root Cause:** Fresh model inspection has no persisted task and correctly supplies task_id0, but the strict shared run schema required a positive ID even for empty, nonwriting metadata inspection.
- **Solution:** Admitted nonnegative task IDs at the shape boundary, then required a positive owner and frozen root for every noninspection request. Existing inspection guards still forbid prompts, previous sessions, write scopes and unreviewed options. Consumer boundary assertions cover taskless inspection, negative IDs and zero-owner inference refusal.
- **Prevention / Reference:** Metadata discovery must not manufacture a task or acquire inference/write authority simply to satisfy a shared schema.

### [2026-10-28] Strict progress IPC required a complete bridge revision

- **Context / Symptom:** Source integration found that adding a structured `progress` event while retaining bridge10 would admit older frontends which reject unknown events during an active turn.
- **Root Cause:** The sidecar event protocol is strict; additive JSON event shapes are not backward-compatible merely because existing fields remain unchanged.
- **Solution:** Advanced host admission, outgoing run schema and sidecar inspection together to bridge11, with older revision refusal covered before dispatch. Documented complete sidecar source/policy/dependency replacement; no compatibility alias or mid-turn fallback was added.
- **Prevention / Reference:** Check strict event consumers whenever adding a producer event, not only producer-side parsing.

### [2026-10-28] A negated review tail was treated as implementation intent

- **Context / Symptom:** The actual controlled progress preview for `Review the code changes without implementing a fix.` rendered Glaceon with Implement instead of a nonwriting review; evidence is `target/unassigned-progress-f/codex-120x36-en/en-explicit-review-guard-preview.txt`.
- **Root Cause:** Intent classification split `without` into an independent clause and interpreted the negated implementation verb as a positive request.
- **Solution:** Kept the positive clause before `without` and excluded its negated tail from action matching. Later punctuation-separated positive requests still participate. English/Korean regressions cover nonwriting review and positive refactoring/independent implementation; the actual progress journey retains the original negated review sentence.
- **Prevention / Reference:** Keep operation authority bound to affirmative user intent, never provider steps or isolated verbs inside negation.

### [2026-10-27] Unadvertised configured models appeared to have known capabilities

- **Context / Symptom:** The actual candidate-B unknown-model journey rendered `effort fixed by provider; not applied` as model-capability disclosure even though the controlled metadata response omitted that model.
- **Root Cause:** Native discovery appended configured/custom IDs to `catalogue.models` as synthetic entries. The common capability view could not distinguish those local configuration facts from advertised model metadata.
- **Solution:** Stopped injecting synthetic advertised rows, removed the now-obsolete discovery argument and migrated callers. Kept `catalogue.current`, exact configured selection and the Custom-model/effort flow. The native metadata regression covers all five dialects and verifies that omitted configured/custom IDs remain selectable without dispatch while capability disclosure stays unknown.
- **Prevention / Reference:** Preserve source provenance instead of branching on human-readable sentinel descriptions or treating configuration as provider acknowledgment.

### [2026-10-27] Command popup discarded todo command arguments

- **Context / Symptom:** The controlled progress journey reached adopted forms and revision/manual-mode checks, but `/todo-eager on` opened the bare toggle action without enabling the requested prelude.
- **Root Cause:** The existing command popup's argument allowlist omitted `todo`, `todo-edit` and `todo-eager`, so it recognized the command label while discarding its argument.
- **Solution:** Added these commands to the same bounded argument path used by existing commands. A real popup consumer regression covers enabling eager mode without preparing or sending a provider request; no parser, grant or dispatch guard was relaxed.
- **Prevention / Reference:** Exercise slash commands through the actual popup rather than only calling the reducer's argument handler directly.

### [2026-10-27] Native todo results could not continue the shared tool loop

- **Context / Symptom:** The corrected rich-model canonical journey emitted a todo proposal but no source read or second HTTP round. Actual TUI diagnostics reported `Tool result is not an actual structured local outcome`; later queued fixture responses then produced misleading transport failures.
- **Root Cause:** Todo success and refusal results omitted the required string `path` field from the existing shared local-outcome envelope.
- **Solution:** Added `path: ""` for this metadata-only tool to both success/refusal envelopes, preserving the existing status/message contract. Kept result parsing, approval, writer and nonmutation rules unchanged; a consumer regression feeds both outcomes through the real continuation validator. A new frozen candidate and complete native/feature qualification are required after this source change.
- **Prevention / Reference:** Qualify new tools through the actual continuation consumer and a real multi-tool request, not just event normalization or a single returned JSON value. Failed candidate-A reports remain retained.

### [2026-10-27] Older smoke callers skipped the newly common effort step

- **Context / Symptom:** The first frozen-candidate native profile passed catalog/artifact/source/conversation journeys but receipts/context refused an unknown popup and controlled Codex retained the old model. Captures showed the legitimate Reasoning level picker, not failed transport or missing file authority.
- **Root Cause:** Separate legacy model-selection helpers assumed unsupported/custom models finished immediately; the canonical journey now keeps the effort step visible even when only Default/auto is available.
- **Solution:** Updated the records reconnect helper and controlled model-replacement journey to explicitly accept the displayed effort step before workspace/session review. No model, policy, consent, timeout or expected filesystem effect was substituted. The same frozen candidate then passed all seven full-profile drivers/24 journeys in `target/unassigned-native-b`, including the repaired macOS context path.
- **Prevention / Reference:** Migrate every consumer of a changed UI transaction. Do not silently dismiss an unfamiliar sheet or remove exact next-request model assertions to make a smoke pass.

### [2026-10-27] Canonical request summary prevented compact dispatch

- **Context / Symptom:** The first integrated feature run compiled1,810 Rust tests but failed28; compact request consent never became reviewed, later permission fixtures still showed the generic local preview, and response landmarks were unavailable.
- **Root Cause:** The common summary consumed eight rows before the frozen Request was injected, exceeding the actual compact review body's eight-row capacity. Dispatch was correctly refused, so later provider events did not belong to active work.
- **Solution:** Fitted the canonical essential summary and frozen Request into the real eight-row body, retained full capability disclosures in `/journey` and exact details, and restored blocker-first presentation. Kept consent/exposure/scrolling guards unchanged. Unknown/custom model consumers now explicitly complete the common effort step; unchanged-model selection remains a no-op that cannot reuse armed consent.
- **Prevention / Reference:** Count dynamically injected scope as well as base summary rows. A capability disclosure must not crowd out effect-specific consent, the actual request or the actionable blocker. Final integrated/runtime outcomes are recorded in the checkpoint.

### [2026-10-27] Todo inspection queued unrelated specialist execution work

- **Context / Symptom:** Adoption returned no active label and the todo JSON editor did not open even after inspecting the proposed list. The next integrated run exposed Request/caret loss after cancelling that editor.
- **Root Cause:** `/todo` opened the complete specialist desk before establishing todo mode, queuing execution-history reads that made the existing busy guard refuse adoption/editing. It also retained/cleared the underlying Request even though the card owns a separate JSON editor.
- **Solution:** Established progress mode first and kept its card inert without initializing specialist evidence/history or clearing the underlying Request. Explicit specialist commands leave progress mode normally and still cannot bypass its editor. Retained current-revision/visible-inspection adoption and duplicate-label guards; regressions assert no unrelated execution work and exact Request/caret preservation after editor/card cancellation.
- **Prevention / Reference:** Read-only navigation must not manufacture work ownership or permission. Do not repair this class of bug by bypassing global busy guards.

### [2026-10-27] Git plumbing rejected Windows extended-length canonical paths

- **Context / Symptom:** Native run40484420922 passed Check/Linux/macOS-x86_64 but Windows failed16 selected-commit preparation tests because Git could not read the canonical `\\?\C:\…\project\.git\config` path.
- **Root Cause:** Native filesystem canonicalization and external Git path parsing have different accepted path spellings. The selected-commit runner passed the Windows extended-length spelling into Git arguments.
- **Solution:** Kept frozen canonical filesystem identities for containment/revalidation, but passed `.git`, `.`, `.git/config` and the isolated index relative to the already-owned frozen process cwd. No PATH lookup, inherited Git configuration or authority fallback was added. Added a real canonical Unicode-repository selected-commit regression.
- **Prevention / Reference:** Filesystem-valid paths are not automatically valid external-tool arguments. Final native Windows qualification, not a Unix pass or cross-compilation, owns Windows execution proof.

### [2026-10-27] Context smoke waited for idle output while a request was connecting

- **Context / Symptom:** The same run's macOS-arm64 `platform-tools-context` journey failed at Enter with `native console output did not settle within the bounded capture window`; retained captures showed an active Connecting spinner.
- **Root Cause:** The driver applied the idle-output capture contract to asynchronous dispatch, before the existing HTTP/answer/work-settlement predicates.
- **Solution:** Marked that explicit Send as asynchronous with `settle=False` while retaining exact HTTP continuation, rendered answer and released-work assertions. No retry, timeout increase, coverage skip or product consent change was introduced.
- **Prevention / Reference:** Preserve strict idle capture for actual idle surfaces; synchronize active work with its observable protocol/state completion rather than an absent animation frame.

### [2026-10-27] Local release tags retained pre-rewrite history

- **Context / Symptom:** The checkpoint described a pending public rewrite, but the remote main and release tags were already rewritten while28 local tags still referenced old commits. An all-refs scan therefore included removed reference traces even though the current source tree was clean. The checkpoint's original backup path was absent on this workstation.
- **Root Cause:** The prior rewrite/publication state and this local clone's tag refs had diverged; handoff prose alone did not establish current ref identity.
- **Solution:** Created and verified a fresh all-refs bundle before mutation, restored it into a disposable mirror, then force-fetched the already-rewritten public tags under explicit user authorization. All32 tag object identities matched the remote. Reconciled32 GitHub release targets to their rewritten peeled commits without replacing assets, scanned complete history/tag contents, and validated documented commit ancestry. Asset integrity evidence remains separate from ref identity.
- **Prevention / Reference:** Preserve external backups before tag synchronization. Rewriting refs does not erase unreachable local objects, GitHub caches, direct links or forks; optional server cache purge is not an asserted outcome.

### [2026-10-27] Compact selected-commit review rendered no scope rows

- **Context / Symptom:** The actual60×18 Git journey displayed a blank review with `Reviewed 0/87 rows` and correctly refused approval. Its120×36 counterpart completed the selected-only commit and drift/writer/restart cases.
- **Root Cause:** The renderer required56 inner columns, but the ordinary60-column terminal leaves54 columns after the existing sheet border/padding. Direct state/exposure tests did not exercise the actual root layout.
- **Solution:** Derived fit from the real fixed decision controls and the existing exact wrapped-row renderer instead of a guessed outer width. Kept the root60×18 minimum, contiguous whole-scope exposure, neutral choice, hidden/resize invalidation and one-shot reservation. A new root-renderer regression failed before the fix at60×18 and covers both supported sizes plus59×18/60×17 refusal; final source/runtime results are recorded in the checkpoint.
- **Prevention / Reference:** Render through the real root/panel hierarchy when testing minimum terminal dimensions; do not substitute an unpadded rectangle or weaken exposure to make an empty review eligible.

### [2026-10-27] Runtime drivers confused retained ownership and clipped disclosures

- **Context / Symptom:** Initial image journeys mistook the persistent image title for a send refusal, then expected a blocked frozen request's lease to disappear automatically. Compact prose checks failed across frame borders/clipped status rows. The Git writer-conflict driver waited for a detailed reason in the ordinary output view.
- **Root Cause:** The fixtures assumed incidental presentation and ownership behavior rather than using the existing refusal/cancellation/error-inspection flow.
- **Solution:** Observed the actual visible changed/deleted-image refusal prefix, asserted no transport, explicitly cancelled the blocked owner and then awaited settlement. Removed static disclosure-wording/route-copy assertions while preserving captured complete pages, strict complete-exposure eligibility and exact wire/model/receipt/byte assertions. Opened the advertised `/run-details` inspector for the exclusive-project reservation error. Product storage, consent, cancellation and ownership guards were unchanged.
- **Prevention / Reference:** Failed reports remain in `target/wave7-screenshots-{a,b,c}` and `target/wave7-selected-commit-{a,b}`; corrected image journeys passed in `target/wave7-screenshots-d`. A refusal probe is not authority to retry work or silently release a user's pending request.

### [2026-10-27] Selected commit preparation held Git's own HEAD transaction lock

- **Context / Symptom:** The first integrated Wave7 Rust run passed1,747 tests but failed15, including selected-commit success/partial-result cases with `cannot lock ref 'HEAD'` because `.git/HEAD.lock` already existed.
- **Root Cause:** Eevee reserved `HEAD.lock` itself, while `git update-ref` needs to acquire that same transaction lock even when publishing the reviewed named branch.
- **Solution:** Retained Eevee's exclusive index/configuration locks, left HEAD/ref transaction locks to Git, and kept final exact HEAD/ref/index/configuration/source/reflog revalidation plus the expected-old-object compare-and-swap. A deterministic external-HEAD-lock interference regression verifies refusal without removing or overwriting another writer's lock. The repaired integrated run passed1,763 tests with2 ignored.
- **Prevention / Reference:** Do not pre-acquire locks owned by the plumbing command being invoked. Lock acquisition is not a substitute for revalidating the frozen operation immediately before publication.

### [2026-10-27] Existing Git reflogs could alias unselected file contents

- **Context / Symptom:** In a disposable Git2.43.0 reproduction, an existing branch reflog hard-linked to an unselected dirty file grew that file from23 to167 bytes when `update-ref` succeeded, despite `core.logAllRefUpdates=false`.
- **Root Cause:** Disabling automatic reflog creation does not prevent appending an existing branch/HEAD reflog. Full-path source opens also did not provide the existing descriptor-relative ancestor containment guarantee.
- **Solution:** Froze and rechecked both writable reflogs and their native ancestors, refused linked/reparse/nonregular or multiply linked log files, and reused `repo::open_contained_node` for selected source/admin reads with same-handle identity/size/mode checks and safe reopen checks. Shared Git version admission now requires stable Git2.x>=2.36 before repository/index reads; older versions interpreted `core.fsmonitor=false` as a hook path.
- **Prevention / Reference:** Reflog and ancestor-drift regressions are included in the passing integrated Rust run. [Git configuration documentation](https://git-scm.com/docs/git-config) describes the older fsmonitor boolean incompatibility. The later security-review attempt was policy-blocked, not an approval.

### [2026-10-27] Clean Windows environment blocks terminated after their first variable

- **Context / Symptom:** A portable reproduction of the former Windows block builder found a double-NUL terminator after the first of three variables. The clean selected-commit child requires all frozen Git identity/configuration variables.
- **Root Cause:** Every entry received two NULs instead of one entry terminator followed by one final block terminator.
- **Solution:** Centralized direct UTF-16 environment encoding with one NUL per entry, a final extra NUL, and a double-NUL empty block. Preserved case-insensitive key ordering, allowed `=` in values rather than names, and kept clean launches from inheriting parent configuration. Portable encoding assertions and an actual owned Linux clean-environment child passed in the integrated Rust run.
- **Prevention / Reference:** Portable encoding and Linux process proof do not establish native Windows execution; Windows-only code is checked separately.

### [2026-10-27] Screenshot consent invalidation disturbed unrelated source work

- **Context / Symptom:** The first integrated run also failed deferred direct-source/optimizer journeys; compact partner-release consent no longer fit. Earlier integration inspection found screenshot route resolution performing filesystem work from the UI loop.
- **Root Cause:** Image-only obscuring reused broad source invalidation, source reset could enqueue a load for the prior producer, and the expanded release disclosure exceeded the compact surface.
- **Solution:** Moved screenshot route qualification into the existing context worker, made the UI's route memo pure, deferred owner changes across accepted image writes, and invalidated only image exposure/removal consent from image-only hooks. Main-loop owner activation owns screenshot loading. Condensed the complete partner-release scope while retaining whole-scope exposure and neutral approval. Existing consumer source/optimizer/release regressions passed in the repaired integrated run.
- **Prevention / Reference:** The Home composer explicitly displays the selected PNG/JPEG byte count. Driver refusal assertions must observe the actual changed/deleted-image error rather than mistaking that persistent attachment indicator for a refusal.

### [2026-10-27] Restoring a parked draft required an unnecessarily live Request editor

- **Context / Symptom:** After real writer contention/storage-only recovery, the tray durably contained all three drafts, but `r` did not restore the selected text from a suspended composer (`target/wave6-drafts-d`).
- **Root Cause:** Restore required `editing=true` even though the existing retained Request contained the exact text/caret needed for replacement review.
- **Solution:** Resolved the owned Request uniformly from the live editor or retained draft, binding both exact bytes and its own caret. Active non-Request editors remain protected. Neutral cancellation, stale retained caret and approved replacement have a regression; both real terminal sizes completed the entire fresh-authority Send journey in `target/wave6-drafts-final`.
- **Prevention / Reference:** Restore stages text, not a session or connection. The session-only connection was explicitly set up again after restart before the fixture's final Send; no credential restoration, timeout relaxation or automatic retry was introduced.

### [2026-10-27] Post-archive task picker still owned the restored Request editor

- **Context / Symptom:** The real Wave6 archive/restart journey displayed the task picker, but Home/Enter submitted the restored unsent Request instead of resuming its highlighted task. The failed quit then correctly retained an unknown provider outcome (`target/wave6-retained-d`).
- **Root Cause:** The management acknowledgment loaded another task with its retained editor active, then opened `Overlay::Tasks` without suspending that editor. Generic editing dispatch preceded task-picker handling.
- **Solution:** Retained the restored editor through the existing action-editor owner before presenting the picker. Compact/wide Managed→Home/Enter regressions preserve exact text/caret and acquire no provider intent, preflight or lease. The unmodified direct-picker journeys passed in `target/wave6-retained-g`; no quit guard or timeout was relaxed.
- **Prevention / Reference:** A visible overlay does not itself retire editor ownership. Exercise real acknowledgment-driven navigation, not only directly assigned overlay fixtures.

### [2026-10-27] External editor Ctrl+C killed the workbench's foreground process group

- **Context / Symptom:** Real disposable-editor cancellation terminated Eevee with SIGINT before it could restore the TUI (`target/wave6-editor-diagnosis-d`).
- **Root Cause:** Direct `Command::status` inherited Eevee's foreground group. Canonical terminal Ctrl+C reached both processes.
- **Solution:** Extracted the existing authentication `NativeSignals`/`NativeChild` owner into `platform::native_process` and reused its POSIX foreground-group handoff/restoration and Windows job admission; migrated authentication rather than duplicating a runner. `target/wave6-editor-p` passed both sizes with distinct editor/workbench groups, Ctrl+C, temporary cleanup and final termios restoration. `target/wave6-auth-c` passed both existing authentication fixture journeys, including listener cancellation and no model execution.
- **Prevention / Reference:** Terminal lending also requires native process/signal ownership. Windows code is compile-checked separately; Linux PTY proof is not native Windows execution.

### [2026-10-27] External editor review bound the revision before its own navigation save

- **Context / Symptom:** Opening `/external-edit` reported “Owner or draft changed while waiting” despite unchanged Request bytes/caret (`target/wave6-editor-b`).
- **Root Cause:** The command froze its revision before the generic event tail persisted the new editor surface. Its own navigation then invalidated the review.
- **Solution:** Acquired the navigation revision before binding the immutable owner/draft scope. The regression uses the actual command chooser and durable acknowledgments. Separate launch and returned-candidate consent remain exact; stale input still refuses.
- **Prevention / Reference:** Full scope exposure is not current readiness. A separate intermittent fixture stall was proven to be a late `records.inflight` load (`target/wave6-editor-predicate-trace-m.jsonl`); the driver now observes explicit Tab→`[Proceed]` eligibility before Enter, without bypassing the idle gate or replaying an editor launch.

### [2026-10-27] Parked-draft controls were blocked during work and retained hidden review state

- **Context / Symptom:** The actual running-request journey refused `/park-draft` with “Finish or cancel the active provider request first” (`target/wave6-drafts-b`). A full-scope regression also showed Ctrl+K retaining replacement consent after the review became obscured.
- **Root Cause:** The global busy-command allowlist omitted the new inert tray actions; direct feature-method tests bypassed it. The tray's exposure tracker did not invalidate on navigation or obscuring.
- **Solution:** Allowed only text parking/tray inspection through the existing busy dispatcher, retaining all provider/effect authority. Invalidated replacement/discard exposure on chooser/open/close/resize and refused hidden exposure accumulation. Both actual terminal sizes parked two drafts while the original request stayed owned, sent neither on completion/restart/owner changes, and retained exact text/caret after obscuring (`target/wave6-drafts-c`).
- **Prevention / Reference:** Drive the real command registry and rendered consent lifecycle. Keep deferred effects separate from unsent text; never solve this by scheduling or automatically sending the tray.

### [2026-10-26] Alias pre-filter missed case/normalization-aliased parents on macOS and Windows

- **Context / Symptom:** Native CI (run `40291871944`) failed `actual_case_aliases_are_rejected_without_capture_mutation_when_supported`, `workspace_alias_batches_never_gain_partial_authority` (macOS, Windows) and `actual_normalization_aliases_never_overwrite_or_manufacture_uncertain_effects` (macOS: `Reviewed absent parent appeared ... uncertain: true`). Linux passed because its byte-sensitive policy skips these cases.
- **Root Cause:** The quadratic-avoidance pre-filter in `verify_name_bindings` only compared paths whose folded keys were equal or a component prefix. The native walk also refuses pairs like `parent/a.txt` vs `PARENT/b.txt` or NFC vs NFD `é/…`, where an aliased *parent* is spelled differently but the leaves differ, so those pairs were never compared and admission accepted them.
- **Solution:** `alias_candidates` builds a resolved key per component prefix (existing directory object identity, otherwise NFC lowercase) and adds every pair whose resolved key is a prefix of the other, plus every pair whose equal resolved prefix is spelled differently. A Linux-runnable unit test covers case, normalization, prefix and same-object (short-name) cases. Separately, a summary test used `/tool path/tool`, which is not absolute on Windows.
- **Prevention / Reference:** Alias/normalization behavior is platform-dependent; pre-filters must be proved as supersets of the exact check with host-independent unit tests, and Linux-only local runs are insufficient. `cargo check --tests --target x86_64-pc-windows-gnu` compiles Windows-only code locally.

### [2026-10-26] Public native CI failures lacked accessible assertion details

- **Context / Symptom:** Native run40288713516 passed Check/Linux but failed Rust tests on macOS/Windows. Public check annotations exposed only exit101; raw job-log API returned403, and linked web step logs were unavailable without authentication.
- **Root Cause:** Repository SSH push access does not authorize GitHub Actions log APIs. The previous plain Cargo step did not include assertion details in public check annotations.
- **Solution:** Kept the exact locked/all-target Cargo invocation and original exit status, while emitting only a bounded escaped failure section as a check annotation. An actual isolated Cargo fixture verified failure exit101, percent/newline escaping, and passing exit0 without error annotations. No release gate was waived and no credential files were inspected.
- **Prevention / Reference:** Full console output remains in the normal job log; bounded annotations make native-only failures diagnosable through the ordinary public Checks API.

### [2026-10-26] Context smoke attempted connection setup before committed-input readiness

- **Context / Symptom:** The fixture waited for an API-key editor, but the retained frame explicitly refused setup with “Finish or retry task/context/journal saves and active work before connection setup.”
- **Root Cause:** The fixture had observed initial task/repository state rather than a fresh committed Request. The precise pending admission flag was not retained; a readiness race is an inference, not a proven internal subtype.
- **Solution:** Used explicit Ctrl+S and a newer durable-revision barrier before initial/restarted connection setup. Product admission guards, exact wire/provenance assertions and draft/caret checks remained unchanged. The repaired full native profile passed both context sizes.
- **Prevention / Reference:** Check the actual refusal frame before treating a missing expected editor as a navigation failure. `target/ux-release-native-{final,repaired}/platform-tools-context/` retains failed and passing evidence.

### [2026-10-26] Compact advisory rows hid a completed response

- **Context / Symptom:** The 60×18 receipt journey showed a completed response and its navigation controls, but not the actual answer (`target/ux-release-native-final/platform-tools-receipts/results.json`). The wide journey passed.
- **Root Cause:** Optional evolution/suggestion rows left five response rows. Request, header, completion notice and two navigation rows consumed all five before the body could render.
- **Solution:** Reserved six response rows before wrapping optional advisories. The regression combines same-form manual evolution, suggestion, completed response and an unchanged Unicode draft/caret at both supported smoke sizes.
- **Prevention / Reference:** Verify actual answer visibility, not just a “complete” badge. Integrated Rust verification passed1,679 tests; actual full-profile proof is tracked in the active checkpoint.

### [2026-10-26] Source-observation fixtures still assumed the retired mutation bound

- **Context / Symptom:** The source-read smoke waited for mutation refusal but instead reached an exact Modify approval. Retained HTTP evidence reported `bytes=162000` and `editable=true`.
- **Root Cause:** The fixture exceeded the old64-KiB bound, not the existing4-MiB bound. Some native preview/prompt/resource disclosures also still advertised the retired bounds.
- **Solution:** Kept the product's1024-file/4-MiB limits and corrected their disclosures. Made the Unicode/CRLF fixture larger than4MiB but below the16-MiB source-capture bound, retaining exact range/full-digest and no-mutation assertions. Added a separate explicit Plan request proving denial of an otherwise valid small-file write.
- **Prevention / Reference:** A large-file mutation refusal and a read-only-policy refusal are different boundaries. Preserve both rather than shrinking supported files to satisfy an obsolete fixture.

### [2026-10-26] Compact artifact review needed paging and failure cleanup hid the original assertion

- **Context / Symptom:** At60×18, the frozen purge revision was below the viewport; the driver timed out, then reported `loopback listener/worker cleanup exceeded deadline` instead of its original visibility failure.
- **Root Cause:** The fixture did not page the required review scope. Its custom peer callback remained blocked on a30-second gate after the12-second UI assertion failed, beyond the shared cleanup deadline.
- **Solution:** Paged to the actual frozen revision without weakening concurrent-capture isolation. Advertised PgUp/PgDn in artifact review controls. Added script-owned abort/release in `finally` so failed journeys unblock their callback without sending more tool work; shared cleanup bounds were unchanged.
- **Prevention / Reference:** Original failures remain under `target/ux-release-native-final/platform-tools-artifacts/60x18/`. Required complete-scope approval must remain enforced even when a review needs several pages.

### [2026-10-26] Local command form navigation bypassed its cancellation review

- **Context / Symptom:** The real execution journey started a root/child/grandchild process tree, then changing Jolteon to Vaporeon showed a generic owned-work refusal instead of the existing Stay / cancel-and-wait review (`target/ux-release-execution-after/results.json`).
- **Root Cause:** The form-navigation guard used aggregate `partner_busy`, which included the front local command itself and returned before `execution::State::request_navigation` could offer its neutral cancellation transition.
- **Solution:** Kept provider ownership, parked local work, command saves and unknown outcomes blocking, while allowing the front local command to reach its existing review. No command is cancelled or form changed merely by opening the review; affirmative navigation still waits for owned-tree and journal settlement.
- **Prevention / Reference:** `owned_local_command_form_change_reviews_cancellation_but_provider_work_stays_blocked` checks neutral no-effect and provider refusal. `scripts/execution_smoke.py` checks the actual process tree, explicit cancellation, task-switch retention and clean quit.

### [2026-10-26] Neutral command review stranded the request behind an execution inspector

- **Context / Symptom:** At 60×18, selecting `n` in a pending command review removed the confirmation but left “Approval pending” and “No output yet” instead of returning to the draft. Baseline captures: `target/approval-return-before/{before-neutral-return,after-neutral-return}.txt`.
- **Root Cause:** Neutral Enter/`n` cleared `confirmation` without clearing `shown`; Escape already used the complete deferral path.
- **Solution:** Reused `defer_approval` for all neutral return choices. The exact command remains pending, while the previous editor and Unicode draft/caret return without execution or an armed review. Restricted the Shift+A hint to eligible provider-owned command reviews rather than local commands/cancellation/purge.
- **Prevention / Reference:** `command_stay_returns_to_the_preserved_request_without_answering` covers both explicit and engaged automatic review. `scripts/execution_smoke.py` exercises the real return surface and subsequent command lifecycle.

### [2026-10-26] Empty provider search navigated to a hidden stale route

- **Context / Symptom:** Searching for `no-such-provider` showed “No matching route”, but Right/Left/Tab/Shift+Tab opened actions for a route that was not displayed. Actual 120×36 and 60×18 baseline captures are retained in `target/ux-release-nav-defects-before/`.
- **Root Cause:** Enter checked membership in the filtered provider rows; the other page-navigation keys trusted the old selected index.
- **Solution:** Applied the same visible-selection guard to every provider-page navigation key. Preserved the exact unsent draft/caret and required an actually displayed route before opening actions.
- **Prevention / Reference:** `empty_provider_search_cannot_open_actions_for_a_hidden_route`; actual before/after PTY navigation also checks command-argument guidance and no unexpected provider configuration writes.

### [2026-10-26] WSL release tooling lacked an authenticated GitHub CLI

- **Context / Symptom:** Linux `gh` was absent; the available Windows `gh.exe` reported no logged-in hosts. Configured rust-analyzer also exited with code0 on reference requests.
- **Root Cause:** This workstation had working repository SSH authentication, but no GitHub CLI API login, and the language-server process could not initialize.
- **Solution:** Verified the existing SSH identity and public API access. Source/tag pushes use SSH; qualification and public release inspection use the public API, while publication remains inside the existing credentialed GitHub Actions workflow. Reported the LSP tool failure and used scoped caller inspection, with integrated compiler/test verification required before release rather than claiming unavailable reference results.
- **Prevention / Reference:** Do not inspect credential files, bypass release qualification or confuse successful SSH authentication with an API token.

### [2026-10-26] Raising the write-target limit made every file operation quadratic

- **Context / Symptom:** After raising `MAX_FILES` from 32 to 1024, `thirty_two_nested_targets_are_admitted_but_excess_batch_has_no_effects` ran for more than five minutes instead of under a second.
- **Root Cause:** `WriteSet::verify` runs several times per operation and (1) re-read and rehashed every frozen target's full bytes and (2) `verify_name_bindings` compared every pair of target paths, allocating joined paths per component. Both were fine at 32 targets but O(n) bytes and O(n²) pairs per operation at 1024.
- **Solution:** Unrelated targets are revalidated by their captured file stamp (device/inode, size, modification and change times; Windows identity/size/write/change times); the operated file is still reread and hashed. Name-binding checks now group case-folded NFC component keys and run the exact native comparison only on pairs whose keys are equal or a component prefix of one another. The boundary test batches admissions rather than applying 1023 files one by one.
- **Prevention / Reference:** When raising a scope bound, time a test at the new bound in a debug build; look for per-operation loops over the whole scope.

### [2026-10-25] Scoped move inspection swallowed advertised arrow scrolling

- **Context / Symptom:** Complete wide PT12 journeys passed, but60×18 inspection initially showed only upper partner metadata; the mapped Glaceon move name remained below the fold. The footer advertised Up/Down scrolling while scoped input ignored those keys.
- **Root Cause:** Ordinary Records arrows select another partner. The scoped inspector correctly prevented that retargeting but only forwarded paging keys, leaving its advertised arrows without a safe scroll implementation.
- **Solution:** Added bounded line scrolling directly inside the frozen mapped-owner branch; ordinary owner-selection keys are never forwarded. Preserved PgUp/Dn/Home/End, invalidated any affirmative review flag on navigation and retained exact Escape return. Extended the existing read-only regression with arrow/end-boundary navigation and unchanged selected owner; RED observed scroll0 instead of3 (`bg_1237`). The actual compact driver now navigates to retained content rather than assuming every field fits its first viewport.
- **Prevention / Reference:** Verify the advertised navigation on the actual compact surface. Read-only means no selection/ownership mutation, not an inert or unreachable inspector. Final source/runtime proof is in the checkpoint.

### [2026-10-25] Settled native outcome uncertainty blocked local formation staging

- **Context / Symptom:** Full PT12 EN/KO writer journeys observed real socket EOF, finished peer workers, saved storage, empty active-work/tool-journal rows and retained exact dispatch attribution, but a fresh formation review still said staging was blocked by owned work. Both failed final staging in `wave5-formation-{en,ko}-wide-j`.
- **Root Cause:** Formation staging and mapped-owner badges used broad `outcome_unknown()` status as a work-ownership predicate. Existing native behavior intentionally retains unknown remote completion while permitting explicit follow-up/task navigation after local settlement; CLI and uncertain tool effects remain different, frozen ownership.
- **Solution:** Reused `State::owns_work()` alongside actual busy/preference settlement instead of clearing status or forcing a restart. Existing local execution, CLI/tool-effect, failed-journal and pending-receipt ownership remained guarded. Extended the existing correlated native-versus-CLI regression through canonical formation load/preview; it first failed `Busy != Eligible` (`artifact://2576`). Final GREEN/source and actual runtime evidence are recorded in the checkpoint.
- **Prevention / Reference:** Unknown provider completion/billing is not successful completion, but neither is it an owned local writer. Admission must follow frozen transport/effect ownership, never the currently selected provider or a display-status word.

### [2026-10-25] Tool-free native cancellation was mistaken for a missing tool journal

- **Context / Symptom:** PT12's real keyless Implement writer acquired the exact task/partner/request with `writing=1` and correctly refused formation Apply, but its cancellation fixture timed out after40seconds. The UI already reported local reader settlement with partial/unverified output and unknown provider outcome.
- **Root Cause:** The fixture incorrectly required a nonempty `provider_journal`. Native journal rows represent actual tool effects, not every transport dispatch; this peer intentionally emitted text only. The existing native smoke already required unchanged journals for such turns.
- **Solution:** Diagnostic `target/wave5-writer-cancel-diagnostic-a/diagnostic.json` observed real socket EOF, finished peer workers, no errors, `active_work=[]`, saved storage and an empty tool journal both at the first+0.002s sample and+40.009s. The exact dispatch receipt remained. Corrected the fixture to preserve its pre-submission tool journal and still require physical local EOF, durable ownership settlement, exact dispatch attribution and honest partial/unknown outcome. Saved post-cancel facts even on failure. No native product code, timeout or safety guard changed.
- **Prevention / Reference:** Dispatch evidence, model completion and tool-effect journals are different facts. Never synthesize a tool row, infer remote cancellation from local EOF or use a nonempty-journal assertion as proof of successful work.

### [2026-10-25] PTY prerequisites crossed browsing, role and deferred-review boundaries

- **Context / Symptom:** New Wave5 fixtures pasted a Request while browsing, attempted Jolteon's command editor from Eevee, assumed a retained prepared command survived denial, and waited for translated captions that the existing Sources surface did not use. Neutral Stay was also incorrectly treated as Deny. Product frames correctly retained the draft, exposed role/setup choices or deferred approval rather than executing.
- **Root Cause:** Standalone fixture actions bypassed the existing input ownership and actual workflow prerequisites. A changed task and a busy owner can coexist; asserting one specific blocking sentence incorrectly constrained precedence instead of testing refusal.
- **Solution:** Reused explicit Request focus, correct local role, complete paged move review and fresh exact command preparation. Used actual `/deny` only for denial, not neutral Stay. Captured post-admission baselines, checked exact leases/unchanged roster and drafts rather than a particular refusal phrase, returned through the existing workspace before another source choice, and steered Sources using actual loaded inventory/filtered target and the shared capture convention. Kept control-bearing local review separate from the safe frozen provider packet; native control-byte refusal remained intact.
- **Prevention / Reference:** Preserve failed reports, inspect the actual frame and follow established smoke helpers before changing product code. English/Korean title assumptions and incidental editor metadata must not replace consumer-visible bytes, ownership, effects and restart checks.

### [2026-10-25] Held renewal needed post-navigation revision binding and transactional replacement

- **Context / Symptom:** Wave5 source review found that normal Memory/Sources-to-Held navigation incremented task revision after renewal intent had frozen it; the actual command route would reject its own approval. A stale empty cached slot also used ordinary `SetHeld`, whose upsert could overwrite a different durable reference without old/new review. Retained/proposed source controls were rendered raw.
- **Root Cause:** Direct command-handler coverage omitted outer `App::update` navigation bookkeeping; replacement CAS protected only the explicit renewal operation, not the initial-assignment alternate path. The new surface had not reused Sources' escaped-content presentation.
- **Solution:** Recorded the staging event's navigation change before freezing intent, retained later-revision invalidation and waited for matching persistence without queuing approval. Made ordinary nonempty assignment create-only or identical-target idempotent; exact replacement uses expected-old-target CAS in the existing transaction. Escaped displayed CR/LF/ESC/directional controls without changing reviewed bytes or digests. Actual command-popup/update, occupied-slot writer and control-content regressions passed in the1,675-test source gate; real PTY acceptance remains a separate checkpoint gate.
- **Prevention / Reference:** Exercise consent through the real outer reducer and durable writer, not only direct helper calls. Never rebase stale approval, invent an old file body, auto-select the renewed target or alter a frozen request.

### [2026-10-25] Whole-snapshot inspector assertions included incidental editor chrome

- **Context / Symptom:** Two new formation warning-navigation tests failed whole-`Snapshot` equality despite preserved task facts. Complete output analysis showed the only difference in both failures was `navigation.editing: true → false` on entering an observational overlay (`artifact://2537`).
- **Root Cause:** A read-only inspector legitimately leaves the normal Request editor; a whole-row equality assertion incorrectly treated that presentation transition as task mutation.
- **Solution:** Deleted whole-snapshot assertions rather than changing or re-pinning editor behavior. Kept exact retained partner drafts/carets, lead, mapped selection, source/memory selection, no queued work and explicit return-to-formation checks. All1,675 tests then passed. The scoped Escape path explicitly restores its formation owner even when overlay history had already contained Agents/Records.
- **Prevention / Reference:** Assert consumer-visible state and ownership, not all serialized navigation metadata. Warning navigation is not formation application, route selection or permission.

### [2026-10-25] Cargo clean left a nonempty generated evidence tree

- **Context / Symptom:** After all public release jobs finished, requested cleanup of67G `target` returned `Directory not empty (os error66)`. A subsequent filesystem read/measurement showed882M of older generated evidence trees remained.
- **Root Cause:** Cargo's recursive cleanup did not remove the complete mixed build/evidence tree; the underlying filesystem trigger was not established. No new build or test was running.
- **Solution:** First archived and verified current release/runtime and critical failure proof outside target, preserving a runnable genuine public binary. Confirmed target was not a symlink and contained no tracked files, then removed only the residual repository target and verified it absent. Source and user `.DS_Store` were untouched.
- **Prevention / Reference:** Keep long-lived release evidence outside the build directory. The checkpoint records archive path/SHA256 and cleanup result; never delete evidence while runtime jobs still use it.

### [2026-10-25] Windows deadline fixture underflowed the monotonic clock

- **Context / Symptom:** Exact-source run40106372420 passed Check and four native rows, but Windows job120908793907 failed `terminal_deadline_requests_cancellation_without_unblocking_unsettled_effects` with `overflow when subtracting duration from instant` in the Rust standard library;1,584 other Windows tests passed. No0.1.31 tag or partial release was created.
- **Root Cause:** The fixture assigned `Instant::now() - LIFETIME` (15minutes), assuming every platform clock could represent that past instant. Windows counters near their origin cannot. Backdating cancellation by30seconds had the same assumption.
- **Solution:** Added private clock-injected polling/cancellation beneath unchanged production entry points; each poll samples one real monotonic time. The existing test advances from creation to just before/at lifetime and grace boundaries instead of subtracting from the platform origin. Lifetime/grace values, unknown-effect fail-stop, unsettled ownership and release refusal stayed intact. Final local gates passed1,640/two ignored, formatting, strict Clippy and release build; candidate `83019599dfd73cbfade013b5dd62179e49313a0ca2edd7fc3d13a641e2818212` passed real-Git zero-output and all six actual ACP modes. Fresh complete native qualification remains required; the failed run is unpromotable.
- **Verified completion:** Source `06ebff7` passed Check/all-five-native run40108914042, including Windows job120916314715; publication40110904023 promoted those packages. Downloaded public host `7a1e18db…` passed all six ACP modes and complete installed runtime acceptance. No failed-source artifact was promoted.
- **Prevention / Reference:** Advance an injected monotonic clock for long deadlines. Do not sleep, lengthen timeouts, skip Windows, or add checked-subtraction fallbacks that stop exercising expiry. Failure log: `artifact://2498`; metadata: `target/idle-patch-native-ci-failed{,-jobs}.json`.

### [2026-10-25] Incompatible-reader fixture used a compatible predecessor

- **Context / Symptom:** Passing public0.1.30 to `tool_plan_upgrade_smoke.py` for the0.1.31 UI patch timed out on `missing state: Storage: save failed`: the prior app correctly displayed `Storage: saved`. A separate roundtrip's whole-row equality then failed only on `revision` and `updated_at`.
- **Root Cause:** That fixture proves refusal by a reader lacking the newly strict plan field;0.1.30 already supports the unchanged patch format. Normal record admission is not the build-derived lifecycle rollback-identity check. Ordinary inspector navigation can save revision/time bookkeeping without losing task facts or replaying work.
- **Solution:** Kept the product reader unchanged. Used genuine0.1.29 for the incompatible-field refusal proof (`idle-patch-upgrade-029`, three passing cases) and a genuine0.1.30↔patch roundtrip for compatible reading (`idle-patch-compatible-030-final`, three passing cases). Deleted the incidental byte-for-byte task-row expectation; asserted every task fact, partner/run data and zero attempts, retaining before/after states and metadata deltas. Original failed reports remain unchanged.
- **Prevention / Reference:** Select the predecessor by the format boundary under test. Compatible app reading does not authorize bypassing the separate conservative offline rollback check or the full-backup requirement.

### [2026-10-25] Delayed record refresh repainted an unchanged idle terminal

- **Context / Symptom:** Public0.1.30 installation acceptance recorded20 passing lifecycle groups and13 passing terminal scenarios, then mono-ASCII100×30 emitted96 bytes/two cursor-only frames during idle despite identical before/after cells. The failed installation report was retained, not rerun into a pass.
- **Root Cause:** A delayed genuine repository observation invalidated the derived record cache. Enqueuing its load and consuming the reply both unconditionally requested repaint although no home content changed. Pairing restoration also returned readiness rather than a visible-change signal. Temporary mailbox diagnostics and `target/late-git-idle-before` reproduced the exact96-byte failure with real Git behind a controlled release gate.
- **Solution:** Separated hidden cache maintenance from visible invalidation, propagated correlated record-event change results through the existing store mailbox, and made pairing restoration report actual selection/badge changes. Habit/activity/held-item and open inspector changes retained repaint. Removed temporary production instrumentation. Candidate `8e664a08407c693dee7c2ab95d527a41ba207889949619ae27f479b65a8cd056` passed the identical delayed-Git case with zero bytes and all34 terminal scenarios; Rust1,640/two ignored, formatting, strict Clippy and release build passed. Fresh exact-source qualification/public installation still precede final acceptance;0.1.30 assets remain unchanged.
- **Verified completion:** Public0.1.31 host `7a1e18db…` passed the delayed genuine-Git case with zero bytes and all34 installed terminal scenarios, including56 zero-output idle windows. All24 install/update/uninstall/reinstall groups passed; the original failed0.1.30 report remained unchanged.
- **Prevention / Reference:** Keep strict zero-output idle acceptance and bounded settling unchanged. Sample delayed genuine background completion, not just a fast warm startup; state applied successfully is not by itself a reason to repaint.

### [2026-10-24] Release source omitted bilingual catalog inputs

- **Context / Symptom:** Exact-source native run40099449466 failed Check/all five native builds with38 `E0599` missing generated message/template variants, including `ToolPlanTitle`, `ImpactChecksCommand` and `DexComparisonUnavailable`.
- **Root Cause:** Commit `5040a70` included the Rust callsites but omitted six modified `locales/{en,ko}/{context_tools,desks,records}.ftl` inputs. Local builds used those working-tree catalogs, so their passing output did not qualify the incomplete committed source.
- **Solution:** Added the existing complete English/Korean catalog inputs to the release source without changing feature behavior or suppressing missing variants. Required a new exact-source Check/all-five-native qualification; the failed source was never tagged or published.
- **Prevention / Reference:** Treat build-time catalogs as source inputs. Clean native compilation is the regression proof; do not replace it with generated-file/source-text tests or waive a failed platform.

### [2026-10-24] Plan refusal reasons disappeared below long details

- **Context / Symptom:** A genuinely lost mutation response correctly remained `Unknown`, but the actual plan inspector did not show its refusal reason in the first viewport. Trusted detail separators also appeared as literal `\\n`.
- **Root Cause:** Notices followed the entire graph, and trusted multiline labels were passed through the same control-character escaping as untrusted field values.
- **Solution:** Put notices before graph details, reset scroll when entering a new preview/refusal and render separate trusted lines while escaping each untrusted value. Actual `wave34-tool-plans-c` completed all ten groups, including visible lost-response refusal and graceful unknown-outcome exit. Call metrics were then labelled recorded observations that may omit lost-response work; zero is never proof that nothing ran.
- **Prevention / Reference:** Check compact/wide actual surfaces, not only in-memory status. `wave34-read-plan-quit-f` separately exercised a held real read-plan preparation, explicit ALL-work cancellation/quit and zero-replay restart; its fixture intentionally opted into the existing guarded exit helper.

### [2026-10-24] SDK preview revalidation changed its own launch identity

- **Context / Symptom:** The actual host restored an untouched request with `Claude runtime/SDK/code/config/auth/project identity changed; preview again` before the controlled SDK produced any tool event.
- **Root Cause:** Each trust capture allocated a fresh global approval-arrival sequence inside the non-approval `runner::Prepared` launch identity. Full equality therefore rejected an otherwise unchanged runtime/package/config/auth/project.
- **Solution:** Constructed the launch identity deterministically without allocating approval order. Kept full stable-input equality and actual preview, tool and command approval ordering. The rebuilt host then completed the actual SDK write → real Rust compiler failure → reviewed correction → successful compile → native program output journey in `target/wave34-sdk-command-d`.
- **Prevention / Reference:** Identity snapshots contain stable observations, not UI sequence counters. SDK45 policy/real-MCP cases and a repeated-capture/changed-runtime regression cover the boundaries; this is local controlled-query proof, not authenticated model conformance.

### [2026-10-24] Canonical executable launch lost cargo/rustup multicall semantics

- **Context / Symptom:** The approved D01 `cargo test --locked --workspace ...` exited1 with `unexpected argument '--locked' found` and `Usage: rustup[EXE] <+toolchain>`, rather than running the caller test.
- **Root Cause:** Executable resolution correctly froze the canonical `rustup` image, but native spawning also used that canonical name as argv0. Rustup could no longer recognize the requested Cargo invocation.
- **Solution:** Kept the frozen image/SHA/cwd/source checks and supplied the reviewed specification's program as argv0. Unix used `CommandExt::arg0`; Windows retained the frozen application image and quoted the reviewed command-line token zero. No PATH re-resolution or new durable field was added. Actual English wide/compact D01 journeys then reached Cargo and the intended ignored caller test failed101 against the changed callee (`target/wave34-impact-en-c`).
- **Prevention / Reference:** Real-process regressions cover exact arguments, aliases, shell invocation and changed-image refusal. Freezing the executable image must not discard the requested invocation semantics.

### [2026-10-24] Owned host denial and non-Jolteon evidence blocked agent completion

- **Context / Symptom:** ACP `/deny` was disabled with `Finish or cancel the active provider request first`; otherwise successful compiler journeys could not quit because critical Moves storage rejected Flareon `ObserveRun` with `Move kind does not belong to this form`.
- **Root Cause:** The legacy manual-command guard did not recognize the current provider's exact owned callback. Separately, command evidence was validated as a selectable form-specific move definition instead of an observation of its actual producer.
- **Solution:** Routed only the exact current-owned pending callback through existing denial authority. Retained actual producer-form command facts and backfill independently of selectable-definition eligibility, without relabeling the owner or weakening definition validation. All six actual ACP happy/deny/cancel/kill/release/readonly journeys passed with graceful settlement in `target/wave34-acp-terminal-b`.
- **Prevention / Reference:** Keep command evidence, selectable definitions and execution authority separate. Regressions cover stale/foreign denial, actual producer attribution, reopened integrity and unchanged form restrictions; do not suppress storage failures or force an exit to claim success.

### [2026-10-24] A definite undispatched refusal was misclassified as lost-response unknown

- **Context / Symptom:** Changing source after exact command preparation started no process, but the durable plan did not settle as failed/no-effect.
- **Root Cause:** Plan reconciliation treated every `Interrupted` record as unknown, including the durable abandonment combination `NotStarted` with no start timestamp.
- **Solution:** Classified only that explicitly undispatched combination as failed/no-effect. Preserved original journal evidence, source binding and command-slot release; ownership-unknown/lost-response interruptions stayed fail-stop unknown. The actual stale-source case passed without a run, exit result or mutation in `target/wave34-tool-plans-b`.
- **Prevention / Reference:** A refusal before dispatch and a crash after possible effects require different certainty. Never infer no-effect from a missing response or replay confirmed commands.

### [2026-10-24] An idle learning offer interrupted an active empty Request

- **Context / Symptom:** The actual Dex recurrence journey lost its draft-entry interaction to a learned-move offer after returning to an owner with three eligible command observations.
- **Root Cause:** Idle-offer admission allowed an active Request editor whenever its current text was empty, including the interval while a user cleared and replaced a draft.
- **Solution:** Deferred automatic offers for every active editor, including an empty Request, while retaining explicit idle learning after the editor closed. Removed the now-unreachable editor-retention branch and kept the candidate untouched. Actual recurrence/relay acceptance then passed in `target/wave34-recurrence-en-b`.
- **Prevention / Reference:** Empty input is not absence of editing intent. A regression proves both uninterrupted empty-editor ownership and later reachable idle offers; no automatic learning, execution or grant was added.

### [2026-10-24] Structural observations lacked the native structured-result envelope

- **Context / Symptom:** Controlled Codex search worked, but native Responses stopped with `Tool result is not an actual structured local outcome`.
- **Root Cause:** New structural results omitted the shared `path` and `message` fields required by native result admission.
- **Solution:** Added truthful requested-directory scope and status-specific messages, including invalid/unsupported/incomplete outcomes, without weakening admission or coercing domain statuses. Actual native/controlled English wide and Korean compact search/cache/privacy/paging journeys passed in `target/wave34-structural-{en,ko}-d`.
- **Prevention / Reference:** Exercise the complete native continuation and advertised artifact retrieval, not only a local result builder. Existing grammar/range/status regressions remain; redundant envelope-copy tests were removed.

### [2026-10-24] Coding agents lacked a callable build route and classified build intent as readonly

- **Context / Symptom:** Priority agent create/edit/build/run work exposed a missing model-callable build route and ordinary build requests entering a nonwriting operation instead of Implement.
- **Root Cause:** File mutation tools alone did not provide command execution, and operation classification did not consistently recognize ordinary build/run/test intent. Provider-native shell availability was not an authorized substitute for the host's reviewed runner.
- **Solution:** Added Implement-only native HTTP/controlled Codex `eevee_run_command {cwd,command}`, Claude SDK host MCP `eevee_command/run_command` and ACP terminal operations through the same reviewed execution runner. Classified ordinary build/run/test intent as Implement while retaining explicit Plan/Review, readonly, quoted-source and negation boundaries. Commands retained project-relative cwd, typed Direct/Shell specifications and exact per-command approval; inherited environment remained unsanitized/unfrozen and no OS sandbox was claimed.
- **Prevention / Reference:** Actual native/Codex bilingual, production SDK-MCP and six ACP lifecycle journeys exercised real compiler/program effects with controlled peers. They did not establish live model understanding or billing. Nonzero/cancelled/unknown is not success; unknown effects retain writer ownership and never replay. Native/Codex advertised artifact tools retrieve retained output; SDK/ACP bounded output and workbench inspection do not invent unadvertised retrieval tools.

### [2026-10-24] SDK real-MCP command calls raced discovery and lost receipt ordering

- **Context / Symptom:** The actual SDK real-MCP integration reported `Tool run_command not found` when command execution reused discovery transport.
- **Root Cause:** Discovery and command serving shared transport lifetime/state, allowing a command call to race discovery ownership. Submitted and tool-result receipt ordering also needed to reflect the real command lifecycle rather than transport arrival alone.
- **Solution:** Used a distinct command-server MCP transport and corrected submitted/tool-result receipt ordering. Kept command execution behind production policy and exact approval; SDK-native Bash/Agent remained disabled and externally mediated writes retained their external-policy boundary.
- **Prevention / Reference:** Treat discovery completion, command submission and tool-result settlement as separate ownership events. SDK45 policy/real-MCP cases and `wave34-sdk-command-d` passed against production policy and the genuine installed SDK, with controlled query production and real approved compiler processes rather than an account/model call.

### [2026-10-24] Uncertain SDK post-write reconciliation reopened command authority

- **Context / Symptom:** Source review found that uncertain SDK post-write reconciliation could reopen command authority before an actual verified finish.
- **Root Cause:** Reconciliation uncertainty was treated as sufficient settlement instead of preserving the pessimistic state for potentially completed external effects.
- **Solution:** Kept uncertainty blocking until actual verified finish, without granting a new command, replaying the write or accepting uncertainty as success.
- **Prevention / Reference:** Reconciliation is evidence handling, not renewed consent. The production-policy cases exercised uncertainty fail-stop; the compiler journey exercised verified successful write reconciliation. Neither infers that an uncertain external write had no effect.

### [2026-10-24] Source reviews found freshness, effect-ownership and read-plan quit gaps

- **Context / Symptom:** Source reviews identified a source-freshness bypass, early writer release/continuation after unknown effects and a read-plan quit deadlock.
- **Root Cause:** Freshness enforcement, effect settlement and quit ownership were not consistently preserved across the reviewed execution and plan paths. A terminal-looking response or cancellation alone could not establish that all owned effects had settled.
- **Solution:** Repaired the source-freshness bypass, retained writer ownership and stopped continuation for unknown effects, and repaired read-plan quit settlement. Kept confirmed commands nonreplayable and required fresh reviewed inputs for eligible plan resume.
- **Prevention / Reference:** Actual plan recovery covered stale-source no-effect refusal, known mutation preservation and lost-response fail-stop; ACP covered deny/cancel/kill/release and ownership. Active read preparation separately passed explicit cancel/quit. Historical passing evidence never qualifies changed source, and controlled peers are not authenticated-provider conformance.

### [2026-10-24] Windows mandatory locks prevented complete snapshot reads

- **Context / Symptom:** Native run40081125036's Windows job120834732114 failed nine snapshot cases with `os error 33` and two ancestor-exchange fixtures with sharing violation32.
- **Root Cause:** Snapshot inventory reopened the exclusively locked writer-lock file instead of reading through its actual owning handle; Windows byte-range locks are mandatory for separate handles, unlike POSIX advisory locking. The live-owner observation helper had the same assumption. The two mutation fixtures tried to rename directories deliberately pinned without delete sharing, so Windows correctly blocked their setup attack. A subsequent read-only database `sync_all` was also nonportable: Windows flushing requires writable access.
- **Solution:** Inventory now reads every lock-file byte through the retained owner, rewinds borrowed reads for both passes and still revalidates the named native identity. A multi-buffer nonempty-lock regression preserves source/backup/restored bytes and consumes the restored store. Restore propagates SQLite close errors after `synchronous=FULL`, then uses the existing portable directory-sync convention instead of opening a read-only handle merely to flush. Mutation fixtures now prove actual Windows sharing denial and intact scope; POSIX still exchanges ancestors and proves stale-operation refusal. No lock bytes were omitted, ownership weakened or test skipped.
- **Prevention / Reference:** Local pinned gates passed1,589 cases/two ignored, strict Clippy/build and actual snapshot CLI/TUI acceptance. Repaired source `d9d3c62` passed [native run40082305882](https://github.com/kimmandoo/eevee/actions/runs/40082305882), including Windows tests/build/offline-install/runtime. The earlier failed run was cancelled as unpromotable rather than reusing partial success.

### [2026-10-24] Mutation acceptance confused automatic arrival, selected consent and durable settlement

- **Context / Symptom:** New controlled mutation journeys reported `file review did not reach its final visible detail page`, then encountered a null cancelled JSON-RPC result. Korean journeys also submitted an implementation-capable request after a fixture attempted to select Plan too early.
- **Root Cause:** Automatic permission arrival ignores scroll until explicit engagement. Selecting Right engaged the summary and selected Allow; a later Enter confirmed that choice rather than opening details. Cancellation correctly returned `result: null` with JSON-RPC cancellation code `-32800`. Separately, the durable write claim disappeared before the front-end consumed its terminal event: the captured Korean frame still showed `Receiving response`, `working` and the busy `/cancel` footer. The fixture's role command was refused while busy; it had not established an accepted Plan policy.
- **Solution:** Engaged the real summary before contiguous scope paging, used `d` for details instead of confirming a selected decision, handled null cancellation without treating it as success, and waited for the rendered nonbusy Request owner before changing role. Kept all actual file-byte, collision, stale-source/ancestor, authority and outcome assertions. All eight native/controlled English/Korean compact/wide journeys passed, including the actual nonwriting Plan transition; composed proof is in `target/wave12-mutation-verification.json`, with original failures retained.
- **Prevention / Reference:** SQL claim settlement is not proof that a rendered control accepts a new action. Use actual owner readiness; never compensate by adding read-only words to the request, removing authority assertions or automatically retrying a model call.

### [2026-10-24] A real version-probe fixture exceeded its unchanged deadline under default test concurrency

- **Context / Symptom:** `providers::agent::tests::consented_probe_keeps_only_a_recognized_version` reported `Version probe timed out after 3 seconds; no version accepted` during the complete default-concurrency run.
- **Root Cause:** The real-process fixture crossed its wall-clock deadline under that run's scheduling/load; the exact scheduler cause was not established. It was not a failing version-parser assertion.
- **Solution:** Kept the production three-second deadline and all assertions. Ran the complete suite with bounded local test concurrency: `cargo +1.90.0 test --locked --all-targets --all-features -- --test-threads=4`; all1,588 cases passed with two existing ignored fixtures. No test was retried inside the suite, skipped or marked successful on timeout.
- **Prevention / Reference:** Report the actual concurrency used. This local verification command did not change CI configuration or claim that default-concurrency behavior was repaired.

### [2026-10-24] Disposable installation and terminal packages omitted their local dependency

- **Context / Symptom:** Public v0.1.29 verification initially failed to build its real older-version upgrade fixture because `upgrade-source-*/vendor/ratatui-core/Cargo.toml` was missing. After that fixture was repaired, installation and live self-update passed, but the isolated terminal lifecycle probe reported the same missing dependency.
- **Root Cause:** Both temporary packages copied the root Cargo manifest and lockfile without its new pinned path dependency. The terminal probe used selected platform sources, but still inherited the complete root manifest.
- **Solution:** Copied the tracked `vendor` tree into both disposable packages and selected CI-pinned Rust1.90.0. Kept the real older-version executable and real terminal lifecycle probe; published binaries/installers were unchanged. Retained successful installation/update evidence and continued terminal/uninstall acceptance separately rather than rebuilding the already-proven older-version fixture.
- **Prevention / Reference:** Audit every temporary package that copies a manifest, including minimal probes, for local dependencies. Reports distinguish the original failed run from its subsequent continuation; no failed report is rewritten as a passing run.

### [2026-10-24] First-save task identity reset an already classified implementation request

- **Context / Symptom:** Run40066393690's Linux compatible/Responses Korean compact journey selected Flareon but dispatched a nonwriting lease and read-only tools for `Generate three project files`. A deterministic regression reproduced frozen `Plan` input where `Implement` was required. CandidateJ also reproduced the nonwriting lease in the actual Korean compact UI with a rapid first paste/Enter (`target/first-save-before-rapid`).
- **Root Cause:** The first durable save assigned the existing local task an ID, but its planner retained task0. Subsequent preparation treated that identity assignment as a task switch and reset the selected operation to its default.
- **Solution:** Adopted the first durable identity in current and parked initial planner states without resetting capability, explicit Plan/Review policy or local selection. Actual task switches retain the existing reset boundary. The frozen-input regression passed for ordinary implementation and both explicit read-only policies. CandidateK passed pinned Rust1.90.0 formatting,1,555 tests/two ignored, strict Clippy, release build and actionlint. All eight rapid first-save native journeys passed across English/Korean, both sizes and both compatible protocols, with real reviewed exact-byte file creation (`target/first-save-after`).
- **Prevention / Reference:** Persistence acknowledgment is not a new task. Do not compensate with an explicit implementation command in the fixture; ordinary first submission must retain the user's classified intent.

### [2026-10-24] Artifact fixture bypassed explicit writer unlock and early failure broadened evidence scanning

- **Context / Symptom:** The same run's macOS arm64 artifact reopen test failed with `Cannot acquire the writer lock ... operation would block`. The subsequent evidence uploader reported `EACCES ... /Library/Application Support/Apple/AssetCache/Data`.
- **Root Cause:** Fixture shutdown took/dropped the file descriptor directly instead of following production's explicit unlock, allowing another test's concurrent fork to retain the open-file lock until exec. Separately, the output-directory environment was initialized after tests; early failure interpolated an empty value into `/`.
- **Solution:** Explicitly unlocked after closing SQLite in fixture reopen/cleanup, retaining the real ten-MiB exact paging/search/reopen test. Conditioned evidence upload on a nonempty initialized output namespace; selected runtime failures still retain their complete evidence and reject qualification. No retry, serialized test suite or timeout increase was added.
- **Prevention / Reference:** Keep fixtures consistent with `State::drop` and never append `/` to an unchecked optional artifact path. Local qualification uses pinned Rust1.90.0; README development commands now select it explicitly rather than the host's newer lint set.

### [2026-10-24] Concurrent smoke drivers selected commands before their rendered owner settled

- **Context / Symptom:** Optimized run40064426753 passed Check and four native targets but the Linux full profile failed two journeys. Controlled Korean setup left literal `provider-codex` in Request instead of selecting the provider. The catalog refusal journey stayed in `/approvals` search with `The workspace changed while commands were open`.
- **Root Cause:** The controlled setup treated a committed nickname database row as proof that the editor's asynchronous completion had reached the UI. The catalog driver opened a scoped chooser immediately after asynchronous submission, before its pending review became the rendered owner. Bounded concurrency exposed both preexisting fixture synchronization races; the product correctly refused stale chooser scope.
- **Solution:** Returned to the rendered workspace after nickname commit, before provider selection. Waited for the catalog's actual review hint/refusal and used direct review focus instead of racing another chooser. Kept empty-request, exact refusal, zero-network and ordinary follow-up assertions; did not add retries, sleeps, larger timeouts or product permission bypasses.
- **Prevention / Reference:** Database persistence and rendered interaction ownership are distinct readiness boundaries. Evidence: `target/ci-optimized-linux-failure`; repaired concurrent runtime proof passed all six journeys in241.403s (`target/ci-scoped-ownership-local`) and Python22/four skips passed. A fresh complete exact-source native run remains required before publication.

### [2026-10-24] Repeated terminal cross-products dominated native CI

- **Context / Symptom:** The user reported excessive test volume and slow CI. Completed run40054403499 took70m52s on macOS x86; file journeys consumed21m09s and context/tools38m11s, compared with8m40s for Rust tests plus release compilation. Windows failed, so its duration is not a successful full-profile baseline.
- **Root Cause:** Each of five native rows serially repeated shared protocol/language/size cross-products. Controlled file details also sent48 PageDown events after the final visible page, and optimizer input paid for two consecutive output-settling windows.
- **Solution:** Assigned exhaustive shared coverage to Linux x86, preserved explicit Windows/macOS rendering coverage and representative bilingual native file coverage on every architecture, and retained all five native unit/build/install/package gates. A bounded two-driver runner records selected coverage, durations and file-backed logs and fails if any selected child fails, times out or changes the binary. Replaced fixed paging with sequential render-until-stable navigation and retained separate summary/consent checks; optimizer input now uses one unchanged bounded settled send. The matrix selected100 journeys instead of236. Python22/four skips and actionlint passed; real local macOS-profile16 journeys passed in364.717s and all four controlled Codex paging journeys passed. Fresh CI timing and all-platform success remain required.
- **Prevention / Reference:** Inspect `coverage-manifest.json`, `results.json` and `logs/` in native evidence. Compare measured step/driver durations before deleting useful unit regressions or shortening readiness deadlines. Local smoke time is not a claimed hosted-CI duration; the timeout still fails rather than reducing scope or retrying into success.

### [2026-10-24] Goal-only Codex sessions poisoned the next ordinary implementation request

- **Context / Symptom:** With a saved goal and empty Request, `/session-new` completed successfully but the next ordinary implementation request showed `Public implementation context lost its bounded task/partner/provider ownership`. CandidateH reproduced the refusal at both terminal sizes (`target/recent-review-before-goal-only`).
- **Root Cause:** The completed-public-context producer stored an empty user turn beside the answer, while the next-request validator correctly required nonempty bounded public request/answer pairs.
- **Solution:** Omitted only the invalid goal-only history pair; retained its visible public answer and kept strict validation, without inventing an instruction from the goal. Added a completion-to-follow-up regression and exercised actual controlled Codex goal-only completion followed by ordinary deny/allow/read/edit/model-change flows. CandidateI passed eight bilingual native/controlled conversation journeys44 groups (`target/recent-review-i-conversation`).
- **Prevention / Reference:** Producers must satisfy the same ownership/content contract as their consumers. Goal-only advanced sessions are not equivalent to an explicitly submitted Request.

### [2026-10-24] Artifact navigation and deletion review lost immutable selection ownership

- **Context / Symptom:** Recent-change review found that paging a 10-MiB artifact, returning to the list, selecting a smaller artifact and pressing `n` reused the first artifact's byte offset. The unchanged candidateH reproduced `Artifact range requires an existing byte offset and 1–65536 bytes` at both sizes (`target/recent-review-before-artifacts`). A concurrent new capture could also change the row rendered by a pending purge review while confirmation retained the former handle.
- **Root Cause:** Pagination/history and UI response presentation were task-scoped but not handle/view/intent-scoped. New rows selected index zero, while purge rendering consulted that mutable selection instead of its frozen handle.
- **Solution:** Bound cursors to handle/view, correlated presentation to the latest explicit inspection, revoked late UI presentation on Escape/reopen, preserved selection by handle and rendered/validated purge against its own immutable identity. Added reducer and bilingual compact/wide rendering regressions. CandidateH also reproduced actual concurrent-capture purge-scope drift at both sizes (`target/recent-review-before-purge-race`). CandidateJ passed all four actual artifact journeys24 groups, including concurrent capture during a frozen purge review, cross-handle, Details, same-task reopen, Locations isolation and truthful view-specific control hints (`target/recent-review-j-artifacts`).
- **Prevention / Reference:** Durable capture acknowledgments and provider retrieval replies remain independent of cancelled UI intent. Keep displayed destructive scope identical to the dispatched operation even while new records arrive.

### [2026-10-24] Neutral wide glyphs left orphan cells in the native Windows console

- **Context / Symptom:** Compact Korean model/receipt/conversation screens retained old glyph halves and corrupted ASCII identifiers. Replaying retained ConPTY bytes in both pyte and independent xterm6 reproduced the corruption. Run40054403499's paired `ReadConsoleOutputW` evidence contained50 orphan trailing `CHAR_INFO` cells before VT serialization (`target/native-cjk-paired-proof.json`).
- **Root Cause:** Ratatui core0.1.2 refreshed uncovered trailing cells only when the former wide glyph had a visible blank-cell style. Windows retained neutral glyph trailing halves too. A shifted new wide glyph could also cover another old glyph's lead while leaving its far half untouched.
- **Solution:** Vendored the exact pinned core with its MIT license/provenance and patched only the diff iterator: refresh uncovered old footprints, follow shifted overlaps, bound them to their row and never overwrite a new wide glyph's own trailing columns. The iterator remains allocation-free; unchanged frames emit no updates. A native-cell behavior regression covered81 narrow/wide/shifted transitions; integrated1,554 tests/two ignored, strict Clippy and release build passed. Run40060136588 on `5139a48` passed the entire Windows job, including all bilingual file/context/tool journeys and resize/purge scenarios. All266 paired native snapshots had zero orphan halves and no unavailable captures; exact compact model IDs remained visible (`target/native-cjk-fixed-proof.json`). Its superseded macOS-x86 job was cancelled for the CI revision, so a fresh complete same-source run remains required for publication.
- **Prevention / Reference:** Compare actual native cells with serialized terminal bytes before blaming a capture parser. Do not hide stale cells by filtering output, weakening visible identifier assertions or clearing every frame.

### [2026-10-24] Text-mode fixture creation changed exact tool-read bytes on Windows

- **Context / Symptom:** Run40040530847 passed Check and all four macOS/Linux native jobs. Windows passed bilingual files, English receipts and both English context sizes, then catalog acceptance reported `tool did not execute the actual owned source read`.
- **Root Cause:** Retained `en-60x18/http.json` proved the selected tool succeeded and returned the exact source with `\r\n` and its matching digest. The fixture wrote `SOURCE` through default text-mode newline translation but compared the returned content/hash against LF-only UTF-8 bytes. The provider/executor had not changed or lost the file.
- **Solution:** Wrote explicit UTF-8 fixture bytes and checked unchanged saved bytes with `read_bytes`, retaining strict tool status, exact content and exact SHA256 assertions without normalizing returned data. Continued every independent native context/tool journey after a failure while accumulating a nonzero overall exit status; this preserves all failure evidence without waiving any gate or retrying work.
- **Prevention / Reference:** Use byte-explicit producers for exact-wire fixtures. The complete fresh Check/all-five-native run still must pass before tagging; four successful target jobs do not authorize publication.

### [2026-10-24] Reused compiler output directories and Windows path spellings blocked native evidence

- **Context / Symptom:** Run40031296268 passed Check and both complete macOS jobs. Both Linux file drivers stopped at `FileExistsError` for `target/platform-files-en` before creating evidence. Windows passed all unit checks, bilingual file workflows and English receipts, then timed out waiting for an already committed owned-run activity.
- **Root Cause:** Fresh-only acceptance outputs lived inside the compiler-cache target tree; the next Linux run found restored empty directories. The Windows timeout snapshot contained the exact expected event and owner under the task's canonical `\\?\C:\...` project, while the fixture filtered with a differently spelled path. The catalog's SQL lookup shared that same spelling assumption.
- **Solution:** Moved every native journey's output to an attempt-specific runner temporary namespace outside compiler caches and retained upload of all evidence. Kept fresh-only directory creation. Compared fixture project directories with `Path.samefile`, including catalog lookup, preserving exact project ownership rather than accepting all rows or dropping the activity/catalog assertions.
- **Prevention / Reference:** `041-timeout-sqlite.json` under `target/native-029-second-windows-failure` proves the committed event/task/run identities. Keep executable/package provenance, complete bilingual criteria and fresh exact-source qualification mandatory; a passing earlier row is not reusable release authorization.

### [2026-10-24] Expanded native qualification exposed platform-specific fixtures and job exhaustion

- **Context / Symptom:** Native run40022572931 passed both Linux targets but failed Check on two `needless_borrow` diagnostics, Windows on five unit cases, and both macOS jobs at the 60-minute limit while still progressing through mandatory bilingual context/tool journeys.
- **Root Cause:** macOS normalization used an owned string while other platforms used a borrowed string, making the same argument borrow redundant only off macOS. A Windows test still treated every >64-KiB file as unreadable after observation was separated from mutation. Four ledger tests used `/offline/project`, which is not a volume-qualified Windows absolute root. The enlarged serial native acceptance workload exceeded the prior macOS job ceiling; no runtime assertion failure was observed in those timed-out rows.
- **Solution:** Used a uniform borrowed normalized path while allocating only on macOS; checked the unchanged mutation-mode bound rather than the obsolete observational refusal; constructed ledger fixtures from the native temporary root. Raised the complete native job ceiling to120 minutes without removing, weakening or bypassing any acceptance step. CandidateH passed1,544 Rust tests/two ignored, formatting/strict Clippy/release and12 bilingual native/Codex/ACP source-observation journeys. Fresh exact-source native qualification remains required before publication.
- **Prevention / Reference:** Keep platform-valid fixtures, distinguish observational policy from mutation authority, and size the CI envelope for the complete mandatory workload. Linux package success never authorizes a partial-platform release.

### [2026-10-24] Recovery driver confused explicit lead stop with all-owner confirmation

- **Context / Symptom:** The locked-journal journey at39×10 expected `/cancel` to leave its owned process alive until a later confirmation; the process stopped immediately.
- **Root Cause:** `/cancel` deliberately stops only the current lead, including below minimum terminal size. The driver retained the older execution-popup confirmation sequence. Separately, inspection found that legacy execution confirmation selection and activation were not consistently gated on visible scope.
- **Solution:** Exercised the explicit lead-only stop while SQLite remained locked, retaining the terminal-result preservation and storage-only retry assertions. Required visible/shown scope for all affirmative execution confirmations and ignored hidden affirmative selection keys; transition regressions require a fresh visible choice.
- **Prevention / Reference:** Distinguish a safe explicit current-lead stop from neutral all-owner cancellation/quit and approval of new work. A confirmation object alone does not prove that its complete scope rendered.

### [2026-10-24] Optimizer cancellation comparison used a consumed Request

- **Context / Symptom:** The runtime optimizer driver reported different selected packets after cancelling a later preview. The first wire proof contained the original Request; both post-send SQLite snapshots showed that Request had already been consumed and cleared.
- **Root Cause:** The next preview used different mandatory instruction facts. Source planning correctly invalidated the old applied packet before cancellation; the driver compared unlike preconditions.
- **Solution:** Restored the exact Request, explicitly reviewed/reapplied the original plan and matched its real cache packet SHA to the first outgoing packet before previewing/cancelling the alternative. Kept strict outgoing byte/value equality and draft/pin preservation assertions. Made fixture request entry idempotent to avoid artificial clear/retype invalidation, and used ordinary Home caret navigation to release restored attachment-chip ownership before sending.
- **Prevention / Reference:** Inspect the committed task and actual wire bytes around each transition; a previous successful send is not evidence that its consumed request still exists.

### [2026-10-24] Provider evidence reused local attribution and command approval metadata

- **Context / Symptom:** Actual artifact-list round trips on candidateD exposed companion names/forms at both sizes (`target/ten-d-artifact-owner-red`). The first complete wide compaction send also exposed full `prepared.partner`, `approval_arrival` and expiry metadata (`target/ten-e5-context-en/120x36/applied-evidence-linked-compaction-wire-proof.json`).
- **Root Cause:** Artifact and run provider responses reused complete immutable local records, mixing UI attribution and dispatch preparation with historical evidence.
- **Solution:** Allowlisted artifact metadata with numeric ownership and added a shared borrowed historical-run projection for compaction and independent review. Kept command/outcome/retention/input identities, excluded local display and approval material, and preserved full local records. Added privacy/control-invariance regressions and actual outgoing-wire checks.
- **Prevention / Reference:** Tool results and compacted history belong to the provider-packet boundary just as initial task context does; stored evidence is not automatically a safe wire schema.

### [2026-10-24] Compact optimizer hid mandatory overflow below verbose metrics

- **Context / Symptom:** At60×18 the optimizer showed a previous applied-selection notice and token metrics, but the current refused preview was below the initial viewport. The footer still advertised apply.
- **Root Cause:** Verbose accounting preceded the current refusal, while one status line did not distinguish an unapplied preview from the prior applied packet.
- **Solution:** Prioritized current overflow/refusal in the fixed status and first summary rows, distinguished prior selection, and replaced the overflow footer with budget/cancel/details and blocked-apply guidance. Kept full metrics scrollable and the same no-effect apply guard. Runtime acceptance checks the initial compact screen.
- **Prevention / Reference:** Inspect supported-size surfaces for the current decision, not only whether its text exists somewhere in the scroll buffer.

### [2026-10-24] Canonical temporary project identity hid an already committed run activity

- **Context / Symptom:** The actual failed command exited1 and its exact activity row existed, but the context driver timed out waiting for that activity.
- **Root Cause:** Its temporary project used the macOS `/var` alias while the committed project identity used `/private/var`; inherited row filtering compared their strings.
- **Solution:** Canonicalized the fixture root before constructing the journey, following the existing fixture convention. Kept exact task/run/owner and activity checks rather than repinning IDs or weakening ownership.
- **Prevention / Reference:** The wide E5 journey then reached the complete compaction send. Resolve fixture identity once at construction, not separately at each assertion.

### [2026-10-24] Rejected ranked candidates produced duplicate cache revisions

- **Context / Symptom:** The real attachment/memory/tool-result measurement reached `Duplicate context revision key` while atomically saving optimizer revisions (`target/ten-d3-context-en`).
- **Root Cause:** Optional definition/signature/reference candidates counted an exact range before budget acceptance. Rejection removed its overlap witness, allowing another role to revisit that range and append the same revision key again. The store correctly refused the duplicate batch.
- **Solution:** Reused same-turn counts through an owned-key map and indexed persisted rows by borrowed keys; persisted each identity once without counting same-turn reuse as a persisted hit. Kept same-key/different-digest conflict refusal and the atomic store guard. Added budget-rejection, distinct-file identity, conflict/accounting and Sources-worker/store/UI regressions.
- **Prevention / Reference:** Exercise real selection-to-persistence transitions with multiple syntax roles, not only standalone tokenizer/cache lookups.

### [2026-10-24] Large optimizer manifests and owned logs exceeded inherited persistence constraints

- **Context / Symptom:** The real multi-file optimizer preview showed valid reduced packets and persisted token rows, but no warm assembly manifest. Actual 10-MiB owned output could not start despite a configured 16-MiB retention budget. Compact artifact inspection initially displayed verbose IDs instead of the retained page.
- **Root Cause:** Assembly JSON was embedded in a primary key and silently skipped above64KiB. The execution table still enforced its legacy8MiB reservation check. Long request/handle/revision fields consumed the minimum-size inspector.
- **Solution:** Added a strict nullable assembly metadata column with stable hash keys, atomic persistence and actual UTF-8 quota accounting; removed the silent skip. Rebuilt execution reservations transactionally for16MiB, preserving rowids/data/other constraints and refusing unexpected dependent objects. Prioritized page/content and counts, with full identity available through `i` and unchanged complete-scope purge.
- **Prevention / Reference:** Keep genuine historical8MiB fixtures and predecessor-binary upgrades. Exercise a >64KiB manifest through save/restart/warm reuse, full10MiB output/search/page, and a fresh64KiB-budget run whose retained/discarded counts reconcile exactly.

### [2026-10-24] Busy receipt recovery and execution diagnostics were unreachable through their advertised commands

- **Context / Symptom:** A real rolled-back outgoing receipt left work settlement pending, but `/receipts` was disabled as active provider work. A reservation failure advertised `/artifacts`, which now opened the generic tool inspector instead of the command error.
- **Root Cause:** New blocking receipt state joined the provider busy predicate without adding its safe recovery navigation. A legacy execution detail action collided with the new global artifact command.
- **Solution:** Allowed safe receipt/artifact/diagnostic inspection while busy, preserving each operation's own consent and ownership checks. Cut the old execution action over to global `/run-details` and migrated its banners, help, locales and recovery driver; no compatibility alias remains.
- **Prevention / Reference:** Exercise recovery through the actual global chooser under real storage pressure. Inspecting evidence must not require the very settlement it is intended to unblock.

### [2026-10-24] Actual context/tool terminals exposed palette and literal-editor routing gaps

- **Context / Symptom:** `/tool-setup <path>` remained in an empty command palette; artifact `/` search opened global commands instead of its literal query. Backspace and slash inside the new catalog/artifact editors could reach global form/navigation shortcuts.
- **Root Cause:** App-level command calls bypassed the palette's separate literal-argument admission list. New overlay editors had not joined the shared text-ownership guard, and their search shortcut ran after global slash dispatch.
- **Solution:** Routed supported optimizer/catalog arguments through the canonical command path and its existing disabled-action guards. Gave artifact/catalog search entry and active literal editors their own key ownership without making paste a grant.
- **Prevention / Reference:** Real terminal drivers must enter commands through the actual chooser, not call App methods as their only proof. Retain full draft/caret and no-dispatch assertions, plus the App-router search/paste/Backspace regression.

### [2026-10-24] Smoke runtime version and consent-state assumptions obscured new acceptance

- **Context / Symptom:** macOS's default Python3.9 failed `hashlib.file_digest` and `zip(..., strict=True)` in verification scripts. A receipt fixture compared a stale durable cursor while autosave was still pending. Other journeys waited behind a visible evolution receipt, treated Escape as cancellation, omitted explicit transport-diagnostic acknowledgment, or attempted an unknown-outcome quit without its required all-owner consent.
- **Root Cause:** The verification environment differed from pinned CI Python3.12. Fixtures conflated local terminal state, delayed committed drafts, review navigation and authoritative cancellation/settlement.
- **Solution:** Used installed Python3.12 with the already installed pure-Python pinned terminal dependencies, without changing product runtime prerequisites. Awaited exact committed text/caret before receipt baselines. Dismissed only identified transient views, focused the actual pending review, used explicit cancellation/recovery and opted into the existing all-owner quit review after an intentionally incomplete stream.
- **Prevention / Reference:** Run verification with Python3.12 and `pyte==0.8.2`, `wcwidth==0.2.13`, as in native CI. Preserve first-failure terminal and database captures; subsequent exact wire, draft, no-network and claim assertions remain required.

### [2026-10-24] Direct optimizer commands queued unrelated catalogue work

- **Context / Symptom:** App-level coverage found `/optimize` and `/compact` opened Sources, queued its ordinary catalogue request and then rejected their own requested operation as busy. Keyboard evidence regressions initially also failed behind catalogue work their fixtures had never submitted.
- **Root Cause:** Direct context actions reused a navigation path whose browsing side effect was inappropriate for a frozen optimizer request. Fixtures confused an unsent catalogue with settled Sources state, and one attempted to reopen an already-open overlay, which intentionally performs no new browsing work.
- **Solution:** Shared Sources navigation while letting direct context actions omit catalogue creation. Preserved existing pending work instead of clearing it. Settled real worker catalogue requests in keyboard fixtures and used an actual close/reopen before asserting ordinary browsing behavior.
- **Prevention / Reference:** App coverage asserts the requested operation, exact applied packet, unchanged draft/basket and preservation of already-pending work. The actual optimizer terminal driver exercises both direct commands separately.

### [2026-10-24] Context/tool integration exposed stale evidence and unsaved-receipt settlement hazards

- **Context / Symptom:** Review found compaction could include currently excluded historical captures, keyboard compaction could use stale or unhydrated run history, artifact purge could leave cached receipt source text, and an unrelated provider failure could revoke the compatible tool catalog. Receipt storage could also reach its per-task limit after transport or leave a definitively rolled-back write blocking completed work indefinitely.
- **Root Cause:** Historical source identity was not independently checked against current policy; inspector snapshots outlived later commits; cache invalidation and connection revocation lacked the complete owning scope; receipt capacity was checked too late, and all failed writes shared one unresolved state.
- **Solution:** Revalidated current historical-source eligibility before compaction and dispatch, refreshed committed evidence and refused pending/failed history hydration, cleared active/parked receipt source caches on artifact purge, and scoped catalog revocation to its provider. Reserved receipt capacity in the existing work-acquisition transaction. Added neutral exact-request recovery only for a correlated definite terminal outcome and rollback-proven storage failure, with an explicit memory-only missing-receipt disclosure; ambiguous commit outcomes remain blocked.
- **Prevention / Reference:** Keep source bytes, durable evidence, cache state and provider outcome separate. Regression coverage includes concurrent final-slot admission, genuine quota rollback/storage-only retry, ambiguous-write refusal, stale consent, scoped purge and unrelated-provider failure. Actual terminal quota recovery is available through `scripts/receipts_smoke.py --quota-recovery`; its successful execution must be recorded separately.

### [2026-10-24] Native file-workflow qualification exposed SQLite contention and ConPTY capture sequencing

- **Context / Symptom:** Run39982573545 passed both macOS rows and Linux ARM, but Linux x86 ordinary requests stopped with `database is locked`; Windows file journeys failed `native console output did not settle within the bounded capture window`.
- **Root Cause:** Read-only admission kept a rollback-journal SHARED lock throughout full-store inspection while the sole writer attempted COMMIT with a250ms busy timeout. A targeted transaction/cursor reproduction demonstrated this mechanism; the exact failing CI SQL statement was not traced. Windows captures showed an intentionally held response spinner during dispatch and thousands of empty synchronized frames generated by4096 no-op Delete keys.
- **Solution:** Ordered same-process readers and writer commands through a per-canonical-store gate acquired before any SQLite lock and released before event backpressure, preserving foreign contention failures without retries/timeout inflation. Marked active asynchronous inputs as such and retained semantic completion assertions and strict final settling; cleared path restrictions using their actual input-size bound.
- **Prevention / Reference:** Added actual worker/read contention and independent-store regressions, native animated ConPTY regression, and all-target bilingual file workflows. Keep platform failure artifacts; successful compilation or a macOS fixture is not Windows/WSL proof.

### [2026-10-24] Public continuation, fresh-chat intent and source observation inherited unrelated capability limits

- **Context / Symptom:** Source audit found controlled Codex Implement follow-ups omitted preceding public implementation dialogue, native `/new` still included canonical history, model changes erased visible output, and safe source files over64KiB disappeared from host discovery and even small range reads.
- **Root Cause:** Fresh controlled threads retired opaque sessions without retaining canonical public pairs; native fresh/resume flags did not control history; model authority and display state were cleared together. Observational source identity reused the complete write-baseline cap, while queries/scopes were applied after exhaustive enumeration.
- **Solution:** Kept bounded owner-scoped public conversation separately from opaque sessions, froze explicit fresh intent and retired matching history only on dispatch, preserved attributed display, and separated source observation from mutation eligibility. Added pre-enumeration directory scope, exact-path membership checks, full-source privacy/hash validation up to16MiB, bounded64KiB range outputs and explicit noneditability above the mutation limit.
- **Prevention / Reference:** Actual native/Codex/ACP terminal drivers cover serialized continuation/fresh boundaries, cancelled unsent preparation, model display, oversized source reads and narrow discovery. These are deterministic runtime contracts, not live-model semantic acceptance.

### [2026-10-24] Short implementation requests remained read-only and implicit CLI auth blocked native setup

- **Context / Symptom:** User reported WSL implementation refusal and PowerShell file-creation failure citing `auth.json`. Previous deterministic release checks did not cover these request/credential paths.
- **Root Cause:** Short `구현해줘` / `Implement it` lacked an object noun and retained automatic Plan. Plan→Implement discarded public plan context with opaque history; Codex vendor Plan workflow could reinforce the refusal. Native account resolution also imported CLI credentials implicitly. Windows provider-file checks rejected trusted elevated ownership, and explicit JSON parsing rejected UTF-8 BOM. The user's exact ACL/token failure remains unobserved; these code defects are not proof of a particular private credential's state.
- **Solution:** Added explicit implementation intent and completed-plan-bound proceed, bounded public plan carryover and operation-scoped host-tool instructions. Removed implicit CLI import while retaining explicit Load CLI recovery; separated IO/privacy errors, admitted safe trusted-owner Windows provider files and stripped only a UTF-8 BOM for explicit import. Preserved OS denial, source boundaries, explicit readonly and exact write consent.
- **Prevention / Reference:** Integrated source passed 1,441 Rust tests (two ignored), formatting, strict Clippy and release build. Authentication fixture journeys passed at both terminal sizes without model calls or implicit resend. Bilingual real file workflows and five-target native gates are required separately; no live-account or WSL-host success is inferred from fixtures.

### [2026-10-24] Isolated Windows metadata diagnostic failed before HTTP connection

- **Context / Symptom:** Native run39973998985 failed the real HTTP401/403 diagnostic test on Windows with `fixture connection deadline`; joining the failed fixture before checking child output hid the child's diagnostic.
- **Root Cause:** The subprocess cleared its entire environment, including Windows runtime-discovery variables. Unlike existing repository subprocess isolation, it did not restore the OS environment needed by the child.
- **Solution:** Preserved only `SystemRoot`, `WINDIR`, `TEMP` and `TMP` alongside the isolated home and explicit loopback configuration; retained credential/proxy/provider isolation. Reported child failure before fixture-thread failure.
- **Prevention / Reference:** Focused local HTTP401/403 test and formatting passed. Exact-commit native run39976379482 passed Check and all five platform test/build/offline-install jobs, including Windows, on `e18f65b`.

### [2026-10-24] Full workbench smoke retained superseded initialization and recovery assumptions

- **Context / Symptom:** The complete workbench driver waited for `/init` to switch to Espeon, then later expected a send immediately after failed model metadata retrieval. At 60×18 it also expected HTTP status details to be visible without scrolling.
- **Root Cause:** The old driver encoded the removed forced appearance and pre-diagnostics recovery path. Current behavior preserves appearance and retained drafts, exposes a scoped diagnostic and requires deliberate recovery without automatic replay.
- **Solution:** Removed the incidental appearance assertion. Drove the actual connection diagnostic, scrolled to HTTP503, explicitly opened its existing setup action, returned without saving or network, asserted the exact retained request and unchanged GET/POST counts, then deliberately sent.
- **Prevention / Reference:** All 35 complete real workbench groups passed at both sizes using unchanged candidate D (`target/project-workbench-d-complete`), including no-dispatch initialization, metadata failure/recovery, cancellation, storage failures, migration, quotas and task lifecycle.

### [2026-10-23] Local context comparison lost its request and required a dispatch lease

- **Context / Symptom:** Actual `/context-compare` navigation cleared pending comparison preparation; after preserving navigation state, native preview failed with `Native file tools require their operation-bound durable lease`.
- **Root Cause:** Editor buffer-length changes during navigation were mistaken for user mutations. Provider preparation also shared a dispatch-only lease guard even though exact local receipts intentionally acquire no durable work.
- **Solution:** Invalidated comparison at actual text mutation boundaries, including equal-length edits, rather than navigation. Added explicit local-only immutable preparation across adapters; it retains no Run-capable state and native dispatch independently rejects it before network/file activity. Ordinary dispatch still requires its operation-bound lease.
- **Prevention / Reference:** Key-driven asynchronous catalogue and mutation regressions plus native exact-body/attempted-dispatch tests passed. Four EN/KO 120×36/60×18 terminal journeys reconciled all section/envelope bytes after overlapping capture removal, with no inference, claims, journals or executions and retained draft/caret/source bytes (`target/context-028-final-{en,ko}`).

### [2026-10-23] Ordinary project planning escalated to writing on follow-up

- **Context / Symptom:** The user reported permission barriers and missing responses when asking to plan a project, and questioned whether initialization was required.
- **Root Cause:** Planning classification omitted project/app nouns. Ordinary submission also unconditionally promoted unclassified follow-ups to Implement, changing operation ownership and retiring the planning conversation. `/init` was unrelated to ordinary planning admission.
- **Solution:** Recognized English/Korean project-planning intent and removed ordinary submission's unconditional coding fallback. Ambiguous follow-ups retain the current operation/history, while explicit mutation intent still selects Implement subject to user policy. The separately reviewed advanced-session fallback remains advanced-only.
- **Prevention / Reference:** Behavioral regressions cover direct planning, selected-path omission from readonly input, ambiguous follow-up continuity and subsequent genuine file-change intent. Actual native and controlled-Codex terminal proof is required before publication; fixtures do not establish live model semantics.

### [2026-10-23] Project initialization disabled later file creation

- **Context / Symptom:** The published `/init` path staged a proposal-only request, prohibited file creation and left later ordinary requests read-only.
- **Root Cause:** Initialization called the explicit Plan policy setter, selected a planning appearance and cleared selected paths. Ordinary capability selection correctly respected that explicit restriction, so the internal initialization action became a persistent user-policy override.
- **Solution:** Initialization now stages useful guide creation/update through the ordinary request path without changing explicit policy, appearance or paths. Expanded bounded multi-file continuation without weakening exact per-file approval, and made missing-model recovery open the local picker without resend.
- **Prevention / Reference:** Actual English/Korean 120×36 and 60×18 journeys created an approved guide followed by twelve nested modules across thirteen protocol rounds, checked exact disk contents, and preserved deliberate read-only policy. Four separate missing-model journeys retained exact requests, performed no recovery network calls and sent once only after explicit model selection and Enter. Evidence: `target/project-flow-{en,ko}-c`, `target/model-recovery-{en,ko}-a`.

### [2026-10-23] Complete runtime matrices exceeded the runner deadline

- **Context / Symptom:** The 600-second cooperation invocation ended after 22 successful groups before completing both sizes. The serial native-protocol invocation likewise ended before its full ten variants finished. Neither interrupted run was a completed pass.
- **Root Cause:** The unchanged full cooperation workload took approximately 666 seconds; the imposed runner deadline was shorter than that observed workload. The serial native matrix also needed more than the imposed deadline.
- **Solution:** Re-ran cooperation with a sufficient 1800-second deadline: all 24 groups passed, including journal-quota disposal and real hard-kill recovery. Ran every original native `MATRIX` row through unchanged complete `Journey.run` with three isolated owners: all ten variants/176 groups passed in approximately 251 seconds. No assertion or production consent/ownership rule changed.
- **Prevention / Reference:** Distinguish a process deadline from an application failure. Preserve completed evidence but do not report an interrupted matrix as a pass; allow the full workload to settle and clean up.

### [2026-10-23] Current access refusal retained an older setup-success notice

- **Context / Symptom:** The actual compact Korean HTTP403 diagnostic correctly showed access refusal and its next step, but also displayed the earlier `Native settings saved; choosing this route's model metadata only...` toast. The provider list could likewise retain outdated setup/auth guidance.
- **Root Cause:** Shared connection feedback consulted cached authentication outcomes and generic setup notices independently of the newer typed failure summary.
- **Solution:** Gave observed failures priority on the provider list and let the fixed failure/next-step summary own Actions/Diagnostics without the older toast. Kept active setup, method selection and installation feedback distinct.
- **Prevention / Reference:** Final source passed eight real English/Korean 120×36/60×18 HTTP401/403 journeys after successful setup, including root-list/diagnostic captures, private-output exclusion, exact retained drafts and explicit no-replay remedies. No failure or consent guard was suppressed.

### [2026-10-23] Metadata diagnostic fixture failed before reaching its HTTP listener

- **Context / Symptom:** The new actual 401/403 metadata regression observed no correlated failure because its synthetic connection never reached the loopback listener.
- **Root Cause:** Production metadata execution re-resolved and revalidated the configured source before network dispatch. A fabricated in-memory connection did not have a corresponding safely configured source in the child environment, so the prerequisite refusal correctly preceded HTTP.
- **Solution:** Ran the real configured keyless Compatible source in an isolated child HOME/environment against an owned ephemeral listener. Retained configuration/source admission rather than bypassing it to force the fixture through.
- **Prevention / Reference:** Exercise actual source resolution when testing transport boundaries. The regression checks typed status and private-body/header exclusion for 401 and 403 without credentials or paid calls.

### [2026-10-23] Auth smoke confused prose and centered borders with the recovery control

- **Context / Symptom:** The login fixture intermittently sent recovery input before the returned action screen, then the stricter wide-screen selector waited despite a visible `n:` next step. The compact variant could pass.
- **Root Cause:** Searching for the substring `n:` also matched descriptive text. Requiring it at the start of a whole terminal row then failed for centered overlays, whose underlying left pane preceded the inner border.
- **Solution:** Matched the actual `n:` control after an inner ASCII/Unicode border, waited for its displayed state, and then used the explicit next-step key. No production consent, replay or input-ownership guard was relaxed.
- **Prevention / Reference:** The corrected actual auth fixture passed both 120×36 and 60×18, including failed device login, foreground callback/status and cancellation/listener cleanup while retaining the exact saved Request.

### [2026-10-23] Native account requests depended on unrelated external CLI configuration

- **Context / Symptom:** Account Implement stopped before dispatch with `Send 0 payload bytes`, an unavailable destination and `Cannot read user provider config safely: require an absolute, non-project, user-owned regular file...`. Reproduced the exact public v0.1.25 preview using an isolated synthetic account environment and symlinked CLI `config.toml`, without approving network dispatch.
- **Root Cause:** Native account settings eagerly admitted and parsed the external CLI's model-preference file before resolving the selected account credential, even when model/token defaults were explicitly supplied. A rejected unrelated file therefore prevented native request construction.
- **Solution:** Removed external CLI model-file inheritance from native account resolution/setup. App-owned settings and deliberate environment defaults remain authoritative; selected credential files still require strict privacy, identity and expiry checks. No provider files or permissions were repaired.
- **Prevention / Reference:** Regression coverage rejects accidental CLI model inheritance, proves native resolution with malformed/writable/symlink/project CLI configuration and retains invalid environment/auth rejection. External CLI configuration belongs only to the separately chosen CLI route.

### [2026-10-23] Public-package verifier assumed LF-only native checksum manifests

- **Context / Symptom:** v0.1.25 publication succeeded, but the additional public-package proof stopped at a raw-text equality assertion for the Windows qualified artifact's `SHA256SUMS`.
- **Root Cause:** The Windows manifest contained the correct single archive digest/name with `\r\n`; the throwaway verifier assumed `\n`. Public digests and exact qualified archive byte comparisons had already matched.
- **Solution:** Parsed the manifest into exactly one digest/name line, accepting native line endings while retaining every archive, installer and GitHub artifact digest comparison. Reused downloaded files without overwriting them and continued verification; no tag, public asset or product code changed.
- **Prevention / Reference:** Compare checksum records semantically, but compare the packages they identify byte-for-byte. Do not normalize archives or compare dependency notices across different target builds.

### [2026-10-23] Native qualification exceeded cache egress allowance

- **Context / Symptom:** Native run `39930283345` reported `Failed to restore: Egress is over the account limit.` during the Check job.
- **Root Cause:** The remote cache service refused restoration because the account's cache egress allowance was exhausted; this was not a compiler or test failure.
- **Solution:** The workflow continued through its ordinary uncached path. Check and all five native build/test/package/offline-install jobs passed without changing qualification rules or account settings.
- **Prevention / Reference:** Treat cache availability as optional. Require complete successful source qualification rather than accepting an incomplete run or weakening gates.

### [2026-10-23] Controlled-runtime smoke skipped complete prepared scope

- **Context / Symptom:** The ordinary native file matrix passed, but both controlled Codex sizes stopped at `Run needs the complete scope`. A reopened cooperation review retained its scrolled position, so the harness could not see the immutable Request near the top.
- **Root Cause:** The older driver waited for Run without inspecting the complete executable/configuration trust scope. Its substring selector then mistook `Only Run sends...` in descriptive body text for the `y Run` control, stopped scrolling and sent an unreviewed key that Eevee correctly refused. It also assumed a species selected read/write policy and reopening reset the viewport.
- **Solution:** Added navigation-only review inspection and matched the actual bilingual Run/Cancel control rather than prose. Kept the separate explicit Run action and all scope guards. Controlled fixtures choose actual Plan/Implement policy; the future-draft check returns to the top before inspecting immutable Request bytes and reviewing complete scope again.
- **Prevention / Reference:** Corrected controlled Codex journeys passed English 120×36 and Korean 60×18 (eight groups). Full native protocol verification passed ten variants/176 groups. Exact file controls, immutable request bytes, writer ownership and no preapproval effects remain assertions; no consent guard or scope requirement was weakened.

### [2026-10-23] Fixed evolution forms blocked coding and hid required consent

- **Context / Symptom:** Public v0.1.24 gave a manually fixed Vaporeon only read/list tools for a create request, while a fixed Flareon read request acquired a writer lease. Cosmetic evolution also discarded same-policy conversation. Non-file and prepared external-policy permissions required an approval-menu detour.
- **Root Cause:** Form-to-operation defaults discarded fixed-form request classification, form selection reset explicit policy, and operation commands forced specialty changes. Empty attachments omitted initial native project identity; automatic presentation admitted only typed file requests.
- **Solution:** Separated request capability from appearance, preserved explicit restrictive policy and same-policy history, and made exact path restrictions directly accessible in every form. Shared a bounded frozen safe filename observation across provider envelopes. Reused current-owner consent presentation for generic/prepared permissions with neutral arrival, complete scrollable scope and explicit decisions. Permission titles identify waiting for consent rather than preparing changes.
- **Prevention / Reference:** `target/form-capability-before` records RED. The actual corrective fixed-Vaporeon journey passed read/create/edit denial and approval, exact CRLF bytes, current-folder exclusions and retained same-policy history. Compact Korean ACP exercised automatic prepared consent, generic deny/allow and separately approved real writes. Drivers recognize typed file controls plus neutral consent, rather than an obsolete file-only sentence, and wait for local/provider settlement before opening a path editor.

### [2026-10-23] File requests stayed read-only and nested permission scope could not be approved

- **Context / Symptom:** Actual v0.1.23 sent only read/list tools for `Create handoff.md` and for a quoted filename creation after source discovery. At 60×18, `src/components/workspace/editor/permissions/FilePermissionDialog.tsx` opened a popup but its repeated parent paths clipped required scope; Allow remained disabled and no file was created.
- **Root Cause:** Literal filtering erased filename object context and documentation routing omitted explicit handoff-file mutation. Unclassified follow-ups retained their previous nonwriting operation. The review required all scope to fit one viewport and offered no scrollable summary, creating a permanent compact-layout dead end.
- **Solution:** Retained bounded filename object markers without quoted verbs, recognized explicit file creation/edit/save intent and kept readonly clauses nonwriting. Reused the existing tool/lease/writer path. Added explicit decision selection, fixed localized buttons and contiguous scope exposure through scrolling, with no skipped-row approval. Safe home panes now present the current owner's exact request without an approval-menu detour.
- **Prevention / Reference:** RED captures are `target/file-permission-before-{intent,nested}`. Final candidate B passed eight English/Korean Responses/Chat Completions journeys at both sizes, including real reads, nested create denial/allow, exact hash-bound edit denial/allow, handoff-file creation, Tab cycling and plain-text selection (`target/file-permission-0124-ux-{en,ko}-b`). No original credentials, paid model calls or prose-to-file recovery were used.

### [2026-10-23] Parallel write fixtures collided and permission checks pinned old presentation

- **Context / Symptom:** The integrated suite hit `AlreadyExists` creating a Codex fixture directory; old popup tests also treated navigation as inert and matched an entire localized paragraph inside a bordered terminal frame. The Korean diagnostic teardown could not recognize the owned-work quit review.
- **Root Cause:** PID plus wall-clock nanoseconds did not guarantee uniqueness across concurrent fixture constructors. Presentation assertions predated explicit decision buttons and multiline controls; the quit driver matched only its English title.
- **Solution:** Added a process-local atomic fixture sequence without weakening create-only checks. Removed obsolete wording/incidental-index assertions and retained behavioral checks for no implicit approval, exact request identity, draft/focus and one-shot effects. Matched both localized quit titles only under the helper's existing explicit all-owner opt-in.
- **Prevention / Reference:** The integrated Rust suite passed 1,354 tests/two ignored after correction. `scripts/file_permission_smoke.py` exercises actual decision buttons, scope scrolling and file bytes rather than manufacturing a permission through an approval command.

### [2026-10-23] Public-package verification assumed identical native dependency notices

- **Context / Symptom:** Public v0.1.23 asset digests and manifest checks passed, but the one-session verifier rejected cross-platform `THIRD_PARTY_NOTICES` differences.
- **Root Cause:** The verifier incorrectly required identical notices across native dependency graphs. Architecture-specific `safe_arch`, Linux dependencies and Windows license grouping legitimately differ; newline normalization cannot make different dependency graphs identical.
- **Solution:** Compared each public archive byte-for-byte by SHA256 with its own exact-source, qualified native CI artifact instead. Retained all eight public digest checks, seven manifest checks, safe archive/member bounds, source license/installers, project notice and native executable identities. All five target-specific comparisons passed; no package, dependency notice, qualification rule or published asset was changed.
- **Prevention / Reference:** `target/release-0123-public-verification.json` records each qualified artifact ID and native notice hash. Native CI generated and checked notice coverage for its locked dependency graph. Text-only newline normalization never changes raw package or binary hashes.

### [2026-10-23] Provider runtime drivers retained obsolete payload and exit assumptions

- **Context / Symptom:** The connection journeys passed, but older auth/workbench drivers waited for removed status prose, asserted a single JSON user message without tools, and waited for the old `Exit with unknown outcome` popup.
- **Root Cause:** Those assertions predated role-bearing canonical conversation, bounded coding tools and the all-owner quit review. They rejected valid current behavior rather than proving request ownership or terminal restoration.
- **Solution:** Removed incidental status/body-shape assertions. Reused the canonical payload decoder while checking exact user/assistant history, task/source context, unsent-draft exclusion, request count and no local effects. Waited for restored method controls with unchanged exact argv/listener assertions. Added explicit opt-in to the shared quit helper for the existing all-owner review; ordinary callers cannot automatically approve that scope.
- **Prevention / Reference:** Actual cancellation/quit diagnosis showed clean exit after explicit all-owner Right/Enter; the throwaway was removed. Removed incidental pane-index assertions and selected task IDs from rendered rows only after their save acknowledgment, rather than racing retained drafts. Final B passed all 35 workbench cases (`target/provider-0123-workbench-b4`), ten native protocol variants/176 groups and all bilingual provider UX cases. Corrupt/newer-store fixtures deliberately tear down only their owned process instead of claiming a failed store can save-and-quit.

### [2026-10-23] OpenAI setup mixed authorities and hid compact storage consent

- **Context / Symptom:** Actual v0.1.22 compact captures showed three OpenAI/Codex entries, missing CLI-first navigation, raw access-token entry before login, English status in Korean UI and a clipped plaintext warning. Review also found native cancellation clearing unrelated conversations and a small-terminal auth return dropping its route/model continuation.
- **Root Cause:** Internal transports doubled as top-level product choices; login depended on CLI registration/import, readiness used English strings, and storage admission checked presence rather than complete rendered disclosure. The new native path initially inherited global CLI-auth invalidation and drained the picker before its size gate.
- **Solution:** Grouped the existing routes behind explicit account/API/custom/advanced methods; added standalone browser/device authorization and source-bound credential replacement. Used typed localized readiness, primary/advanced forms, private paste ownership and cell-aware caret placement. Gated Save on both complete disclosures; scoped accepted native mutation and retained owner-bound continuation through resize and ordinary switch review.
- **Prevention / Reference:** `scripts/provider_ux_smoke.py` passed actual English/Korean 120×36 and 60×18 family/login-review/API/custom/request journeys, exact saved draft/caret, neutral cancellation, zero implicit inference and real loopback Responses routing (`target/provider-0123-ux-a`). Live issuer/account acceptance and automatic token renewal are not claimed.

### [2026-10-23] Accepted OAuth sockets inherited nonblocking mode and Windows split authorization URLs

- **Context / Symptom:** Native browser/device exchange fixtures failed with macOS `WouldBlock` / code 35 before valid token or denial responses. Static review found Windows `cmd /c start` interpreting OAuth URL ampersands as command separators.
- **Root Cause:** Accepted sockets inherited a listener's nonblocking mode; setting a timeout did not restore blocking reads. Command-shell URL launch did not preserve the complete query.
- **Solution:** Restored bounded blocking callback/fixture reads, retained absolute deadlines and complete Content-Length framing, and covered fragmented requests. Replaced Windows shell launch with `ShellExecuteW` and a complete UTF-16 URL. Native callback/token/device fixtures and the integrated Rust suite passed; native Windows qualification remains a separate release gate.
- **Prevention / Reference:** `src/providers/auth/openai/tests.rs`, shared browser opener in `auth.rs`; no real OAuth grant or provider-model request was used for these checks.

### [2026-10-23] Global motion preference writes crossed partner snapshots

- **Context / Symptom:** Review found that rapid off/on/off choices across front/parked partners could leave an older pending snapshot last, a motion-only save could persist a startup provider override, and an unhydrated restored lead could mistake its static safety fallback for the user's saved choice. A first inheritance correction then blocked 26 existing cooperation cases.
- **Root Cause:** A global setting was merged through per-partner snapshots without normalizing all unsent owners. Provider preference projection and raw durable preference/admission state were conflated; deriving `restore_required` from `!loaded` manufactured a request-admission blocker.
- **Solution:** Kept one preference writer, normalized every unsent snapshot and acknowledgment to the latest global motion intent, retained durable provider identity separately, and inherited the raw loaded/value/actual restore-required fields. Delivered shared hydration to unhydrated front/parked states. Allowed motion controls during owned work without changing its request authority.
- **Prevention / Reference:** Regressions cover cross-partner write ordering, startup overrides, second-lead restoration and unknown-vs-saved values. Rust 1.90.0 passed 1,327 tests/two ignored and strict Clippy. Actual bilingual/both-size motion journeys persisted static live activity across restart, honored launch overrides and refused the real v0.1.21 writer with byte-identical data before current-version resume (`target/pokemon-0122-motion-c`).

### [2026-10-23] Portable TM files were rejected by relative-only path exclusion

- **Context / Symptom:** Store round-trip tests rejected valid absolute TM input/output files as excluded, despite bounded definitions and a safe publication target.
- **Root Cause:** The portable filename validator passed an absolute pathname to the source-context exclusion matcher, whose contract is a project-relative selector. Root components therefore triggered the conservative rejection intended for unsafe source selectors.
- **Solution:** Checked filename normal components for exclusions, explicitly rejected parent traversal, and retained the shared anchored no-link bounded reader/create-only writer for filesystem authority. Portable move selectors and run cwd/argv kept their stricter project-relative validation; no unsafe path was rewritten or widened.
- **Prevention / Reference:** Store regressions cover all step kinds, existing outputs, import provenance, unsupported/foreign fields, replacement and unavailable selectors. Final C passed all four locale/size journeys, every form's fresh normal review, restart, two-project round-trip and missing-selector refusal without inference calls (`target/pokemon-0122-tm-c/results.json`).

### [2026-10-23] Fast local reviews raced their exact retained draft save

- **Context / Symptom:** Actual formation review rejected an immediate explicit `y` while its unchanged local save was pending. A pin TM's ordinary validation could observe the previous task revision; a retained export-success message then hid the new action's actual status.
- **Root Cause:** Review identity was valid but task persistence had not settled. Move validation was queued before flushing that same local snapshot, and the portable success notice outlived its action.
- **Solution:** A formation may wait only for its exact task/lead/revision/roster save after explicit confirmation; changed identity, work, cancellation, search, resize and quit revoke the pending intent. The store still revalidates atomically. Flushed the exact task snapshot before move validation, without relaxing callback guards, and cleared export success on the next accepted move action.
- **Prevention / Reference:** Regression tests cover queued-intent revocation and preservation of explicit read-only roles. All 28 final-candidate formation groups passed across English/Korean and 120×36/60×18, including actual busy refusal, exact Unicode draft/caret, restart and no implicit effects (`target/pokemon-0122-formations-{en,ko}-c`).

### [2026-10-23] New PTY drivers confused review deferral and process readiness

- **Context / Symptom:** A restarted motion fixture reported a missing API base URL; TM fixtures tried to leave a deferred full-receipt request as if Escape cancelled it, then failed with `IndexError: list index out of range` at restart.
- **Root Cause:** The connection was deliberately session-only, Escape correctly kept the owned request waiting, and existing SQLite rows proved an earlier process had initialized—not that the new process had rendered its first frame.
- **Solution:** Explicitly reconnected the disposable loopback fixture after restart, explicitly cancelled owned request preparation and observed lease release before the next independent TM case, and waited for the new process's actual saved frame before footer/navigation access. Kept provider persistence, approval and ownership policy unchanged.
- **Prevention / Reference:** Runtime drivers use disposable homes/projects, read-only SQL observation and actual terminal input; model inference POSTs stayed absent throughout final C TM import/export/use-review, restart and cross-project journeys in all four locale/size combinations. Motion restart/real-activity, exact key/paste forwarding and prior-writer refusal/current resume also passed all four (`target/pokemon-0122-motion-c2`).

### [2026-10-23] Navigation smoke confused list position and wrapped source details

- **Context / Symptom:** The first driver selected task row zero and reported a same-task return failure, although the visible star belonged to row two. Compact capture text and wrapped payload text also fell outside literal whole-screen string selectors.
- **Root Cause:** The driver assumed task ordering and viewport placement rather than identifying the actual active task and bounded review surface. A wide action list legitimately showed advanced entries below the newly prioritized everyday actions.
- **Solution:** Selected the real starred task identity and separately selected another task to prove its guard, paged source review to its actual bytes, used the payload header and localized mode-specific controls, and asserted action ordering rather than a fixed number of visible rows.
- **Verification:** Both-size predecessor journeys reproduced all four detours; the same final driver passed all four English/Korean candidate journeys with exact draft/caret, live claim, attachment and HTTP-count checks. No product guard was weakened for the driver.

### [2026-10-23] Local stable toolchain differed from pinned release qualification

- **Context / Symptom:** Local tests passed, but default stable Rust 1.97 Clippy rejected existing `is_some()`/`expect()` patterns in planning and handoff with `clippy::unnecessary_unwrap`.
- **Root Cause:** Local default `stable` differed from the repository's Rust 1.90.0 native qualification workflow. The warnings were in unchanged code, not the navigation change.
- **Solution:** Used the already installed `+1.90.0` toolchain for locked tests, formatting, strict all-target/all-feature Clippy and release build. All passed; no lint suppression or unrelated source rewrite was added.
- **Prevention / Reference:** Match `.github/workflows/native.yml`: `cargo +1.90.0 test --locked --all-targets --all-features` and `cargo +1.90.0 clippy --locked --all-targets --all-features -- -D warnings`.

### [2026-10-23] Successful workflow retained an incomplete native job record

- **Context / Symptom:** Corrected-source run `39835415051` concluded `completed/success`, and every Linux ARM64 step—including upload and `Complete job`—reported success. Its job `120022311546` nevertheless remained `in_progress` with null conclusion/completion time. The strict release qualifier refused `Unqualified native job: Build / linux-arm64`.
- **Root Cause:** GitHub's aggregate workflow result and persisted per-job completion metadata disagreed. A successful watcher result was insufficient to establish the required single-attempt native artifact provenance.
- **Solution:** Kept qualification unchanged and published no tag from that run. Recorded the service inconsistency in a documentation-only source commit and requested a fresh complete main-push qualification with new artifact identities, preserving the earlier diagnostic evidence.
- **Prevention / Reference:** Admit only a run whose Check and all five job records, steps, attempt and artifact timestamps satisfy the existing qualifier. Never substitute workflow-level success or combine packages across runs.
- **Verification:** Fresh exact-source run `39838062098` completed Check and all five native jobs, including Windows, with valid completion metadata. The unchanged qualifier admitted its five artifacts; publication `39839919003` promoted only those identities as v0.1.20.

### [2026-10-23] Windows qualification rejected a POSIX-only held-file fixture

- **Context / Symptom:** Native run `39833784492` passed Check and all four macOS/Linux rows, but Windows failed `saving_reviewed_file_never_pins_capture` before the held preference could be queued (1,243 passed, one failed, two ignored). No release tag was created.
- **Root Cause:** The unit fixture used `/fixture` for its app, memory, source and repository identities. On Windows this is rooted but not a fully qualified path with a volume; the existing source-capture validator correctly refused it.
- **Solution:** Used one platform-valid fixture project identity for every related app/memory/source/report/request field, with a Windows volume-qualified path. Production source validation, native CI commands and assertions were unchanged.
- **Prevention / Reference:** Require all five native rows at the corrected exact source commit; do not reuse the four earlier successful artifacts or skip the Windows case.

### [2026-10-23] Runtime drivers confused visible surfaces and newly introduced empty tables

- **Context / Symptom:** The root journey could not see the beginning of the long `/init` draft. ACP journeys reported missing file identity while their captures showed an exact file approval, not the transcript. Schema16→18 reported `Migration rewrote retained evidence`.
- **Root Cause:** The initialized request's caret correctly left its first line above the viewport. After deferring an approval, `/response` deliberately reopened its pending Planning review; a source-text-only predicate matched the consent excerpt. Migration comparison treated an absent predecessor held table as different from a newly created empty table; retained rows were unchanged.
- **Solution:** Observed `/init`'s existing preparation acknowledgment and captured its read-only proposal. Stayed on the inline transcript after ACP deferral, required typed code outside consent and verified that navigation sent no decision or file effect. Canonicalized absent optional held/ribbon tables to empty snapshots while retaining exact row comparison and genuine old-writer byte-preservation checks.
- **Verification:** Actual three-project source journeys, all 24 bilingual ACP journeys and both genuine Schema16/17→18 preservation/refusal/resume paths passed with the same frozen executable. Kept the first-failure captures; no product approval or migration guard was weakened.

### [2026-10-23] Project-local application data was eligible as ordinary source context

- **Context / Symptom:** Boundary review traced custom `--data-dir ./local-data` retained text outputs into ordinary source browsing and explicit attachment. Existing held-item persistence and file tools protected storage, but ordinary context did not. This required explicit attachment, not automatic upload.
- **Root Cause:** Source inventory, capture and ordinary attachment admission used relative exclusions without the application's absolute storage boundary. Launching at or below the data directory could not be protected with a relative prefix alone.
- **Solution:** Froze canonical storage identity in internal repository/context policy, pruned it before traversal and content reads, and checked native Git path resolution again before reading objects. Refused new private-storage pins and previously committed private-storage attachments at shared provider admission. Preserved historical rows and retained bytes; moved ordinary test source fixtures out of their private store directories.
- **Prevention / Reference:** Regressions cover equal/ancestor/aliased storage roots, untouched retained output, valid public-source recovery and genuine legacy attachment admission. The actual project-scope driver filters private output to zero eligible sources before capturing the public source.

### [2026-10-23] Native Git filenames duplicated a decomposed macOS source

- **Context / Symptom:** The literal nested-project regression returned two changes for one Unicode source when Git and the filesystem represented its name differently.
- **Root Cause:** Git's precomposed index spelling and the working tree's native decomposed spelling were treated as different source identities. Metacharacters in a nested cwd also required literal Git pathspec treatment.
- **Solution:** Resolved admitted Git names to the native opened-handle path on macOS, retained original object identities for baseline content, and used literal scoped Git pathspecs. Checked protected-storage boundaries before and after native resolution without normalizing Unicode or case-folding filesystem identity.
- **Prevention / Reference:** The full Rust suite passed the literal metacharacter/Unicode project regression and protected-storage boundary cases.

### [2026-10-23] SDK known refusals and inaccessible descendants blocked recovery

- **Context / Symptom:** Integration review traced a known pre-execution source/write refusal into a fatal unknown result when the SDK later returned its ordinary error. Host discovery could also lose all safe sibling results because one descendant directory could not be enumerated.
- **Root Cause:** The result ledger recorded user denials only after successful enrollment, omitting mandatory-hook/early callback refusals. Directory traversal caught no per-descendant permission errors.
- **Solution:** Added an exact bounded pre-execution refusal ledger accepting only its matching nonreplayed error result, without overwriting approved/possibly partial effects. Discovery skips EACCES/EPERM descendants with incomplete coverage while preserving requested-root/identity failures. Also preserved built-in protected-directory checks across canonical root ancestors, so launching inside a private store cannot erase its protected path prefix.
- **Verification:** All 44 sidecar policy/actual production-IPC cases passed, including refusal-result recovery into real valid create/edit, known failed versus partial effects, real inaccessible-directory glob/grep recovery and protected-root no-dispatch. No account/model call was made.

### [2026-10-23] Nested launches widened context and changed exclusion meaning

- **Context / Symptom:** The actual preceding binary launched in a nested Unicode/metacharacter folder showed three source candidates instead of the one project-local allowed file: its own configured-excluded file, the current source with a parent-relative prefix, and a sibling project's file. Retained RED evidence is `target/project-scope-before`.
- **Root Cause:** Tasks and file tools used canonical invocation cwd, while repository observation promoted context inventory to Git toplevel. The same relative exclusion then referred to a different physical location; initialization and some diagnostic/rerun paths also reused the wrong root.
- **Solution:** Kept canonical launch-folder identity as the project, separated optional enclosing Git metadata, scoped inventory/diffs and exclusions to project-relative paths, and retained bounded filesystem browsing without inventing Git evidence in plain folders. Historical enclosing-root captures remain unchanged and stale; request admission refuses mismatched report/capture roots instead of rebasing them. Kept exact cwd identity separate from escaped UI labels.
- **Prevention / Reference:** `scripts/project_scope_smoke.py` exercises actual nested/plain-folder capture, initialization and canonical/control-character identity. Source/dispatch regressions cover literal paths, sibling exclusion and historical scope refusal.

### [2026-10-23] Compact milestone inspector buried its original evidence

- **Context / Symptom:** The actual 60×18 milestone journey displayed only disclosure and event-list text before the fixed controls. The selected original source could not be identified on the first page; a rendered first-screen evidence regression failed in both-language coverage.
- **Root Cause:** The inspector rendered its long disclaimer and all three event headings before the selected receipt's evidence identity. Scrolling worked, but initial inspection prioritized explanation over facts.
- **Solution:** Rendered the selected factual event and original artifact/move identity first, then ownership, freshness and supporting detail, with other event headings and the full disclosure below. Kept paging and fixed controls. Updated the common smoke driver's known non-consent drawers so returning from the ordinary evidence inspector through its existing Ribbons return frame was handled without dismissing an unknown dialog.
- **Prevention / Reference:** `compact_ribbon_inspection_shows_original_evidence_without_paging` protects minimum-size original-source visibility; the paging regression separately covers every retained evidence row.

### [2026-10-23] Milestone harness confused path aliases, restored inspection and durable acknowledgment

- **Context / Symptom:** A new milestone journey reported `actual handoff receipt did not commit` despite a successful actual export; the retained read-only database capture contained that exact receipt. The Schema17 migration journey could not see its goal behind the restored approved-memory sheet. An existing workflow journey observed `Recorded Accept` before its database row changed.
- **Root Cause:** The new fixture used Darwin's `/var` spelling while durable project ownership was canonical `/private/var`, so an exact-identity filter hid real rows. Migration correctly restored the predecessor's memory-inspection navigation. The accepted-review notice described local reducer state, not the single writer's eventual commit acknowledgment.
- **Solution:** Canonicalized the disposable fixture root like the existing records driver, without weakening product path identity. Bound export proof to the exact compact-document digest and revision. Captured and closed only the known committed non-consent memory inspector before checking preserved goal data. Changed workflow acceptance to wait for the real committed review rather than treating a visible notice as persistence proof.
- **Prevention / Reference:** Keep actual database identity, navigational occlusion and committed acknowledgment separate. Preserve failed terminal/SQLite captures and genuine predecessor binaries; never synthesize a schema or insert fixture milestone rows.

### [2026-10-23] Held configuration was inspectable but absent from the compact workspace

- **Context / Symptom:** Actual 60×18 captures showed no saved/proposed item in the normal workspace even though `/held` correctly showed the configuration and frozen receipt. A targeted rendered regression also showed the old strip had a status but no file identity, and dismissal left that strip painted.
- **Root Cause:** The optional strip required nine free rows after wrapped generic form advice, omitted its target, and did not gate on visit dismissal. Targeted transition coverage additionally exposed local dismissal being refused during saving and matching preference acknowledgments unconditionally clearing its dismissal flag.
- **Solution:** Gave the deliberate held choice the generic-advice slot while retaining explicit carry-over and safety/live-code priority. Used concise localized states, an escaped/clipped identity and a reserved `/held` control; retained a useful output viewport. Made visit-only dismissal independent of context/persistence guards and preserved it through matching acknowledgments, without touching the preference or selected context. Added failing-before identity/control, redraw, unsaved-draft and pending-save/ack transition coverage instead of verifying only the inspector.
- **Prevention / Reference:** Check the ordinary minimum-size surface, long/CJK identities and fixed controls as well as opened panels. `scripts/held_smoke.py` retains separate main-workspace saved/proposed/dismissed captures.

### [2026-10-23] Held-context terminal harness crossed unrelated review lifecycles

- **Context / Symptom:** The first real held-source journey could not refresh after a stale-source refusal; later journeys waited behind an optional move offer or appeared to ignore dismissal. Record checks also required an old byte caption, obsolete scope-entry wording and a fixed number of pages for an enlarged native payload.
- **Root Cause:** A blocked immutable preview intentionally retained its claim for explicit cancellation. Three pin uses legitimately offered a move. Escape acknowledged an evolution notification but was correctly forwarded to a dirty editor, opening a neutral draft-discard dialog; the subsequent “dismiss” text never reached the palette. Source-wire receipts invalidate on composer changes, and complete tool schemas can exceed arbitrary full-payload paging assumptions.
- **Solution:** Waited for actual `Request blocked:`, cancelled that exact preview and required no claims/no POST before restoring fixture bytes. Deliberately postponed the visible optional move offer. Used a neutral F12 to acknowledge evolution without an editor action; retained both first-failure modal captures and substantive draft/chip assertions. Entered generic owned review through its current Alt+A control, then explicitly cancelled without sending. Deleted obsolete receipt-copy/full-payload-wording assertions while keeping actual raw HTTP single-occurrence bytes, exact source/memory revision/history and no-effect scope/cancellation checks.
- **Prevention / Reference:** `scripts/held_smoke.py` exercises real English/Korean source and exact-memory flows at both sizes. Keep product state, durable ownership, transport effects and temporary overlay visibility as separate assertions.

### [2026-10-23] Compact held receipts and wide-cell rendering assertions

- **Context / Symptom:** Held-item integration exposed a compact frozen-membership assertion failure and apparent Korean text corruption when switching a rendered receipt from excluded to included.
- **Root Cause:** Optional notice/policy rows preceded the frozen receipt in a fixed-height body. Separately, the TestBackend text decoder concatenated every cell, including continuation cells underneath wide Korean glyphs; those cells are not independently painted and could retain old symbols after a diff.
- **Solution:** Prioritized the frozen owner, target and contribution before optional help. Decoded rendered test rows by actual symbol display width using the existing terminal text primitive, rather than stripping spaces, changing labels or adding a dependency. Kept the exact membership/owner/draft assertions at both supported sizes and languages.
- **Prevention / Reference:** Verify the actual PTY surface as well as rendered state. Distinguish wide-glyph continuation cells from visible characters, especially after a localized state transition.

### [2026-10-23] Installed terminal harness mistook a visible evolution popup for lost goal data

- **Context / Symptom:** The actual public v0.1.19 installation reached its full terminal harness; 120×36, 100×30 and 70×24 passed, while 60×18 stopped at `form selection lost the goal`.
- **Root Cause:** Decoding the retained failing ANSI record showed the complete nonmodal Eevee→Sylveon evolution popup over the compact workspace. The old harness demanded the goal be immediately visible without acknowledging the newly requested popup; the failure did not establish data loss.
- **Solution:** Updated the real form-selection journey to capture and acknowledge the visible receipt with Escape, then inspect the original goal and continue its existing edit/discard/idle/lifecycle assertions. Kept the first failed evidence and used a fresh installation output directory; the published executable/tag was not rebuilt or changed.
- **Prevention / Reference:** Distinguish temporary occlusion from durable-state loss. Preserve substantive goal/draft checks after deliberately dismissing a transient view instead of deleting or weakening them.

### [2026-10-22] Evolution notification competed with compact work surfaces

- **Context / Symptom:** Combined popup checks exposed a compact notification overlapping composer identity and retained-response fixtures still displaying the new transition receipt. An unseen transition could also have been consumed by input intended for another sheet.
- **Root Cause:** A full-screen-centered decorative panel competed with the existing compact composer, and transition dismissal did not distinguish an actually displayed notification from a blocked one. Automatic carry-over banners intentionally hide on deliberate input, so inspecting them afterward must reopen retained state.
- **Solution:** Constrained the popup between the persistent header and composer; deferred it behind approvals, other editors/sheets, explicit carry-over inspection and active generated-file code. Recorded actual visible rendering before input could consume the receipt, retained normal editor input, and suppressed restore/same-form replay. Rendered regressions now exercise popup acknowledgement and deliberate carry-over reopening instead of assuming no transition exists.
- **Actual compact follow-through:** A 60×18 PTY capture then showed nested generic-sheet margins reducing the notification to five rows, clipping its reason/continue/no-approval disclosure. Used the protected header-to-composer region directly rather than applying a second sheet margin; the actual bilingual journey now requires the safety/continue disclosure to be visible.
- **Prevention / Reference:** Combined bilingual/both-size evolution, file-generation and scrolling journeys exercise these surfaces together; use actual terminal captures for CJK text, since raw TestBackend cell dumps include wide-cell padding.

### [2026-10-22] Compact file progress had a title but no code viewport

- **Context / Symptom:** The first actual v0.1.18 candidate failed the 60×18 Korean mid-argument journey before any navigation: the new file card said generating/not saved, but no `STREAM_STAGE_ONE` source line could appear. The follow range reported `0-44/44`.
- **Root Cause:** Writer preparation, a wrapped automatic-evolution receipt and carry-over strip consumed every response-body row. Adding streamed data alone did not make the feature visible in the normal first-use surface.
- **Solution:** Current-request file output now takes viewport priority over duplicate preparation/transition/advisory/carry-over chrome, without mutating retained disclosures or future drafts. Actual permission scope and foreign writer ownership stay visible; old file cards cannot suppress a new request's context.
- **Prevention / Reference:** The gated actual driver requires the first code inline before any `/response` or latest-navigation action. Added a compact whole-surface regression covering generated/applied cards, retained transition/carry-over state, draft/caret and old-request isolation.
- **Display follow-through:** Actual captures also exposed CRLF source rendered with replacement markers and a filename that scrolled above the compact code tail. Normalized only CRLF display pairs (not file bytes or standalone controls), kept the filename beside the file lifecycle header, and preserved explicit carry-over inspection separately from automatic chrome. The stream fixture now splits exactly at its first-code gate instead of letting an aggregated transport fragment deliver both stages at once; cancellation exit follows the real all-owner unknown-outcome review.

### [2026-10-22] Language-server queries exited before returning symbols

- **Context / Symptom:** Both the initial Rust symbols request and exported `ItemKind` references request failed because `rust-analyzer` exited with code 0 before returning results.
- **Root Cause:** The server terminated during the query; its underlying cause was not established. No reference results were available and an empty result was not treated as evidence that there were no callers.
- **Solution:** Used bounded repository searches for exhaustive consumers, then pinned-toolchain compilation, behavior tests and actual terminal journeys. No editor/server configuration or user environment was changed.

### [2026-10-22] Preview saturation preempted a critical uncertain-effect journal

- **Context / Symptom:** Integrated live-preview tests failed the existing saturated-event safety regression: `LocalEffectsUnknown` and terminal settlement remained, but the authoritative failure journal count was zero.
- **Root Cause:** A new non-authoritative file-card update could exhaust ordinary UI capacity before the reserved journal event was enqueued. The same ordering risk existed in interrupted permission handling.
- **Solution:** Kept actual local uncertainty first, published its authoritative journal before the optional file-card update, and similarly recorded interrupted permission evidence before presentation. Preview capacity errors still stop the transport; no warning was suppressed and no reserve or authority gate was weakened.
- **Prevention / Reference:** Retained `saturated_ordinary_event_capacity_cannot_hide_local_uncertainty_or_terminal_settlement` as a behavior regression. Actual executor receipts and terminal/callback state must take priority over progress presentation.

### [2026-10-22] Public Windows license used native CRLF line endings

- **Context / Symptom:** The one-session v0.1.17 public verifier passed downloaded asset/manifest hashes, then failed a raw `LICENSE` byte-equality assertion on the Windows ZIP.
- **Root Cause:** Windows checkout preserved 201 CRLF endings: the archived license was 11,525 bytes versus the source's 11,324 LF bytes. All four tar licenses matched source bytes directly; replacing only CRLF with LF made the Windows license identical too. No license content or public package was corrupted.
- **Solution:** Compared license text with CRLF-only normalization while retaining strict original-byte GitHub/manifest SHA-256 checks for every archive/installer and native executable checks. Recorded each archive's license line-ending style; all eight public digests, seven manifest entries and five safe archive/header/license/notice checks passed unchanged assets.
- **Prevention / Reference:** Treat platform-native license line endings separately from release integrity. Never normalize package bytes or checksums and never replace a published asset to satisfy a text-only verifier.
- **Rechecked 2026-10-23:** The v0.1.21 throwaway verifier initially made the same raw Windows-license assertion. All four tar licenses were byte-identical and the ZIP differed only by the same 201 CRLF endings. Restricted normalization to Windows license/notice text; all raw public asset digests, native binaries and exact-source installer checks remained unchanged and passed.

### [2026-10-22] Automatic routing separated a read-then-edit request from its source context

- **Context / Symptom:** Candidate A created files, but all ten native creation fixtures failed at “List the project files, read the generated Python source, and edit only its value using the observed digest.” The request entered a nonwriting scope. The first automatic-mode terminal driver also needed to dismiss an unrelated advisory banner to expose compact output.
- **Root Cause:** Clause splitting required an edit action and source noun in the same clause, losing the antecedent of “its value.” The old advisory banner competed with the already selected automatic role and transient transition strip for scarce response rows.
- **Solution:** Reused source context only across affirmative, nonliteral clauses while keeping negation/read-only precedence and exact consent unchanged. Added the exact runtime request, en/ko multi-clause and Submit→frozen-Implement regressions. Automatically evolved roles retain advisory evidence in explicit inspectors but no longer reserve a competing response-banner row; manual roles retain their strip. Removed the driver's dismissal workaround rather than accepting it as normal UX.
- **Follow-through / Prevention:** Candidate B passed all eight roles at both sizes/languages, actual read/list/nonwriting scopes, exact file/document consent, explicit Plan refusal and eight real restarts. The viewport regression failed before the banner correction. Candidate C then exposed a manual-role variant: the new evolution receipt stayed visible indefinitely, unlike existing carry-over feedback. Reused deliberate-key/paste lifetime handling to reclaim its rows without consuming input or changing frozen ownership, retaining passive resize/release visibility. Final qualification must exercise both automatic and manual compact output without hidden dismissal; original failures are retained.
- **Final local resolution:** Candidate D passed all automatic and manual form flows at both terminal sizes in English/Korean without any advisory-dismissal workaround. A dedicated regression preserved disclosure through resize/key release, consumed no subsequent paste and kept the original frozen request intact; all 1,187 Rust tests/two ignored and strict pinned Clippy passed. The preceding C also passed every ordinary file-generation/source journey in both languages.

### [2026-10-22] Ambient Rust toolchain differed from native release qualification

- **Context / Symptom:** An unqualified local `cargo clippy` selected Rust 1.97 and rejected existing permission/handoff `is_some` plus `expect` patterns with newer `unnecessary_unwrap` diagnostics. The native workflow explicitly qualifies Rust 1.90.0.
- **Root Cause:** The shell's ambient default was not the repository's release toolchain; comparing its evolving lint set with the pinned native gate was misleading.
- **Solution:** Ran explicit `cargo +1.90.0` formatting, locked all-feature tests, strict all-target/all-feature Clippy and release build. Candidate A passed 1,183 tests/two ignored and strict Clippy. Also removed the header formatter made obsolete by the shared width-budgeted layout and reused existing test boxes rather than allocating replacements. No lint allowance or release-gate reduction was added.
- **Prevention / Reference:** Use the workflow's explicit toolchain for local release proof; an ambient-toolchain upgrade is a separate change.

### [2026-10-22] Successful file execution still had an obscure approval interaction

- **Context / Symptom:** The user reported file-creation UX still felt wrong after v0.1.16. Actual compact Korean first-file output showed editing/completed labels and English `/approvals` hints while no file existed. A neutral approval Enter sent default Deny. Earlier runtime fixtures opened `/approvals` and used Enter as a deliberate denial, hiding the interaction problem.
- **Root Cause:** A generic permission selector was reused as the primary file workflow; display text omitted code, selected-Deny submission was ambiguous, pending/working chrome duplicated status and consumed the transcript, and machine digest receipts could leak into ordinary output.
- **Solution:** Added deliberate owned Enter/global FIFO Alt+A entry, typed localized operation/path/parents and bounded code/diff preview, inspection-only neutral Enter, explicit a/y once or n denial, preserved future drafts and correlation, and separated human outcome text from machine journals. Collapsed redundant single-owner waiting chrome and suppressed lower-priority suggestions/queued notices while a review is pending.
- **Prevention / Reference:** Tightened the actual creation driver to alternate Enter/Alt+A without `/approvals` fallback and verify repeated neutral Enter caused neither effects nor a response. Both languages/sizes passed real create/read/edit, denial and race boundaries. Journal refusals remain structured even when display is human-readable; their consumer regression caught an intermediate accidental loss of the machine refusal status before release. No permission or filesystem safety gate was removed.

### [2026-10-22] Windows discovery serialized native separators into model paths

- **Context / Symptom:** Initial native qualification `39722703001` failed two Windows discovery tests: actual list results included `"src\\lib.rs"` instead of `"src/lib.rs"`; the generated Unicode path also did not match the portable returned spelling.
- **Root Cause:** Discovery serialized `PathBuf` directly and searched the query against native separators. Cross-platform tool consumers and directory queries use `/`; the actual retained filesystem paths must still preserve their native spelling and identity.
- **Solution:** Converted only discovered wire strings, before query matching, to `/` on Windows with length-preserving ASCII replacement. Reused owned native string storage; left filesystem traversal, Unicode, frozen identities, protections and readback unchanged. Both Git/non-Git regressions now query a real portable directory prefix rather than a platform-independent suffix.
- **Prevention / Reference:** All-five-native qualification remained mandatory; a locally passing macOS run did not qualify Windows. Failed source `9786a2d` was not tagged or published. The same run's Linux ARM worker reached notice generation after passing tests/build, then was cancelled at its 45-minute deadline with unavailable job logs; its internal cause remained unconfirmed and its missing package was not substituted from another run.
- **Qualification resolution:** Corrected source `75a5a2b` passed Check and all five complete native rows in `39728072097` attempt1, including both formerly failing Windows tests and Linux ARM notice generation/package/offline installation. Only that run's immutable artifacts were admitted for v0.1.16; no gate was disabled and no old package was reused.

### [2026-10-22] Ordinary requests never advertised coding tools

- **Context / Symptom:** The user still could not create files after v0.1.15. An actual ordinary-composer probe sent `"tools": null` and acquired a non-writing lease (`target/ordinary-generation-before`).
- **Root Cause:** Previous creation fixtures explicitly selected Flareon/Implement, while the normal Eevee entry retained Plan. Native coding also exposed only full-file write, without project discovery/read/targeted edit; optional `/paths` still routed ordinary Eevee away from its coding flow.
- **Solution:** Resolved implicit supported-route coding at ordinary and advanced preparation, preserved explicit Plan/Review, synchronized restored form policies, and admitted local restrictions from Eevee. Added bounded real list/read/write/read-hash edit workflows, truthful read/write disclosures and shared exact mutation consent. The revised runtime driver begins without implementation-role setup and checks actual bytes, not model claims.
- **Prevention / Reference:** Verify first-use entry separately from executor correctness; keep explicit read-only and unsupported-route checks. Native/controlled fixtures do not establish paid/live-model conformance.

### [2026-10-22] Continuous output hid continuation text and compact trailing lines

- **Context / Symptom:** Reusing an earlier assistant item after tool receipts could hide the final answer while following. Restored sessions could reuse another transcript's row offset; a one-row compact viewport followed a synthetic blank row after a terminal newline.
- **Root Cause:** Item-selection paging assumptions survived in arrival order, cached-anchor fallback and line splitting. Separating native display rounds also required preserving credential context from preceding rounds.
- **Solution:** Added chronological native round segments, kept cumulative credential redaction when projecting each segment, followed real continuations after tool receipts, and reanchored restored selected items. Ignored only the structural trailing newline row while preserving intentional blank lines. Actual PTY verification checks both-direction cross-response navigation, paused append/reflow and raw saved draft/caret bytes; surface-only Unicode comparisons normalize terminal graphemes rather than changing provider/file data.
- **Prevention / Reference:** Exercise compact single-row output and model-text → tool → final-text transitions. Do not require a particular paragraph to remain visible after a viewport-sized page move.

### [2026-10-22] Native Git filename spelling and obsolete patch receipts broke follow-up reads

- **Context / Symptom:** A controlled write created a decomposed-Unicode filename, but discovery omitted it and the next read refused it as absent/ignored. Workflow relay verification also found no retained author after an actual approved write.
- **Root Cause:** Git's macOS precomposition changed inventory spelling relative to native directory names. A downstream author decoder still expected the obsolete 12-KiB prose/diff receipt instead of current creation/replacement results.
- **Solution:** Requested native Unicode spelling for inventory without modifying repository/global configuration, retained ignore/protection checks and used logarithmic inventory prefix lookup. Emitted compact verified post-write digest receipts for mediated mutations and removed the obsolete prose decoder; provider claims, reads, failures and denials remain non-authoritative. Post-write hashes come from the already verified retained baseline, not a second inference/hash or parsed display wording.
- **Prevention / Reference:** Keep generated-file readback and exact dirty CRLF preservation in real controlled/native loops. Historical migration handling remains historical; current authorship requires current trusted producer metadata.

### [2026-10-22] Retained Windows publication handles blocked their own verification

- **Context / Symptom:** Native run `39641905695` failed ten Windows write/continuation regressions with `The process cannot access the file because it is being used by another process. (os error 32)` after publication. Linux Check also rejected a platform-unused collation enum variant.
- **Root Cause:** Publication correctly retained its DELETE-access staging handle for native identity and rename protection, but the new read-only verification handle did not share that existing DELETE access. Dropping authority to reread would have weakened the safety boundary.
- **Solution:** Allowed read-only verification handles to coexist with existing native access while retained writer/ancestor handles continued excluding competing writes/deletion. Applied actual platform cfg to the enum instead of suppressing warnings. Fresh run `39647421794`, attempt 1, passed Check and all five native test/build/license/package/offline-install rows on source `f748ce5e46da0e7f4e84df9af108a3acb334a563`; the failed package set was not promoted.
- **Prevention / Reference:** Windows share flags must accommodate retained handles' requested access, not merely the new handle's access. Retain the real Windows publication/identity regressions and require fresh exact-commit all-platform qualification after subsequent changes.

### [2026-10-22] Native exclusion spelling did not match SDK boundary syntax

- **Context / Symptom:** Valid exclusion prefixes such as `private/` and Windows `src\private` could pass host configuration validation but abort SDK policy construction before query admission.
- **Root Cause:** Host exclusions were component-based native prefixes, while IPC forwarded native spelling and the consumer incorrectly applied the stricter grammar of writable targets.
- **Solution:** Serialized normal components with `/` and validated exclusion-prefix structure separately from write-target safety. Trailing/native separators, reserved or unwritable names no longer disabled the policy; absolute/traversing/malformed boundaries still refused. Denied descendants, adjacent permitted generation and actual byte-preserving writes remained covered. Rust 1,135 and SDK 34 cases passed; fresh Check/all-five-native qualification passed in run `39650525709`, attempt 1.
- **Prevention / Reference:** Keep filesystem-native addressing separate from IPC syntax, and do not weaken exact target or protected-path checks to accommodate display spelling.

### [2026-10-22] Controlled follow-ups reused an incompatible thread and compact consent overflowed

- **Context / Symptom:** Actual wide Codex creation denied neutrally, then the next ordinary Request was blocked because a resumed thread could not carry the dynamic write tool. At 60×18, a long auxiliary option-effect sentence consumed another row and hid Allow even though exact file/parent scope was visible.
- **Root Cause:** Composer submission reused every owned non-Review thread, including controlled Implement. Permission producers supplied verbose option effects on top of complete immutable operation/path/parent headers.
- **Solution:** Prepared a fresh controlled Codex thread for ordinary Implement follow-ups while retaining explicit resumed-implementation refusal. Shortened once-only option effects without truncating any scope or content. Frozen candidate B passed actual deny-then-create flows at 120×36 Unicode and 60×18 ASCII in English and Korean; unrelated existing bytes remained unchanged.
- **Prevention / Reference:** Retain the two consecutive controlled turns and compact approval checks in `scripts/file_creation_smoke.py`. Original failures are preserved in `target/file-creation-a-{en,ko}`; passing focused proof is `target/file-creation-b-controlled/results.json`.

### [2026-10-22] Conservative name admission rejected distinct Unicode files

- **Context / Symptom:** The integrated native coding regression refused `nested/empty.txt` and `nested/日本語.txt` before any mutation on macOS, although these were distinct targets.
- **Root Cause:** Unknown Unicode collation was treated as established aliasing. Separately, global post-publication scope verification could turn an unrelated selected-path change into a false uncertain result after a known exact write.
- **Solution:** Rejected established native/ASCII and macOS normalization aliases during read-only admission without merging path identities. Unproven collation retains exact frozen names and atomic no-overwrite enforcement; an observed alias becomes stale, never a replacement grant. Post-write validation now checks the retained written object/content/name rather than unrelated targets. All 1,111 Rust tests passed with two ignored after integration.
- **Prevention / Reference:** Native filesystem collation is not universal Unicode lowercasing. Unknown collations may be rejected later as stale, but must not authorize overwrites or mislabel known writes as ambiguous effects.

### [2026-10-22] Failed SDK writes could leave untracked parent directories

- **Context / Symptom:** A newly supported nested Write could create parents or partial content and then return an SDK tool error. Accepting that result allowed later successful model text to terminate normally despite unresolved local effects.
- **Root Cause:** The policy cleared the approved operation on every failed tool result without comparing the actual root, ancestor and leaf state against the frozen pre-operation fingerprint.
- **Solution:** Accepted failed writes/edits as settled only when their fresh filesystem fingerprint was unchanged. Changed or unreadable state now follows the production bridge fatal/host unknown path; failures never seed successful sibling-parent authority. All 29 SDK policy cases passed, including actual production bridge IPC with a synthetic SDK proving later success could not overwrite the fatal outcome. No live model or original credential was used.
- **Prevention / Reference:** Keep provider tool success/failure separate from established filesystem effects. External SDK policy is observational, not atomic OS confinement.

### [2026-10-22] Installed-terminal idle sampling preceded startup stream quiescence

- **Context / Symptom:** The installed 100×30 missing-image case emitted two synchronized-update/reset/cursor-only frames during its idle sample. Before/after text and caret were unchanged; no continuing animation or production heartbeat was established.
- **Root Cause:** A committed task header and a completed initial frame did not establish completion of asynchronous startup caches. The 120 ms stream boundary could admit a half-second idle sample before later startup output had drained.
- **Solution:** Required an observed half-second quiet-stream boundary, bounded by the existing two-second settling deadline, before starting the unchanged fixed half-second zero-output measurement. Continuous repaint still fails settlement; any output inside the measurement still fails, without filtering or restarting its window. The complete actual-package check passed all 23 lifecycle groups and 34 installed terminal scenarios in `target/release-0114-installed-c`.
- **Prevention / Reference:** Preserve actual control bytes and both captures when timing fails. The original evidence remains under `target/release-0114-installed-b/terminal/missing-cells-100x30`; no production code, tag or public asset was changed for this calibration.

### [2026-10-22] Real installer-upgrade fixture omitted generated-catalog build inputs

- **Context / Symptom:** The actual public-package lifecycle check passed fresh installation and ownership/integrity refusals, then its real older-version source fixture failed with `environment variable OUT_DIR not defined at compile time` and cascading missing catalog types.
- **Root Cause:** The fixture copied `src`, manifests and installers but omitted the newly required `build.rs` and `locales` source inputs. The published executable and installer were not the failing boundary.
- **Solution:** Copied the real build script and locale resources into the isolated source fixture, without copying generated output or relaxing compilation. The complete actual public-package check then passed all 23 installation/update/uninstall/artwork groups and 34 installed terminal scenarios, including a real older-version build and live owned-binary update. Original failure logs were retained; published assets were unchanged.
- **Prevention / Reference:** Keep real source-build fixtures complete when introducing compile-time resources. Original evidence: `target/release-0114-installed/upgrade-build.log`; complete passing proof: `target/release-0114-installed-c/results.json`.

### [2026-10-21] Windows qualification rejected valid native failure-location paths

- **Context / Symptom:** Native run `39618172959` passed Check and four native package rows, but Windows failed two record tests. The observed paths were `src\first.rs` / `src\second.rs`; assertions expected literal `src/first.rs` / `src/second.rs`.
- **Root Cause:** Consumer regressions pinned POSIX serialization rather than native filesystem path identity. Windows correctly retained its native separators; production parsing, containment and failure aggregation were not the failing boundary.
- **Solution:** Removed the three separator-string assertions and compared borrowed `Path` identities instead. Retained exact admitted-path counts, both distinct contained locations, rejected traversal/secret locations and the remaining location after output purge. Production paths were not rewritten. Fresh run `39621114639`, attempt 1, passed Check and all five native test/build/license/package/offline-install rows on `ef70fc168262c273eeccccd9662f9568a1ec93fc`. The initial run's partial package set was not used.
- **Prevention / Reference:** Test path identity semantically across platforms. Require Check and all five native rows for one exact commit/attempt; do not promote a partial green matrix.

### [2026-10-21] Ranking fixture edited a file before the process baseline settled

- **Context / Symptom:** A seeded fixture ranked every Python file as changed, or missed the intended `changed.py` edit in the compact Korean journey.
- **Root Cause:** Repository refresh deliberately retains the process's first baseline. Seeding a commit after that baseline counted fixture setup as work; editing immediately after restart could instead enter the asynchronous initial baseline.
- **Solution:** Reopened after fixture seeding and waited for an actual completed repository refresh before making the one working edit. Kept the production baseline and ranking rules unchanged. Actual nine-form ranking passed in English at 120×36 and Korean at 60×18 with the same five eligible matches, distinct failure/diff priorities and visible reasons.
- **Prevention / Reference:** Synchronize fixture mutations with observed worker completion, not task-storage acknowledgment or sleeps.

### [2026-10-21] Removing a restored pairing selected a stale global default

- **Context / Symptom:** An actual restart/removal journey returned to Codex/Default although the acknowledged global preference was a compatible-provider model. The regression failed with `left: Codex, right: Compatible`.
- **Root Cause:** Late global hydration updated a parked slot's model map but not its fallback route. Applying a pairing then captured that slot's passive current route. New partners could also overwrite a freshly merged model with an older current selection for the same provider.
- **Solution:** Kept the committed global route independently of pairing presence, preserved explicit startup overrides, and invalidated the cache on an actual global choice/use. Seeded new partners from that route's selection, not the parked slot's current model. Both consumer regressions passed in the 1,049-test suite; actual two-partner restart/removal passed in English at 120×36 and Korean at 60×18 on candidate F.
- **Prevention / Reference:** A passive current selection is not a new global preference. Preserve dirty user choices while separately reconciling acknowledged global defaults.

### [2026-10-21] Live accounting qualification waited for an obsolete preview caption

- **Context / Symptom:** The actual Codex qualifier reported `TUI prerequisite unavailable: LOCAL PREVIEW` after successful metadata discovery and retained request input, without dispatching a model turn.
- **Root Cause:** The driver treated a nonempty Request's slash as a command, treated the model/reasoning picker like the ordinary workspace, and expected an old technical caption instead of the current owned approval queue and compact request-scope card. The pending request was not a provider transport failure.
- **Solution:** Used the global chooser outside pickers, committed the visible model/reasoning transaction, deliberately opened `/approvals`, and verified the actual authored request, exact model, Plan role and Run control. Removed temporary diagnostics instead of changing provider-generation or ownership guards. Zero-turn preparation/cancellation passed, followed by exactly three authorized real Codex 0.147.0 turns and two actual owned-session resumes.
- **Prevention / Reference:** Gate consent on current scope and user data, not an incidental heading. Sanitized evidence retained counts/scopes and salted identities only; original auth/config files remained unchanged and temporary credentials were removed. All three authorizations were consumed; no further paid turn is authorized.

### [2026-10-21] Literal Unicode instruction paths inherited another prompt's memory

- **Context / Symptom:** A consumer regression failed with `distinct literal path bytes inherited another instruction's approved memory` when composed and decomposed Unicode filenames appeared in otherwise identical authored instructions.
- **Root Cause:** NFC and line-ending normalization changed code-sensitive interior bytes before deriving the repeated-instruction key. Canonical Unicode equivalence does not imply filesystem or programming-language equivalence.
- **Solution:** Trimmed outer whitespace only, validated the unchanged interior text before taking ownership, and hashed its literal bytes. Unsupported interior controls made the instruction ineligible rather than rewriting it. Original drafts and history remained unchanged. The regression failed before the change and passed in final candidate E's 1,047-test suite.
- **Prevention / Reference:** Treat authored source paths, code and spacing as literal data. An approved memory must not cross a different instruction identity.

### [2026-10-21] Settled non-Git source inspection kept connection setup busy

- **Context / Symptom:** Actual tooltip navigation opened Files in a non-Git project, then could not enter compatible-provider connection setup even though local storage had settled.
- **Root Cause:** An absent repository catalogue always enabled `await_refresh`, including unsupported inspection with no queued or running job that could ever finish it.
- **Solution:** Passed the actual queued/loading inspection state into `Sources::catalogue`. Real pending work retained its busy guard; settled unavailable inspection became an honest unavailable result. Root busy/refusal regressions and both-size English/Korean tooltip journeys passed on D and E.

### [2026-10-21] Record consent could bury ownership and ignore the neutral shortcut

- **Context / Symptom:** Long retained errors could push the failed operation's identity into optional pages, while the displayed `n` shortcut did not select Stay.
- **Root Cause:** Consent relied on reaching the end of a detail viewport rather than rendering the complete bounded scope independently. The shortcut had no matching admission path.
- **Solution:** Pinned the exact operation/request/task/partner or historical signature/support/pass scope and its effect above optional details. Confirmation remained disabled when the full scope could not fit. Unmodified `n` selected Stay; `y` required the current visible scope. Long-error, other-owner, tiny-view, neutral-return and explicit-confirmation regressions passed in candidate D's 1,046-test suite.
- **Prevention / Reference:** Never truncate identity to enable consent, and never let paging an error stand in for reviewing the affected operation.

### [2026-10-21] Fixed page strides skipped compact details and displaced learned moves

- **Context / Symptom:** Actual compact source inspection skipped unseen detail rows; verbose artifact markers occupied the move-name preview in the form picker.
- **Root Cause:** Generic paging used hard-coded strides instead of each rendered viewport. Unbounded factual marker descriptions preceded the compact move inventory.
- **Solution:** Recorded the actual visible row count for every generic scroll surface and paged by that count minus one overlap row. Compact markers retained factual status/count/producer on one clipped line; full facts stayed inspectable, and learned moves retained their preview space. Both-size detail-reachability regressions and all 26 actual Moves groups passed on candidate C.

### [2026-10-21] Factual activity formatting could fail its own privacy admission

- **Context / Symptom:** The terminal-run recovery regression reopened a freshly written store with `Invalid activity metadata`.
- **Root Cause:** Command arguments had already been redacted, but assembling a human-readable activity sentence could create a new credential-shaped fragment. The final sentence had not passed the same durable privacy projection.
- **Solution:** Redacted the final serialized activity detail while borrowing unchanged text. Kept strict admission intact and retained nonsecret failure facts. The database/reopen regression and the 1,041-test integrated candidate-A suite passed.
- **Prevention / Reference:** Validate the final durable human-readable projection, not just its input fragments. Do not relax admission or persist plaintext to satisfy a fixture.

### [2026-10-21] Usage refinement and asynchronous terminal receipts needed distinct boundaries

- **Context / Symptom:** Integration review found that newly reported usage identities could be rejected after an earlier unknown report, running command updates could enqueue terminal-only observations, and a ready handoff could be saved after the task had advanced to a later revision.
- **Root Cause:** Unknown-to-known refinement is not a conflicting identity; worker progress is not a terminal receipt; receipt provenance is not necessarily the coalesced save revision.
- **Solution:** Admitted one-way usage metadata refinement while rejecting changed known identities, retained unsuccessful native response reports, observed only actual started terminal runs using their prepared task identity, and persisted typed historical handoff revisions with exact receipt validation. Serialized destructive context actions behind unacknowledged local receipts. Behavioral regressions passed in candidate A.
- **Prevention / Reference:** Keep failed writes attached to their original work owner. SDK cumulative resume accounting follows the locked SDK's published contract and is not live-account qualification.

### [2026-10-21] Metadata-only quit could hide a failed owned write

- **Context / Symptom:** Review of local-record persistence found that preference waiting could obscure a failed owned record operation during quit, while clearing the immediate quit request lost the eventual exit intent.
- **Root Cause:** Preference settlement ran before the record gate, and the record gate had no independent sticky exit state.
- **Solution:** Gated records before preferences, retained the exit request until acknowledgement, and cleared all exit flags only on explicit Stay. Failed operations remained inspectable and owner-bound; repeated-gate and neutral-return regressions passed in candidate A.

### [2026-10-21] Alt response navigation was blocked by the generic modifier guard

- **Context / Symptom:** Response-navigation work exposed a regression in Alt-modified response controls before the source-only 20-case response qualification passed.
- **Root Cause:** Response-specific navigation needed admission before the generic editor guard that rejects Control/Alt input; otherwise legitimate response keys could not reach their owner.
- **Solution:** Routed exact Alt+Up/Down/Home/End and unmodified paging through the guarded response surface before ordinary editor modifier refusal. Kept reviews, overlays and draft/caret input ownership intact; ordinary editing keys did not become response commands.
- **Prevention / Reference:** The earlier source-only response qualification passed 20 cases after the fix. This is not proof for the final integrated v0.1.14 candidate: rerun wide/compact English/Korean reading, draft preservation, pause/latest and terminal restoration on the final bytes. The existing native-Windows C-toolchain limitation below remains a separate cross-check boundary, not native runtime evidence.

### [2026-10-21] Manual release qualification required the supported Python runtime

- **Context / Symptom:** Importing the release qualifier from the analysis host failed with `ModuleNotFoundError: No module named 'tomllib'`.
- **Root Cause:** `release_assets.py` used standard-library TOML support available in the declared CI Python runtime, but not in the analysis interpreter. This was an interpreter mismatch, not a release-artifact or qualification failure.
- **Solution:** Invoked the unchanged strict qualifier through the existing `target/waves-venv/bin/python`. It validated commit `7df8512adb7738fa898dbbe9583e8414301a55d8`, native run `39542438158` attempt 1 and all five immutable artifact IDs; publication then promoted those IDs successfully.
- **Prevention / Reference:** Use the supported Python runtime for release scripts. Do not relax qualification checks, rewrite TOML parsing or install a substitute into system Python to work around the caller's interpreter.

### [2026-10-21] Routine-admission fixtures respected the storage privacy projection

- **Context / Symptom:** The actual smoke's `exact JSON argv saved` assertion waited for a recognizable credential-like argument to appear verbatim in SQLite. The store correctly persisted a masked command instead.
- **Root Cause:** A command's raw live bytes and its privacy-filtered durable projection are not interchangeable. The fixture had asked the existing command-save barrier to acknowledge plaintext that storage intentionally did not retain.
- **Solution:** Preserved redaction and did not claim those private argv were executed. Used nonsecret environment-assignment and out-of-project argument routines for actual approved-run admission checks; kept secret/masked-value refusal in typed-domain and committed-store regressions. All 26 final actual-TUI Moves groups passed without test SQL writes.
- **Prevention / Reference:** Never disable privacy projection, persist secrets for a test, fabricate a journal or require a redacted execution fingerprint. Evidence: `target/wave5-moves-20261021-e/results.json`.

### [2026-10-21] A learning prompt swallowed navigation while recording its display

- **Context / Symptom:** Actual 120×36 and 60×18 journeys selected Never immediately after a six-use prompt, but opened the naming editor instead. The first Down event arrived while the prompt's durable Offered receipt was pending and was discarded.
- **Root Cause:** A blanket move-persistence input guard treated prompt-display bookkeeping like an inventory mutation. Its acknowledgment could arrive between consecutive keys, changing which option the remaining keys selected.
- **Solution:** Distinguished a single pending Offered receipt from blocking mutations/failures. Kept navigation, Unicode typing/paste and explicitly reviewed local choices responsive; later decisions remained FIFO-ordered behind the receipt. Added a three-choice pending-receipt regression, observed its RED `Learn` versus `Menu` failure, then passed it in the **926-test** integrated suite.
- **Prevention / Reference:** The actual journey driver retained immediate selection without an artificial delay; all 26 final wide/compact groups passed. It compared observed revisions to real journals rather than requiring a nonexistent execution revision for pre-run denial and used the checklist's actual Enter control for the template reset case. No missing evidence was fabricated.

### [2026-10-21] A refused older writer could not settle its startup preferences

- **Context / Symptom:** The actual public v0.1.12 executable created Schema 13 facts, the Wave 5 candidate migrated them unchanged to Schema 14, and the old executable displayed `Storage: save failed`. The migration driver then failed with `application neither exited nor offered exit review`.
- **Root Cause:** The published older runtime retained failed startup provider/model preferences against an unsupported newer store, so its ordinary graceful-exit persistence guard could not settle. This was not a failed migration or permission to downgrade; both executables still reported 0.1.12 because Wave 5 source was unreleased.
- **Solution:** Ended only the driver's owned older-writer refusal probe explicitly, recorded that old-runtime graceful exit was not asserted, and checked the migrated database byte-for-byte after teardown. The new runtime still had to exit normally, preserve all exact task rows and resume after the refused downgrade. All three actual Schema 13→14 migration/refusal/resume checks passed.
- **Prevention / Reference:** Back up before opening an older store with newer source. Restore a backup for older software; in-place downgrade is unsupported. Final evidence: `target/wave5-schema-20261021-c/results.json`; driver: `scripts/schema_upgrade_smoke.py`.

### [2026-10-21] Move dispatch and asynchronous review ownership needed authoritative receipts

- **Context / Symptom:** Integration review found that accepting a provider request into a worker queue was earlier than actual dispatch, a closed/reopened move sheet could adopt a late validation reply, and a failed move preparation could otherwise be confused with a later identical manual command.
- **Root Cause:** Queue admission, view identity and prepared-command identity are different boundaries. SDK iterator release likewise does not prove a submitted model turn.
- **Solution:** Added exact request-bound actual-dispatch events for native response headers and completed Codex/ACP prompt writes; required an admitted fresh-UUID-correlated SDK acknowledgment through bridge revision 4. Persisted pin receipts before releasing the work claim. Invalidated validation on leaving the owning sheet and bound acknowledgments to task/revision/partner/form/mode; tagged actual move preparations so a manual command cannot inherit move attribution.
- **Prevention / Reference:** Rust behavioral regressions and all 17 credential-free SDK policy checks passed. Transport receipts do not prove model completion, billing or live-account compatibility.

### [2026-10-21] Purged learning sources and portable paths exposed persistence boundaries

- **Context / Symptom:** Review found that deleting source tasks could leave retained candidates stale below the three-use threshold, blocking strict reopening; command validation also needed to distinguish native argument paths from portable selector metadata.
- **Root Cause:** Candidate updates had been limited to newly eligible counts. Assigned credential flags, bare relative symlink arguments and Windows-native paths do not all have the same syntax as serialized project-relative selectors.
- **Solution:** Refreshed existing candidate counts/provenance even below threshold, retained zero-source reminders while project tasks remained and adjusted postponement for removed sources. Kept portable slash-delimited metadata separate from native path equality, reused shared secret redaction for assigned flags and checked bare/assigned path arguments for links while preserving literal argv and ordinary executable lookup. Worker-side filesystem checks and fresh runner identities remained mandatory.
- **Prevention / Reference:** Strict store reopen/purge/threshold and secret/path/link behavioral regressions passed in the Rust suite. Windows-native runtime behavior remains unqualified on this Darwin host.

### [2026-10-21] Public Windows license had valid CRLF bytes

- **Context / Symptom:** Public asset digests, checksum manifest and archive layouts passed, but a cross-platform byte comparison of the packaged Windows `LICENSE` against the POSIX checkout failed.
- **Root Cause:** Windows checkout used 201 CRLF line endings: 11,525 bytes versus the POSIX file's 11,324 bytes. LF-normalized content was exactly equal; the archive was not corrupted.
- **Solution:** Kept exact raw SHA-256 and GitHub digest validation for every asset and compared license text using line-ending equivalence only. Completed installer identity, archive safety, backup-note and actual downloaded executable checks without changing published bytes.
- **Prevention / Reference:** Never normalize bytes before checksum verification, alter immutable archives, or infer cross-platform license corruption from valid checkout line endings.

### [2026-10-21] Clean startup exit offered cancellation for nonexistent work

- **Context / Symptom:** Resuming an actual Schema 13 task and immediately pressing Ctrl+C showed “Quit / ALL owned work” with an empty owner list, despite saved task facts and idle request/command state.
- **Root Cause:** Quit reused readiness aggregates that included provider preference persistence and optional execution history reads. Brief local persistence was classified as work requiring cancellation rather than a commit/read acknowledgement.
- **Solution:** Separated work-review admission from local persistence waiting. Pending sends/commands, provider/request state, actual command state, unknown outcomes and failed journals still use their guarded decisions. A clean task quietly waits for local preference/history settlement before exit.
- **Prevention / Reference:** The preference-acknowledgement regression retains the exact exit wait without a cancellation request. Actual v0.1.11→v0.1.12→v0.1.11 Schema 13 continuity preserved task facts; the current runtime exited without the empty review. Same-schema reopening is not older-schema refusal.

### [2026-10-21] Native response identities collided after transport recreation

- **Context / Symptom:** Exact follow-up HTTP payloads were correct, but the current partial/complete answer appeared under the first request's label and replaced its retained display item.
- **Root Cause:** Tool-free transports were recreated between turns, resetting their local generation. Display turn/item IDs used only that generation, so separate owned requests reused the same identity.
- **Solution:** Correlated native display turn/items with the immutable durable work request ID; streaming updates within that request still replace the same item. Provider generation remains transport-local, and no provider session or remote turn is invented.

### [2026-10-21] Automatic refusal disposal hid native scope recovery

- **Context / Symptom:** External credential/endpoint drift correctly sent no HTTP request, but the worker's subsequent “Native preview discarded” message replaced the actionable refusal and `/chat-new` recovery detail.
- **Root Cause:** Both foreground and parked ordinary-send admission canceled a blocked preview automatically, discarding its inspectable blockers.
- **Solution:** Retained the exact blocked preview without dispatch/retry, exposed blocked status and `/approvals` details, and allowed explicit local reset/cancel to dispose of it. Compact unknown-outcome status now retains the complete uncertainty phrase rather than clipping “unknown”.
- **Prevention / Reference:** Keep ordinary-send blocker regressions, exact frozen input, zero inference requests and explicit reset/cancel checks. Navigation metadata revisions are not credential effects; probes compare unchanged task facts, exact drafts/carets, claims and evidence instead of incidental timestamps.

### [2026-10-21] Direct memory selection reopened its own loading gate

- **Context / Symptom:** After explicit memory approval, `/select-memory` left zero selected notes and displayed “Loading approved memories; wait before editing.” The loaded record remained visible.
- **Root Cause:** Global direct actions called `open_overlay(Memory)` even when already in Memory; its load side effect ran before the existing same-overlay return, and the intended action was then rejected as loading.
- **Solution:** Started memory loading only when entering from another surface. Repeated selection/editing acts on the visible loaded record without new reads; ordinary entry still reloads and first-use authoring still waits for owned loading completion.
- **Prevention / Reference:** Regression covers actual selection/unselection and editing at both sizes, exact outgoing selected records, no implicit approval and no new load request.

### [2026-10-21] Terminal native uncertainty incorrectly retained live ownership

- **Context / Symptom:** Every native protocol closed its actual response socket on cancellation and displayed saved partial/unverified output, but explicit follow-ups remained blocked by the earlier unknown outcome.
- **Root Cause:** One sticky flag represented both unknown remote completion/billing and unresolved tool-capable effects. Ownership treated both as ongoing work even after the tool-free local transport and durable claim had settled. The runtime probe also incorrectly demanded the idle label instead of the truthful unknown-outcome status.
- **Solution:** Classified effect uncertainty from the immutable submitted request at terminal handling. Preserved remote-unknown disclosure, informed quit and canonical exclusion while allowing later explicit native sends/task changes after normal settlement. Kept CLI/tool effects blocking across route changes; fixed the probe to require actual socket closure, released claims, saved storage and partial/unknown status.
- **Prevention / Reference:** Never infer uncertainty kind from the currently selected provider, clear it through login, manufacture tool-journal rows, or treat local reader cancellation as proof of remote cancellation.

### [2026-10-21] Authentication fixture lost terminal restoration evidence after exit

- **Context / Symptom:** Both-size foreground login/callback/cancel/no-replay groups passed, but final `Session.wait_exit()` raised `termios.error: (25, 'Inappropriate ioctl for device')` after the application exited.
- **Root Cause:** The fixture exec'd Eevee as the controlling-session leader; macOS revoked the slave before the harness could sample restored termios. Fixed-count Escapes had also returned through a retained-Request review instead of reliably requesting exit.
- **Solution:** Requested structural safe exit with Ctrl+C and added an opt-in POSIX controlling-TTY supervisor for auth fixtures. It execs/reaps the actual application child, retains its real PID/status and keeps the controlling session alive until terminal attributes are sampled and acknowledged. Resize reaches the application group; cleanup reaps the supervisor.
- **Prevention / Reference:** Do not suppress ioctl errors or infer restoration from exit alone. Keep termios, cursor, bracketed-paste, alternate-screen and exit-status assertions alongside real foreground child/socket cleanup.

### [2026-10-21] Stable command cutover left contradictory surface guidance

- **Context / Symptom:** Actual surface audit found Help advertising ordinary sends as advanced reviews, execution recommending source-owned `/search`, Jolteon advertising inspection as approval and compact footers hiding Escape. An execution probe entered Sources instead of output search.
- **Root Cause:** Context-dependent commands had been replaced centrally, but some shared first-action text, footer branches and one smoke consumer still described the old workspace semantics.
- **Solution:** Made Jolteon's primary action direct command authoring, distinguished `/run` inspection from `/execute` permission review and `/search` source filtering from `/output-search`. Updated advanced session guidance and the affected consumer; bounded compact footer variants retained navigation and exact draft/caret return.
- **Prevention / Reference:** Inspect actual wide/compact surfaces as well as command dispatch. Label-copy assertions do not prove that the visible action reaches the right behavior.

### [2026-10-21] Native follow-ups lost conversational roles and ordinary intent

- **Context / Symptom:** Native setup exposed instructions rather than a private credential editor, and ordinary follow-ups were serialized as isolated planning input instead of the preceding user/assistant conversation.
- **Root Cause:** Configuration discovery was the only connection path; request construction reused display-oriented planning text without a canonical successful-turn history.
- **Solution:** Added explicit masked setup and route/model selection. Captured authoritative completed public responses in bounded process-local role-bearing history, froze it with each input and serialized it separately from system/task/source context. Kept ordinary answers distinct from requested plans and managed reviews; excluded partial, reasoning and display text.
- **Prevention / Reference:** Verify exact roles and raw content on real HTTP requests, including cancellation, incomplete output, scope drift, local chat reset and task/partner isolation. Never rebuild conversations from provider-tool journals or UI items.

### [2026-10-21] Private connection navigation lost keys during background saves

- **Context / Symptom:** Actual native setup occasionally skipped private fields after Tab/Ctrl+U; the endpoint contained the model value and the model was blank. The TUI reported that saves or active work had to finish.
- **Root Cause:** The application applied global idle admission to every private-form key, including harmless edits and navigation, while task autosaves briefly remained pending.
- **Solution:** Kept all private typing, paste, navigation and neutral cancellation inside the private editor. Applied saved/idle admission only to deliberate Save or reviewed affirmative Forget; repeats/releases still cannot approve effects.
- **Prevention / Reference:** Retain pending-save input regressions and actual field-focus assertions at both supported terminal sizes. Do not hide lost input with sleeps or retries.

### [2026-10-21] Neutral authentication cancellation returned to the wrong page

- **Context / Symptom:** Reopening the same sign-in method after neutral Back appeared stuck at “Preparing connection review…”, despite a functioning preparation worker.
- **Root Cause:** Cancellation reset Methods to Providers. The next Enter reopened Methods instead of preparing the selected method, while the previous preparing notice remained visible.
- **Solution:** Retained the originating page and selected method, invalidated the canceled generation and cleared its preparing notice. Late worker completion remains unable to authorize or reopen canceled work.
- **Prevention / Reference:** Exercise prepare, cancel, stale completion, fresh preparation and neutral Back before any explicit authorization; never compensate with an extra hidden Enter.

### [2026-10-21] Credential privacy required anchored paths and native ACL checks

- **Context / Symptom:** Review found that a checked credential path could be redirected through a replaced parent link, and POSIX mode checks alone did not establish privacy on macOS ACL-bearing objects.
- **Root Cause:** Path validation and opening were separate operations, and Darwin ACL permissions were not included in the handle-bound privacy snapshot.
- **Solution:** Anchored no-follow reads to the verified destination, checked owner/mode/native ACL through the opened handle and failed closed on unsupported inspection. Removed inherited ACLs only from newly created app-owned objects; never repaired HOME or provider-owned files. Kept failed-create cleanup from deleting existing Windows targets.
- **Prevention / Reference:** Keep credential/link/ACL fixtures isolated from real accounts. Plaintext persistence is explicit, not encryption or keychain storage; cleanup failures must disclose retained bytes.

### [2026-10-21] Saving connection defaults discarded configured reasoning effort

- **Context / Symptom:** Editing native connection settings could drop a previously configured reasoning effort or carry it onto a different model without explaining that association.
- **Root Cause:** The new saved profile was reconstructed from visible fields without preserving the original model/effort binding.
- **Solution:** Preserved the configured binding when the model stayed unchanged, visibly reset it on deliberate model changes and refused mismatched associations at dispatch.

### [2026-10-21] Direct actions lost asynchronous intent and editor return ownership

- **Context / Symptom:** First source actions disappeared while catalogues loaded, specialist launch failures immediately closed, restored tasks returned to another composer's context and model selection disappeared behind required provider-switch review.
- **Root Cause:** Direct navigation reused synchronous workspace assumptions, interpreted the initiating Enter as editor completion and did not bind deferred return/picker intent to its exact task/partner/route.
- **Solution:** Retained owned source actions until catalogue/context readiness, canceled stale intent, captured pre-key specialist editing state, rebound restored-task returns and carried route-bound picker intent only through an accepted matching switch review.
- **Prevention / Reference:** Keep exact Request draft/caret and unchanged-effects assertions; direct entry removes optional navigation, not required approvals.

### [2026-10-21] Compact sheets obscured live context and question content

- **Context / Symptom:** Modal entry covered storage/refusal feedback; long technical correlation IDs appeared before the actual question/options at the minimum terminal size.
- **Root Cause:** Independent sheets did not consistently reserve live header/footer rows, and metadata was ordered ahead of the user's decision.
- **Solution:** Used shared two-header/two-footer sheet bounds, retained actual storage state rather than a stale stamp and moved technical IDs after question/options/answer content. Full details remain keyboard-inspectable without weakening visible-summary approval gates.

### [2026-10-21] Bounded fixture version probe expired during runtime qualification

- **Context / Symptom:** The first E execution-relay run stopped after 10 of 14 groups: the deterministic peer's version probe exceeded the application's three-second deadline at 60×18 (`target/wave4-accepted-execution-relays-e`).
- **Root Cause:** Not established. The fixture's version branch only printed its fixed version; the observation did not establish a production protocol defect.
- **Solution:** Preserved the failure and manually ran the unchanged complete suite in fresh isolated disposable processes; all 14 groups passed on identical E bytes (`target/wave4-accepted-execution-relays-e-isolated`). No provider request retry, harness workaround, admission bypass or timeout change was added.
- **Prevention / Reference:** Keep the bounded fail-closed probe and original evidence. A successful isolated run is not a claim that the timeout's cause was proven.

### [2026-10-21] Compact history hid acknowledged provider coverage loss

- **Context / Symptom:** Three explicit resumed native turns reached the real 256-record journal limit. Terminal-only discard correctly kept committed rows and released ownership, but 60×18 history showed only the positive acknowledgement; the unverified gap was invisible (`target/wave4-cooperation-c-reachable/60-journal-limit/initial/final.txt`).
- **Root Cause:** The bounded history footer applied `brief` to a long storage acknowledgement while the coverage-loss warning was appended afterward.
- **Solution:** Prioritized the existing gap as the compact notice, made its first sentence explicitly say write attempts were not saved and were not verification, and appended its full memory-only owner/request/error detail to the scrollable history presentation. No stored observation was rewritten or invented.
- **Prevention / Reference:** Keep the post-discard visible-warning assertion alongside exact terminal-only recovery, 256 unchanged committed rows, deliberate neutral consent and no provider replay.

### [2026-10-21] Compact command editor hid a correct capacity refusal

- **Context / Symptom:** With three real owned local commands at 60×18, applying a fourth correctly refused execution and retained its draft, but no reason appeared. `target/wave4-accepted-cooperation/60-local-slots/initial/final.txt` preserved the failure.
- **Root Cause:** The centered command editor covered screen rows 1–16, including the global notice row. The next key cleared the old notice before closing the editor, so Escape could not reveal it afterward.
- **Solution:** Used the existing specialist-desk layout boundary: reserve two header and two footer rows before centering the editor. Task/storage/refusal/exit text remains outside the modal; the existing caret-driven input viewport retains the complete draft without changing the capacity guard.
- **Prevention / Reference:** Keep the actual fourth-command visibility assertion at the minimum terminal size; absence of a process alone does not prove an actionable refusal.

### [2026-10-21] Native graphics input fixture reopened the wrong editor

- **Context / Symptom:** The native graphics smoke failed `typed text was lost, duplicated or polluted by terminal replies` after successfully decoding a real sprite.
- **Root Cause:** Its pre-first-use-UX flow typed into the Request composer, then reopened Goal after Help and expected that different field to contain the same text. Terminal replies had not corrupted the input.
- **Solution:** Opened Goal explicitly before typing and resumed that same retained field after Help. All five actual PTY routes then passed input persistence and terminal recovery: Sixel, Kitty, unsupported-native cell fallback, explicit cells and images-off.
- **Prevention / Reference:** `scripts/graphics_smoke.py`; decoded native subject pixels remained at integer scale 3, with maximum RGB error 1 for Sixel and 0 for Kitty. Capability responders are not GUI-terminal certification.

### [2026-10-21] Editing-owner banner painted over local command consent

- **Context / Symptom:** The actual mixed reader/writer/command journey displayed `Ember (Flareon) is editingpermission` where the command permission title belonged. The full review locator failed even though the neutral choices and exact command remained below.
- **Root Cause:** A queue wrapper drew the lease banner after rendering the confirmation, directly on its first content row.
- **Solution:** Removed the post-render wrapper. Queue identity became the panel title; the editing owner used an existing consent-summary row without hiding command identity, directory, executable, effects or invalidated-request errors. The compact six-row approval budget and exact-detail scrolling were preserved.
- **Prevention / Reference:** The actual mixed-work journey reached approval and started its exact owned command after the correction; both-size full cooperation qualification remained required. Deferred command smokes deliberately reopened `/approvals`; native-provider denial used the visible neutral Deny choice, not a nonexistent `/deny` palette command.

### [2026-10-21] Parallel context fixtures collided on a timestamp-only directory name

- **Context / Symptom:** The native integrated suite failed a context fixture at `fs::create_dir` with `AlreadyExists (code 17)` while creating a PID-and-nanosecond-named directory.
- **Root Cause:** Wall-clock timestamps did not guarantee unique names for parallel fixture creation in one process.
- **Solution:** Added a process-local atomic counter alongside the PID and timestamp, preserving canonicalization, isolation and the original parallel test execution.
- **Prevention / Reference:** `workspace::context::tests::Fixture::new`; no production path, guard or timeout was relaxed.

### [2026-10-21] Idle transport retirement lost resumable ownership

- **Context / Symptom:** Cooperative runtime review found that a completed request retained its thread/session reference while the idle provider worker—and its backend ownership registry—was dropped. The next Resume could therefore refuse an otherwise owned session. Parked states could also retain old authentication catalogues or future-launch credentials.
- **Root Cause:** Retirement used only transport activity, and authentication updates reached only the front state.
- **Solution:** Retained bounded owner workers while current or cached resumable sessions existed; propagated account invalidation and future credential replacement/clearing to parked states without mutating submitted immutable input. Revoked sessions no longer kept obsolete workers alive.
- **Prevention / Reference:** Consecutive same-owner readonly turns and parked-authentication regressions; no automatic resend or session adoption.

### [2026-10-21] Full provider journal prevented ownership settlement

- **Context / Symptom:** A definitive provider completion with a failed 257th journal record retained the exact work claim. Storage-only retry could not overcome the 256-record limit, while active ownership correctly refused purge, leaving ordinary settlement impossible.
- **Root Cause:** Failed unsaved observations had no explicit terminal recovery distinct from retry or deletion of committed history.
- **Solution:** Added neutral `/journal-discard` review bound to owner/task/request and every exact failed FIFO write attempt, including repeated IDs/partial/final versions. Only provably unsaved failures after definite termination could be acknowledged; committed rows remained unchanged, an unverified memory-only coverage-gap/error notice remained, and ordinary exact-claim release completed settlement. Running/unknown work and ambiguous commits/reads/purges refused recovery.
- **Prevention / Reference:** Real 257th-record store regression and paced actual protocol fixture; recovery never invented successful persistence, purged history or replayed a provider.

### [2026-10-21] Releasing the current idle lead conflicted with live autosave state

- **Context / Symptom:** Review found that a committed release and subsequent front switch could share one revision with differing draft values. Removed recipient intents also remained in live workflows and could reappear on the next save.
- **Root Cause:** Release completion copied roster/task metadata without reconciling live specialist intents or separating committed cleanup from additional front/navigation changes.
- **Solution:** Paused autosave during the release transaction, applied committed recipient-intent pruning, cleared only the removed lead's specialist proposals, and saved additional live changes strictly above the committed revision while retaining peers, transports, unrelated drafts and carets.
- **Prevention / Reference:** Current-lead release with three working peers, recipient-intent resurrection and preserved-evidence regressions.

### [2026-10-20] Empty cooperation recovery rewrote an idle current-schema database

- **Context / Symptom:** The first integrated wave 4 suite failed `current_schema_reopens_without_rewriting_or_inventing_activity`; reopening and saving the identical task changed the SQLite change counter despite no reserved execution or active work.
- **Root Cause:** Startup recovery unconditionally executed `DELETE FROM active_work`, including an empty table. SQLite still performed a write.
- **Solution:** Checked for either reserved executions or persisted ownership before opening the recovery transaction. A quiescent store now returns through the read-only path; real recovery still interrupts evidence and clears stale claims atomically under the exclusive writer lock.
- **Prevention / Reference:** Retained the byte-preserving reopen regression and real previous-binary refusal qualification. Attribution fixtures were separately aligned with actual roster identities rather than weakening work-owner validation.

### [2026-10-20] Cooperative consent and receipts used mutable or previous-request metadata

- **Context / Symptom:** Read-only integration review found that a deferred request A could display a later saved draft B while dispatching A, a new claim could expose its predecessor's receipt/model, and modified affirmative keys could confirm partner release.
- **Root Cause:** Legacy rendering read the current task draft; receipt fields lacked current-request ownership; release consent tested the key code without the modifier/Press boundary.
- **Solution:** Rendered the immutable submitted snapshot, bound current-request receipt/model disclosure to its identity, and required unmodified affirmative Press events after visible release review. Added regressions for A versus B, independent request receipts and Ctrl/Alt/Shift/repeat/release-key refusal.
- **Prevention / Reference:** Future drafts remain editable, but never replace reviewed input or its attribution. Unknown provider usage/cost must stay explicit, not become an earlier request's measurements.

### [2026-10-20] Compact specialist pagination skipped saved detail lines

- **Context / Symptom:** Final 60×18 inspection exposed only the beginning of the comparison warning. A new keyboard-only regression confirmed that paging skipped several user-authored hypothesis lines entirely.
- **Root Cause:** Page size came from terminal height, not the actual detail viewport. Narrow layouts reserved only three detail rows while PageDown advanced six rows.
- **Solution:** Reported the actual rendered viewport height as ephemeral `scroll_page` state and used it for normal specialist and relay paging. The saved-hypothesis regression failed before and passed after at both sizes; the integrated source passed 713 tests / 2 ignored, strict Clippy and release build.
- **Prevention / Reference:** The actual workflow smoke now inspected both linked run IDs, both factual durations and the no-speedup warning across reachable pages rather than stopping at the first `Single-run` substring. Old selected source captures were deliberately removed through normal reviewed UI before the independent source-free design request; freshness guards were not bypassed.

### [2026-10-20] Smoke isolation invalidated toolchain and overlay assumptions

- **Context / Symptom:** After isolating provider HOME, the Cargo smoke produced no source diagnostic. Fixed-count Escape batches in relay fixtures also left a different overlay or reopened a retained Request editor, so later command text could enter that editor instead of the chooser.
- **Root Cause:** Rustup's default data location followed the disposable HOME, while overlay depth depended on the actual navigation history. A fixed number of Escape presses did not establish a workspace boundary.
- **Solution:** Preserved explicit `RUSTUP_HOME` and `CARGO_HOME` for the real compiler fixture while keeping provider configuration isolated. Unwound observed overlay frames to the workspace footer before deliberate Request editing; checked committed ownership and empty recipient text before relaying. Both-size Cargo/location and owning-form journeys passed without changing production guards or skipping effects checks.
- **Prevention / Reference:** `execution_relay_smoke.py`, `owner_redirect_smoke.py` and the workflow fixture's `workspace`/`clear_request` helpers. Chooser fixtures also reset to the first row before selecting an evidence identity instead of inheriting an old highlight.

### [2026-10-20] Decision criteria preview reset scrolling before confirmation

- **Context / Symptom:** A 40-criterion preview regression could not reach the final criterion at 120×36. Repeated PageDown input left the first items visible.
- **Root Cause:** The compact recipe confirmation layout also applied to decision criteria and reset the shared scroll position on every render.
- **Solution:** Kept the complete criteria preview on the ordinary scrollable detail path while preserving compact recipe consent behavior. The same regression passed at 120×36 and 60×18, with every last item inspectable and no confirmation or provider dispatch.
- **Prevention / Reference:** `every_decision_criterion_remains_inspectable_before_confirmation`; full integrated verification passed 712 tests / 2 ignored plus strict Clippy and release build.

### [2026-10-20] Controlled-provider fixture stalled on coalesced JSONL requests

- **Context / Symptom:** The actual reviewed-edit smoke timed out before a provider turn started. A direct probe wrote initialize, initialized and account/read together but observed only the initialize response.
- **Root Cause:** `select` watched the OS pipe while buffered `stdin.readline()` had already consumed later lines into Python's private buffer. The fixture also used its launch directory rather than the separately requested thread directory for policy acknowledgement.
- **Solution:** Read available pipe bytes with `os.read` and drained complete newline-delimited packets from an explicit buffer before waiting again. The direct probe then received response IDs 1 and 2. Thread acknowledgement used the exact requested cwd and reasoning effort; production qualification and timeouts remained unchanged.
- **Prevention / Reference:** `scripts/controlled_peer_fixture.py` is a disposable protocol peer, not live-provider conformance or a replacement for the actual Eevee file-write path.

### [2026-10-20] Specialist desks covered the storage acknowledgement row

- **Context / Symptom:** The actual relay journey reached a supported hypothesis but timed out with `missing state: Storage: saved`; its specialist overlay visibly covered the entire storage/activity header row.
- **Root Cause:** The desk centered a nearly full-height panel against the entire terminal, starting at row 1. Header status was painted first and then cleared by the overlay, so local persistence acknowledgements and failures were not inspectable there.
- **Solution:** Reserved the two header rows and two global footer rows before sizing the desk. Compact confirmations retained their visible-summary/consent gates and scroll handling. Removed the obsolete fixed 16–20-row geometry assertion from the local-summary fixture rather than re-pinning it; semantic full-summary, no-scroll and reviewed-state checks remained.
- **Prevention / Reference:** `workflow_relay_smoke.py` waits for actual storage status while the desk is open at both sizes; do not replace this with a sleep or assume a hidden save succeeded.

### [2026-10-20] Rust panic thread IDs were parsed as part of a source path

- **Context / Symptom:** The actual Cargo/TUI location list displayed `5894500) panicked at src/lib.rs:3:5`. Pin range correctly refused it with `Source is absent from the current nonignored catalogue` instead of capturing `src/lib.rs`.
- **Root Cause:** Newer Rust panic output included a parenthesized thread ID before `panicked at`. The Node stack-frame heuristic treated any last opening parenthesis as an unclosed filename wrapper and included the already-closed thread annotation.
- **Solution:** Used the parenthesized-path branch only while the parenthesis remained unclosed before the coordinates; otherwise normal path tokenization applied. Added the exact observed thread-ID form and a Node path containing spaces to the parser regressions. All three isolated parser tests passed after the correction; actual integrated Pin range verification remained required.
- **Prevention / Reference:** `target/wave3-execution-relay-runtime/120/cargo-locations.txt` retained the failing-before surface. Source containment/exclusion checks were not weakened to accommodate a bad parse.

### [2026-10-20] Finding relays bypassed the source credential boundary

- **Context / Symptom:** Static review of the new finding relay found that its exact range-comparison cache copied the whole raw file into task metadata, including recognizable credentials outside a sanitized pinned range.
- **Root Cause:** Normal source capture applied secret masking, but the separate raw baseline reader and workflow validation did not inherit that boundary. Redacting only the persisted cache would also make later exact range comparisons incorrect.
- **Solution:** Refused credential-bearing raw baselines before staging or mutating the relay and enforced the same refusal in workflow storage validation. Added a regression covering actual Finding → Fix refusal, unchanged source/evidence/workflow state and no credential in the outgoing snapshot.
- **Prevention / Reference:** `relay_paths`, `workflows::State::validate` and the credential-bearing baseline regression. Use synthetic credential fixtures only; a source hash is not permission to persist all its raw bytes.

### [2026-10-20] Deferred decision drafts and edited criteria lost their original provenance

- **Context / Symptom:** Static review found that relaying decision A, choosing B and then fronting A's recipient could attach B's identity to A's text. Ordinary criterion renames/deletions could also leave stale origin rows and eventually exhaust origin capacity despite an empty checklist.
- **Root Cause:** Recipient hydration substituted the latest chosen decision instead of the original relayed snapshot; the normal Criteria editor updated only the checklist, not its provenance lifecycle.
- **Solution:** Persisted the original decision snapshot with the draft relay and refused stale proposal confirmation. Reconciled criterion origins atomically with normal editing: exact identities survived reordering, only an unambiguous single rename inherited its origin, deletions released capacity and ambiguous duplicates refused without mutation.
- **Prevention / Reference:** Regressions cover A→B recipient restoration, normal rename/full task restoration, unrelated reorder, duplicate refusal and 64 criteria → clear → new criterion.

### [2026-10-20] Finding range addressing misclassified appends and omitted untracked deletions

- **Context / Symptom:** Static review found that appending after a finding's final line marked it addressed, while deleting a previously relayed untracked file could leave all its findings unaddressed.
- **Root Cause:** The edit-alignment insertion transition reused the deletion-line intersection flag. Separately, removed untracked files vanished from the repository inventory instead of receiving the tracked-file `Missing` state; failed reads were treated uniformly as unavailable evidence.
- **Solution:** Computed insertion boundaries separately from deleted/replaced lines. Required a complete stable repository observation and confirmed contained no-follow disappearance before treating an omitted file as deleted; links, traversal, replaced roots and arbitrary read failures remained unknown. Neither path automatically resolved a finding.
- **Prevention / Reference:** Boundary/insertion/replacement/deletion fixtures plus no-follow disappearance and app-level deleted-untracked-source regressions.

### [2026-10-20] Ordinary repository refresh did not update stale-check admission

- **Context / Symptom:** Static review found that home `/refresh` could make specialist evidence stale while `/rerun` still compared against the completed run's old execution snapshot and refused the advertised action.
- **Root Cause:** `execution::State::repo_observed` returned before updating freshness unless an execution-specific before/after observation was pending.
- **Solution:** Updated the latest observed source fingerprint for ordinary observations too, while preserving generation gates that alone settle command preparation and final journal snapshots. Added a regression proving an ordinary refresh updates staleness without completing an unstarted execution capture.
- **Prevention / Reference:** The actual execution-relay journey uses home `/refresh`, not an execution-only refresh. Retained command arguments containing irreversible journal masks are separately refused rather than replayed as substituted `[REDACTED]` bytes.

### [2026-10-16] Carry-over evidence fixtures failed native canonical-root validation

- **Context / Symptom:** v0.1.10 qualification run `38866630609` failed tests on macOS and Windows. Native Windows diagnosis reproduced three carry-over assertions reporting one open hypothesis instead of zero for otherwise supported evidence.
- **Root Cause:** The new fixture stored the raw temporary directory as its source root. `repo::read_source` correctly requires the already-canonical identity used by real captures; macOS system aliases and Windows canonical/verbatim path prefixes made the raw fixture root invalid. Existing workflow/recommendation fixtures already canonicalized their roots.
- **Solution:** Canonicalized the carry-over fixture root after creating it, reusing the existing fixture convention. Did not weaken source identity checks, change expected hypothesis status, skip platform tests or reuse packages from the failed run. Qualification must restart for the corrected commit across Check and all five native rows.
- **Prevention / Reference:** Keep diagnostic build output on local Windows storage and clear inherited WSL `TERM`. Public Actions log download required authentication unavailable on this host, so diagnosis used the installed native Windows Rust 1.90.0 toolchain instead. Separate local parallel store timeouts were investigated with serial diagnostic execution; CI's normal parallel test command and assertions remained unchanged.

### [2026-10-16] Carry-over snapshots treated pending context as empty evidence

- **Context / Symptom:** A regression switched forms while the stored pin load was pending; the initial implementation immediately published a zero-pin record. Hiding that pending record with the next key also discarded the switch before its counts settled.
- **Root Cause:** Empty in-memory source/history collections were indistinguishable from completed empty loads at the snapshot boundary, and dismissal erased the pending transition instead of only its future visibility.
- **Solution:** Deferred snapshot settlement through pending source/run-history loads, retained the transition independently of visibility, and recorded late settlement without redisplaying it after input. Reopening then used the settled record. Behavioral regressions covered queued/in-flight run history, delayed pin acknowledgement, the first Unicode insertion/caret and later reopening.

### [2026-10-16] Explicit specialist refresh left carry-over hypothesis counts cached

- **Context / Symptom:** After a supporting source changed with otherwise identical stored metadata, deliberate specialist refresh showed unresolved proof while the next carry-over still counted zero open hypotheses. The failing regression observed 0 instead of 1.
- **Root Cause:** The carry-over metadata cache did not observe the deliberate source-revalidation boundary; unchanged serialized references did not prove the underlying bytes were unchanged.
- **Solution:** Invalidated carry-over hypothesis counts alongside recommendations on explicit workflow refresh. Source restoration was covered by the same regression. Recommendation and carry-over consumers shared one repository fingerprint, while ordinary form/draft/nickname changes remained cache hits.

### [2026-10-16] Compact picker hid the identity of an off-screen recommendation

- **Context / Symptom:** EV01B's actual 60×18 fixture offered `d Dismiss suggestion`, but Umbreon was outside the five visible rows while Jolteon remained selected, so the user could not identify the recommendation.
- **Root Cause:** The picker only marked visible form cells; its heading named the current form but not the off-screen recommendation. The home recommendation could also remain as an unintended background status.
- **Solution:** Included the recommended form in the compact heading without moving selection or portrait rectangles, and suppressed the background home recommendation under Forms. Both actual sizes passed dismissal/restart/new-run journeys; `target/ev01b-tui/60-dismiss/marked-with-current-selection.txt` shows current Jolteon and suggested Umbreon together.

### [2026-10-16] Equivalent review metadata kept a stale recommendation cache result

- **Context / Symptom:** The EV01A source-review fixture restored source bytes and explicitly recorded the same accepted-review metadata again, but the cached recommendation remained absent.
- **Root Cause:** Comparing only stored input values could not distinguish a deliberate evidence refresh with identical serialized values from an unchanged frame.
- **Solution:** Invalidated the suggestion verdict cache at the existing deliberate workflow-refresh boundary, keeping source revalidation outside rendering. Unchanged draft/animation frames still did not reread source files. Replaced copied capture/run cache contents with borrowed views and a bounded metadata digest.


### [2026-10-16] Empty-project startup skipped personal partner identity restoration

- **Context / Symptom:** A PK01 regression loaded a committed partner nickname with an empty task list, but the lead still had the default name. Task-backed restarts already restored the nickname.
- **Root Cause:** The Loaded handler refreshed the lead partner only inside its selected-task branch; the no-task path copied the roster without updating the lead.
- **Solution:** Restored the lead from the committed roster even without a selected task, preserving an unsaved live form when local edits existed. The runtime proof also renames before creating any task, exits, and restarts.


### [2026-10-16] Rejected nickname paste could turn Enter into an unintended clear

- **Context / Symptom:** The PK01 actual-TUI boundary case deleted the old candidate, pasted 13 CJK graphemes, and pressed Enter. Paste was visibly refused, but Enter saved the remaining empty editor and cleared the committed nickname.
- **Root Cause:** Insertion rejection retained the previous editor text but its error did not participate in save admission. Empty text was independently valid as the clear action.
- **Solution:** Retained an insertion-rejected flag until an explicit valid edit/correction. Enter after refused input queued no mutation; store-failure errors remained separately retryable. Added a regression starting from a committed nonempty nickname.
- **Prevention / Reference:** The actual terminal proof covers 13-grapheme refusal, the 12-CJK boundary, duplicate rejection and explicit clearing; display clipping never becomes a different saved value.


### [2026-10-16] Neutral intake-return review hid a retained provider draft

- **Context / Symptom:** The EV07 actual-TUI provider cancellation journey kept the stored follow-up draft but showed the empty composer placeholder after Stay or an unknown cancellation outcome.
- **Root Cause:** Generic slash dispatch retained and closed the composer before opening the local Return to Eevee confirmation. The review did not open an overlay whose closure would restore the editor.
- **Solution:** Handled the busy `/eevee` and `/normal` review before generic editor retention, so neutral Stay and cancelled/unknown outcomes kept the original input, caret and editing state. Regression coverage checks both known and unknown cancellation outcomes.
- **Prevention / Reference:** The native HTTP fixture correctly reported a locally closed reader with remote outcome unknown; intake return remained blocked rather than relabeling it safe cancellation. The owned-command fixture separately verified actual process-tree exit before returning.


### [2026-10-16] Partner attribution displaced compact run evidence and smoke routes

- **Context / Symptom:** The resumed EV03B recovery evidence showed `Failed` in the 60×18 history but lost the `Failed command` freshness line. Local workflow acceptance also waited for the obsolete `Flareon / Codex implementation` title while the actual native route displayed `Flareon / Codex · native (no tools) implementation`.
- **Root Cause:** Partner attribution lengthened the first history line beyond the fixed three-row slot. Smoke drivers also assumed any form could prepare commands or source edits, and pinned older capture/provider display copy.
- **Solution:** Kept state/task/end time on the first history row and moved partner attribution beside the run id on the second. Migrated command-preparation fixtures to Jolteon and implementation fixtures to Flareon. Used the current capture review boundary and provider-independent Flareon title prefix while retaining stored-evidence, no-send and unchanged-source assertions.
- **Prevention / Reference:** `scripts/execution_smoke.py`, `scripts/execution_recovery_smoke.py` and `scripts/workflow_smoke.py`; owning-form switches never replay the blocked action.

### [2026-10-16] Rust language server exited during resumed EV03B inspection

- **Context / Symptom:** The configured rust-analyzer failed the command-gate reference request with `LSP server exited unexpectedly (code 0)`.
- **Root Cause:** The configured server was not usable in this session; no symbol response was available.
- **Solution:** Used scoped source/caller inspection and Rust 1.90.0 compiler, test and Clippy verification without changing global tooling. No exported symbol was changed.


### [2026-10-16] Update progress differed from installation before download

- **Context / Symptom:** Self-update performed its initial release lookup before the installer's visible phases and did not preserve the user's UTF-8 locale when handing over to it.
- **Root Cause:** Release discovery was owned by the updater while animation was owned by the embedded installer; the sanitized helper environment omitted `LANG`, `LC_ALL` and `LC_CTYPE`.
- **Solution:** Reused the embedded installer's animated release lookup, retained locale variables alongside existing terminal settings, and kept read-only checks separate. Windows lookup used a private result file so both console handles remained attached; redirected/CI/dumb-terminal output stayed static.
- **Prevention / Reference:** The actual POSIX install/update lifecycle passed lookup motion, resize/restoration, monochrome shape parity, checksum refusal and read-only checks; four visible drawing frames were shared between install and update. Native Windows syntax/type checks were separate from terminal-animation qualification.

### [2026-10-16] Compact headers hid source failures and model selection

- **Context / Symptom:** At 60×18, purged-context dispatch displayed the detailed refusal but hid its status line. A long project/partner prefix also displaced the selected model and reasoning level from the home header.
- **Root Cause:** The source ownership sentence wrapped into the row reserved for status, while the home header concatenated context before selection without reserving selection width.
- **Solution:** Used separate fixed source ownership/status rows and a bounded compact home header with explicit model/effort space. Complete source failures remained scrollable; long contextual labels retained visible truncation.
- **Prevention / Reference:** The real source-context matrix exercised compact purge, missing/stale source and storage failures. A viewport regression retained model/effort with a long project name; targeted actual-TUI captures checked the selected values beside the draft.

### [2026-10-16] Directory separators escaped the attachment query

- **Context / Symptom:** Typing a scoped `@` query opened Commands at its first slash instead of filtering files, despite the query working in isolated matching tests.
- **Root Cause:** Global slash dispatch ran before the composer submenu and did not classify the active attachment query as literal input. Repeated catalogue requests while typing also temporarily blocked selection of otherwise visible rows.
- **Solution:** Gave the active file query character ownership, fetched one eligible catalogue per opening, and filtered its original indices locally. Case-insensitive scoped abbreviations and spaces never broadened source eligibility or bypassed capture review.
- **Prevention / Reference:** The regression drove real slash/space key events with a retained suffix/caret. Actual terminal journeys inspected the intended scoped source and excluded the same basename in a different directory.

### [2026-10-16] Local selection reviews kept an extra drawer or lost their visible owner

- **Context / Symptom:** Keeping the current provider after its review returned to Connections instead of the invoking editor. A direct unlisted-model review could also disappear during ordinary planning settlement while its pending selection remained.
- **Root Cause:** The connection launch frame was removed only after synchronous selection. The new direct-model review had no picker, but busy classification and automatic drawer settlement still relied on picker presence.
- **Solution:** Removed the launch-only connection frame before waiting for local review completion, counted pending model changes as busy, and excluded them from automatic drawer closure. Neutral cancellation preserved the original provider/session and exact draft.
- **Prevention / Reference:** App-level regressions exercised settlement and cancellation, not only the isolated picker. The direct-model regression failed before the ownership fix; actual wide/compact journeys exercised the provider review and retained editor.

### [2026-10-16] Cancelling metadata connection review left an invisible picker owning input

- **Context / Symptom:** The real-TUI project-executable trust fixture correctly stopped before execution, but Escape returned home and `/quit` was subsequently typed as literal draft text. The interface reported an unfinished request although no modal was visible.
- **Root Cause:** Metadata cancellation completed after the review overlay closed. Its normal completion path reopened the model picker inside planning state without a visible owner, leaving global commands blocked.
- **Solution:** Explicit catalogue dismissal now survives asynchronous cancellation and prevents both completion/error paths from reopening a picker. A new inspection resets this dismissal; blocked automatic inspections still retain their error/retry view.
- **Prevention / Reference:** Regression coverage checks cancellation reaches idle without a hidden picker or queued run. The actual terminal fixture also requires normal command-based exit and verifies the project executable remains untouched.

### [2026-10-16] First direct request cancelled during pending local context restoration

- **Context / Symptom:** The initial Enter-only native request returned to its draft before any network output with a misleading request-changed notice.
- **Root Cause:** The first durable task save could trigger context restoration before its prepared preview arrived. Temporary local readiness loss was incorrectly treated as a changed request.
- **Solution:** Retained the immutable prepared owner while the matching task/revision waited for local readiness, then dispatched exactly once. Real revision changes and invalid terminal sizes still cancel; failure notices preserve the actual reason.
- **Prevention / Reference:** The regression exercises pending save/readiness before dispatch. The actual first-send fixture completed one response with zero model-discovery requests and committed its first-used default route before exit.

### [2026-10-16] Compact Codex method selection browsed native settings without switching the task

- **Context / Symptom:** The 60×18 real-TUI journey chose native responses, but the task header remained CLI and the next action opened native setup instead of returning home. The wide layout's persistent provider column had hidden that incomplete transition.
- **Root Cause:** Removing a method shortcut from visible command choices also removed it from the shared availability validator, so the actual provider-switch command was refused after the drawer changed its browsed row. The intermediate provider frame also remained in overlay ancestry.
- **Solution:** Kept explicit native/CLI shortcuts as connection-method actions under the one Codex provider, preserving their capability validation. A completed local method choice removes the intermediate provider frame and returns to its invoking editor; sessions still use the real handoff boundary.
- **Prevention / Reference:** Assert actual task route and retained editor at both supported widths, not merely presence of a Codex label. The runtime journey checks that method selection closes the extra drawer without launching authentication.

### [2026-10-16] Starting in the user home blocked native provider configuration

- **Context / Symptom:** The actual release TUI launched with its working directory equal to HOME refused a private user-owned provider config with `Cannot read user provider config safely`; no metadata HTTP request was sent.
- **Root Cause:** Every current working directory was treated as an untrusted project boundary, so launching from the home directory incorrectly classified its own user-scoped provider stores as project files.
- **Solution:** Canonically verified home-directory startup is no longer a project-file boundary. Other workspaces, including projects beneath home, retain the project-source restriction. Ownership, permissions/DACL, no-follow identity and endpoint restrictions remain unchanged.
- **Prevention / Reference:** The focused real-TUI journey records failing-before and successful-after discovery without inference or config changes. Regression coverage also refuses the same files without a verified home and refuses project-local configuration in home descendants.

### [2026-10-16] SDK and ACP still rejected models omitted by discovery

- **Context / Symptom:** Exact custom IDs were locally refused even when the selected integration exposed a real model setter. The SDK also blocked every explicit effort because it lacked an applied-effort acknowledgment, although official request Options supported the value.
- **Root Cause:** Catalogue membership had been coupled to request eligibility in adapter-specific preflight and setter acknowledgment checks. Informational effort capability was confused with an assertion of observed application.
- **Solution:** Removed membership gates, retained official selector/control availability and exact correlated model acknowledgments, and kept SDK authentication/tool policy independent. SDK discovery failures remained inspection errors but no longer blocked acknowledged explicit models. Exposed advertised SDK effort request values with applied effort explicitly unknown. Required bridge revision 3 so old installed bridge code could not silently retain the rejected behavior.
- **Prevention / Reference:** Exact-ID and stale-option acknowledgment regressions cover SDK/ACP state transitions. Provider account qualification remains separate from parser/policy fixtures; no unconfigured live account is claimed.

### [2026-10-16] Native platform qualification exposed HTTP and ACL fixture assumptions

- **Context / Symptom:** Native macOS CI failed the loopback SSE reader fixture with `WouldBlock` / `Resource temporarily unavailable`; native Windows tests failed before production validation at fixture `SetSecurityInfo`, returning `ERROR_ACCESS_DENIED` (5). Linux rows and the local serialized Rust suite had passed.
- **Root Cause:** Darwin accepted sockets inherited the listener's nonblocking mode, while the fixture assumed bounded blocking reads. Windows fixtures rewrote inherited ownership/DACL state after file creation; the reported API transition failed on the runner account. The exact internal Windows denial cause was not established, so it was not attributed to guessed handle rights.
- **Solution:** Explicitly selected blocking mode on accepted fixture sockets while retaining read/write deadlines. Created Windows fixtures atomically with current-user owner and protected DACL in `SECURITY_ATTRIBUTES`; NULL-DACL cases use the same creation path. Production owner/DACL/reparse checks were not weakened, and no failing platform test was skipped.
- **Prevention / Reference:** Run the ordinary parallel all-target suite, not only `--test-threads=1`; local parallel acceptance passed 563 tests with 2 ignored after the socket fix, and native Windows owner/DACL fixtures passed qualification run `38762603406`. Each final source candidate still requires its own five-platform qualification. Runtime cancellation checks use actual client socket closure, no-resend and preserved draft/response ownership rather than incidental transient notice wording.

### [2026-10-16] Native response was rejected despite valid streamed provider output

- **Context / Symptom:** Native ChatGPT model discovery returned seven visible text models, including GPT-6.1/GPT-6, with an empty executable PATH. An explicitly reviewed GPT-6.1 request then ended with `Provider did not return an SSE stream; remote outcome unknown`.
- **Root Cause:** The successful HTTP response omitted `Content-Type` even though its body contained semantic SSE frames. A MIME-header prerequisite rejected valid provider output before the bounded protocol parser could inspect it. The request had already been sent, so the unknown-outcome label was retained rather than claiming a harmless preflight failure.
- **Solution:** Removed the MIME-header admission gate and used actual bounded SSE framing plus authoritative protocol completion to validate the response. JSON/HTML, truncated output and absent terminal events still cannot become successful responses. Removed the throwaway response-shape diagnostic without logging response values or credentials.
- **Prevention / Reference:** The real-provider journey is recorded under `target/native-017-live`; the extended workbench fixture deliberately omits the SSE MIME header for actual approved HTTP requests. Its second-turn ownership, final refusal text, refresh failure and cancellation checks exercise the full native path rather than merely echoing parser inputs.

### [2026-10-16] Native connection review found credential-mode and response ownership defects

- **Context / Symptom:** Static review found that explicit keyless mode could still attach an inherited API key, Windows file sources lacked the Unix-equivalent owner/permission boundary, and repeated native responses reused one consumer item identity. Responses refusal events were also ignored.
- **Root Cause:** Credential resolution consulted the environment before honoring keyless mode; file guards were Unix-only; native item IDs were constant across generations; text extraction handled only ordinary output text.
- **Solution:** Resolved keyless mode before any key lookup, added handle-bound Windows owner/DACL/reparse/resolved-path checks, qualified native item identity by request generation, and reconciled final text/refusals without duplicate output. Native consent now shows a typed immutable HTTP destination rather than subprocess-only effects.
- **Prevention / Reference:** Added key-audience/keyless, platform trust, cross-request identity, refusal/final-tail and actual two-request workbench regressions. Existing exact payload/configuration revalidation, no-redirect policy and authoritative cancellation/completion boundaries remain enforced.

### [2026-10-16] Native follow-up was incorrectly routed to unavailable session resume

- **Context / Symptom:** The actual TUI native-service fixture completed its first request, but Enter on the saved second request showed `Direct API requests are fresh and stateless here` instead of a Run review.
- **Root Cause:** Observed-model metadata reused `Event::Thread` with an empty thread ID. The consumer retained `Some("")`, so the composer attempted session resume. After separating that event, identical empty turn IDs still made the consumer associate the second response with the first request.
- **Solution:** Introduced a separate correlated `ModelObserved` event and unique `native-request-{generation}` local turn identities. Native streams report their actual model without manufacturing a provider session; fresh follow-ups and response ownership remain independent.
- **Prevention / Reference:** The model-acknowledgment regression asserts no resumable thread and a fresh follow-up. Producer tests assert distinct thread/turn correlation pairs; the workbench approves two actual HTTP/SSE requests and inspects each request owner before testing metadata failure and client cancellation.

### [2026-10-15] macOS self-update smoke lost its PTY before restoration inspection

- **Context / Symptom:** Published v0.1.6 installation acceptance passed the first 15 checks, then `live-owned-binary-self-update` failed at `termios.tcgetattr(self.slave)` with `(25, 'Inappropriate ioctl for device')`.
- **Root Cause:** The harness launched the update executable as a session leader. Unix self-update intentionally execs `/bin/sh`; that session-leading shell could acquire and revoke the macOS PTY on exit. The existing bootstrap-only wrapper prevented this for direct installation but not the same shell reached through update.
- **Solution:** Moved the existing POSIX parent-process wrapper into shared `animated_install`, covering bootstrap and self-update without changing product binaries or weakening termios/animation assertions. The actual installer remains a child, like a command launched from a normal user shell.
- **Prevention / Reference:** Re-ran complete acceptance against the downloaded v0.1.6 package: all 23 lifecycle checks and 34 installed terminal scenarios passed (`target/release-016-installed-verified`). Python release-script tests ran 17 with 3 platform skips. Keep both shell-entry paths under the same ownership guard.

### [2026-10-15] Codex showed only GPT-5.6 instead of GPT-6 and GPT-6.1

- **Context / Symptom:** `/model` showed only `gpt-5.6-sol`, `gpt-5.6-terra` and `gpt-5.6-luna`. Native inspection returned those exact three entries with no next page, so this was not an Eevee pagination or generation-name filter.
- **Root Cause:** PATH selected Homebrew Codex 0.147.0. With the same ChatGPT account and model configuration, isolated official 0.162.1 returned seven entries including `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-sol` and `gpt-6-luna`. Native lists can be bundled/cache-backed; they are not a guarantee of live global model availability. Separately, Eevee had treated the catalogue's default as the provider configuration default, ignoring the configured `gpt-5.6-luna`/`high` selection.
- **Solution:** Exposed actual executable/version and catalogue provenance, model count and compact scroll controls; added explicit same-installation update/refresh guidance. Read only the effective nonsecret model/provider/effort through official `config/read` and preserved that configured default through pre-dispatch catalogue revalidation. Kept complete pagination and exact advertised-model/effort validation; did not hardcode newer names, inject arbitrary models, silently substitute defaults or upgrade the host CLI.
- **Prevention / Reference:** Update the installation shown in the picker (`brew upgrade --cask codex` for this Homebrew path), then `/models-refresh`; changing Eevee's version alone cannot update its external CLI. The isolated [official 0.162.1 archive](https://github.com/openai/codex/releases/tag/rust-v0.162.1) matched SHA-256 `b293a0fcdd871900a44c0a4f40a1aa401b31e02854f790ebdb8bd5d1177fd1fe`. Native release-candidate journeys observed seven models at 120×36 and 60×18, an explicit GPT-6.1-Sol response, and a separate Default response correctly retaining GPT-5.6-Luna (`target/release-016-live/results.json`). The user did not authorize a global Codex upgrade.

### [2026-10-15] Fresh-home model loading stalled or completed into a hidden picker

- **Context / Symptom:** Published 0.1.5 stayed on `Loading models…` after `/model` on a fresh home. An integrated real-TUI smoke then found that the request-first settlement path closed the loading overlay, leaving a completed picker hidden behind the home screen and blocking another model action.
- **Root Cause:** The automatic catalogue driver and preparation path required a saved task even though metadata inspection carries no task content. The generic response-to-home transition treated catalogue loading like a model response. ACP also validated committed task/review input before its metadata-only branch; Codex required `inputModalities` even though the installed 0.147.0 schema declares it optional with a text/image default.
- **Solution:** Used an empty project-scoped snapshot before task creation, stripped task/context/write/handoff data from metadata inspection, kept metadata dialogs visible through completion, and retained provider-specific errors with retry actions. Moved ACP metadata admission before task-content checks while retaining provider/project/selection checks. Accepted only an absent Codex optional field using its documented default; explicit invalid modalities still fail. Preserved separate consent for manual refresh and all content requests.
- **Prevention / Reference:** Fresh-home and save-during-load regressions cover ownership and draft restoration. Actual Codex 0.147.0 catalogues loaded at 120×36 and 60×18; missing-provider/retry and fresh reviewed-refresh paths ran in the native macOS PTY (`target/home-model-after`). These metadata inspections sent no task prompt.

### [2026-10-15] Response progress appeared twice and the home partner disappeared

- **Context / Symptom:** The user observed duplicate loading in the response UI and no Eevee card on the main screen.
- **Root Cause:** Both the shell header and response metadata rendered `activity_label()` with a spinner, while the response also displayed a progress notice. The request-first shell gated the partner rail on companion focus rather than screen capacity.
- **Solution:** Made the header the sole live-progress owner, kept response metadata about its selected output only, and reserved footer rows only for available controls/notices. Earlier partial output no longer inherits the current stream or observed model. Restored the partner card at ≥90 columns, using the same native-image placement geometry; compact/linear layouts retain the drawer.
- **Prevention / Reference:** Two actual Codex turns at 120×36 and 60×18 exercised connecting, waiting, streaming, completion, previous-request attribution, paused follow and return-to-latest. Captured 110 wide and 123 compact sampled frames without duplicate stage labels. Cell-rendered cached portraits were exercised; this is not a new Kitty/Sixel GUI compatibility claim.

### [2026-10-15] Host Clippy version differed from the repository qualification toolchain

- **Context / Symptom:** Host Rust 1.97.1 passed 497 tests, but strict Clippy rejected an existing `src/ui/handoff.rs` optional-candidate access with `clippy::unnecessary_unwrap`.
- **Root Cause:** The host default toolchain was newer than the repository's Rust 1.90.0 qualification toolchain and enforced a newer lint on unchanged code.
- **Solution:** Ran final formatting, all-target tests, strict Clippy and the runtime build with the already-installed `+1.90.0` toolchain. No lint was suppressed and no unrelated handoff code was changed.
- **Prevention / Reference:** Use `cargo +1.90.0 clippy --locked --all-targets -- -D warnings` to reproduce the supported qualification check. Host 1.97.1 strict lint remains a known unmodified-code limitation. Language-server startup also exited on this host; scoped source inspection was used instead.

### [2026-10-15] Workbench smoke assumed unwrapped setup text and full task text on home

- **Context / Symptom:** The 60×18 journey failed waiting for contiguous `npm install`, although setup visibly wrapped those words onto adjacent rows. The locked-store journey also expected an appended multi-line Goal tail on the summarized home card.
- **Root Cause:** The assertions depended on incidental text wrapping and the older full-task home presentation rather than the actual setup and retained editor state.
- **Solution:** Removed the installation-wording assertion and reopened `/goal` before checking the retained failed-save edit. The smoke still checks missing-provider state, absence of an authentication review, safe navigation back, failed-save retention/retry and byte-preserved persisted drafts.

### [2026-10-15] Microsoft Store Python escaped the native smoke's tracked process tree

- **Context / Symptom:** Windows command cancellation saved `OutcomeUnknown / OwnershipUnknown`: the launched root had exited and Job accounting was empty, but both output pipes remained open. A direct process-handle check found the fixture root stopped while its child and grandchild remained alive.
- **Root Cause:** `uv run --python 3.13` selected Microsoft Store/App Execution Alias Python 3.13.14. Its fixture descendants were not contained in the tracked Job on this host; the exact package/broker mechanism was not traced. Ordinary native Rust children and a separate unpackaged Python tree did not reproduce it. This was not resolved by closing observer handles or changing console-detachment flags.
- **Solution:** Installed unpackaged Python 3.13.14 into an isolated temporary directory with `uv python install 3.13 --install-dir <directory> --no-bin --no-registry`, then set `UV_PYTHON_INSTALL_DIR` to that directory and `UV_MANAGED_PYTHON=1` for native verification. The unchanged cancellation implementation then reported `Cancelled / OwnedTreeExited`, no truncation/error, and all three fixture processes stopped. No global Python, PATH or registry changes were made.
- **Prevention / Reference:** Keep the unknown-outcome guard and original cancellation deadlines; do not count terminated launcher processes as proof that descendants stopped. Use unpackaged Python for ConPTY/owned-tree qualification. Native fixtures record interpreter provenance, and execution timeout diagnostics now include persisted state/tree/cancellation/reservation values. All temporary runner traces and console-flag experiments were removed.

### [2026-10-15] Deferred clean exit blocked waiting for an unnecessary key

- **Context / Symptom:** The actual command lifecycle cancelled its owned process tree and committed the final journal, but cancel-and-exit then remained on idle output instead of exiting or offering an exit review.
- **Root Cause:** Diagnostic tracing confirmed `Navigate(Quit)` reached the app with equal saved/current revisions and no dirty active draft. This clean exit was set near the start of the main loop, which entered blocking `input.read()` before its bottom-of-loop exit check. Request-first focus exposed the clean path that previously often required another save-confirmation key.
- **Solution:** Honored the settled quit immediately after deferred execution actions, before rendering or input waiting. Kept the existing bottom check for input/worker-driven outcomes. Removed the temporary diagnostic prints; no extra polling timer or forced process termination was added.
- **Prevention / Reference:** The existing actual execution smoke's cancel-and-wait/clean-exit/draft-restart scenario reproduced the failure without extra input; it remains the regression acceptance path on Linux and native Windows.

### [2026-10-15] Context smoke still expected the old repeated-root packet

- **Context / Symptom:** The source smoke expected 326 outgoing bytes while the actual reviewed envelope contained 267. Its expectation serialized the full stored capture DTO instead of the already-shipped compact outgoing packet.
- **Root Cause:** The fixture predated root hoisting, hexadecimal source identities and removal of storage-only fields from outbound captures. Execution fixtures also expected duplicate Back-history entries, and treated absent task cards as proof of cancel-and-switch acknowledgment even though the cancellation guard had already hidden those cards.
- **Solution:** Checked the current reviewed envelope against independent committed capture facts and its exact UTF-8 count, comparing parsed JSON rather than incidental key order. Removed the obsolete navigation-cycle expectation and waited for the selected task's actual home content before sending further input. Privacy masking, source identities, stale/missing-source refusals and execution-denial assertions remain.
- **Prevention / Reference:** All 10 actual source/context journeys passed (`target/015-context-final`); keep storage DTO and outgoing review contracts distinct.

### [2026-10-15] Request-first startup displaced a restored specialist overlay

- **Context / Symptom:** The actual specialist workflow resumed a saved task into its Goal editor instead of the saved Memory overlay. Closing Help could also resurrect a previously completed Goal edit instead of returning to navigation.
- **Root Cause:** Request-first editor initialization ran after restoring overlay navigation and overwrote it; a later explicit editor change retained the old composer-return owner.
- **Solution:** Restored overlays take precedence over the underlying editor, with its return owner preserved separately. Opening a different editor replaces that owner, and closing an overlay preserves its actual invoking focus.
- **Prevention / Reference:** A restart/focus regression and all 15 actual Linux specialist workflow cases passed (`target/015-workflows-complete`).

### [2026-10-15] Parallel capacity verification exceeded its worker-event deadline

- **Context / Symptom:** One integrated Rust run passed 480 tests but the capacity test timed out after save 239 while waiting five seconds for a Store worker event. Native verification was also running; this overlap alone does not establish the cause.
- **Root Cause:** Not established. Subsequent host sampling showed no sustained CPU or I/O saturation; no product storage failure was reproduced independently.
- **Solution:** Isolated final verification with `cargo test --locked --all-targets -- --test-threads=1`; all 481 tests passed with 2 ignored. No timeout, capacity, durability assertion or Store implementation was weakened.
- **Prevention / Reference:** Retain the original deadline and investigate with worker/thread diagnostics if the timeout recurs; a passing serial run is not a claimed concurrency fix.

### [2026-10-15] Clearing a submitted request raced committed-input admission

- **Context / Symptom:** A real first-screen request reached its review, but `y` immediately produced `Task changed since preview; save and review a new request` instead of a response.
- **Root Cause:** Returning home cleared and saved the Request draft before the worker's read-only committed-snapshot check. Providers also repeat that check during dispatch and permission handling, so merely delaying until thread creation was insufficient. Retained previous answers were additionally labelled with the newly submitted request.
- **Solution:** Held task saves through the content request's terminal acknowledgment while allowing visibly pending follow-up editing. Ctrl+S cannot bypass this ownership hold; cancellation waits before flushing. Pre-output failure restores the original input when no follow-up exists. Shared bounded request ownership now follows each retained response item.
- **Prevention / Reference:** The regression exercises real Store admission across Run, editing, Ctrl+S, thread establishment and cancellation. Actual latest Codex 0.162.1 first/follow-up requests passed at 120×36 and 60×18 with inline streaming and previous-response attribution (`target/015-request-first-final`).

### [2026-10-15] A source-purge shortcut could approve a hidden resized dialog

- **Context / Symptom:** Review found that `y` in a Sources removal confirmation lacked the visible-summary guard used by other confirmations; below-minimum rendering hid that dialog.
- **Root Cause:** The inner source handler had no terminal-size or rendered-scope state. Adding a direct shortcut exposed the existing cross-layer omission.
- **Solution:** Added current-target summary visibility to consent, invalidated it on resize and hidden renders, and rejected affirmative handling below minimum size. Source effects stay concise and fixed above optional exact capture details.
- **Prevention / Reference:** An actual-render regression covers unrendered consent, shrink/grow without repaint, long Unicode paths, paste/repeat/modified input and the final intended purge identity.

### [2026-10-15] Native smoke helpers could not lock an incremental target over WSL UNC

- **Context / Symptom:** The published Windows install journey passed live self-update, then its lifecycle-probe build failed with `incremental compilation: could not create session directory lock file` and OS error `-2147024895`.
- **Root Cause:** The helper hard-coded the repository's `target`, placing native Cargo incremental locks on a WSL UNC filesystem instead of a local Windows volume.
- **Solution:** Both lifecycle and older-release fixture builders now honor `CARGO_TARGET_DIR`. Native verification sets it to a local-C temporary directory; the pinned Rust 1.90.0 suite and release build complete there.
- **Prevention / Reference:** Keep native binary/build output local to Windows when the source checkout is accessed through `\\\\wsl.localhost\\…`.

### [2026-10-15] Installed-update smoke terminated the native cleanup helper too early

- **Context / Symptom:** Native self-update succeeded but the fixture observed a retained `.ev-stage*` directory after closing its ConPTY session.
- **Root Cause:** The updater's cleanup helper belonged to the harness-owned Windows Job. Closing that Job immediately after the old process exited killed the helper before it could remove the retired directory.
- **Solution:** Waited up to ten seconds for retired-stage cleanup before closing the session/Job. The original final cleanup assertion remains; the product updater was not changed.

### [2026-10-15] Latest OpenCode required its native-package postinstall

- **Context / Symptom:** An isolated latest-provider verification install used `--ignore-scripts`; OpenCode then reported `opencode-ai's postinstall script was not run`.
- **Root Cause:** OpenCode's official package needs its postinstall to select/link the installed native optional dependency.
- **Solution:** Inspected and explicitly ran that official postinstall inside the disposable verification prefix, without changing global providers. OpenCode 1.19.0 and Gemini 0.64.0 subsequently passed actual ACP1 initialization with no model request.
- **Prevention / Reference:** Normal documented OpenCode npm installation runs its required setup. Claude's separate latest-SDK setup intentionally retains `--ignore-scripts` and was verified independently.

### [2026-10-15] Published-release installer smoke duplicated checksum entries

- **Context / Symptom:** The published 0.1.4 archive passed fresh install and ten preservation/security cases, then the local HTTP bootstrap fixture failed with `malformed SHA256SUMS or missing unique checksum for ev-linux-x86_64.tar.gz`.
- **Root Cause:** `ReleaseFixture` always appended `install.sh` and `install.ps1` checksums. Native build-artifact manifests omit those entries, but the downloaded public release manifest already contained them; the fixture introduced duplicates that the real installer correctly refused.
- **Solution:** Preserved the supplied manifest and appended only missing installer entries. No installer validation, checksum assertion or release package changed.
- **Prevention / Reference:** Run installation lifecycle verification against downloaded public-release packages as well as pre-publication build artifacts; their checksum-manifest scopes differ.

### [2026-10-15] ConPTY full clears could escape effective visual synchronization

- **Context / Symptom:** The native save journey emitted about fifty full-screen clears. A minimal native probe showed `Begin → Clear → End → text` for a batched write: this ConPTY forwards DEC 2026 markers before publishing its rendered cells, so markers alone cannot prevent a blank flash.
- **Root Cause:** The main loop forced a full Windows clear on every dirty frame instead of using the terminal cell diff. The previous native smoke also relied on optional CSI 8 resize hints and on retaining the text inside an injected OSC sequence; ConPTY omitted the growth hint and filtered that OSC before the application received it.
- **Solution:** Removed ordinary Windows full clears while retaining startup, resize and native-image invalidation. Drained the old frame and updated the smoke terminal's requested dimensions directly, while still honoring explicit hints. Added a plain-text draft tail to the existing hostile-control paste fixture, retaining its no-execution and no-OSC-escape checks rather than asserting a platform-specific control-filter residue.
- **Prevention / Reference:** Native ConPTY editing at 120×36 and 60×18 produced zero full clears after startup, preserved exact draft/caret storage and restored console state. Six dark/light/mono-ASCII native journeys passed popup return, CJK/combining input, shrink/grow, nine forms, idle silence and exit (`target/native-014-{persistence,ui}`). Do not restore blanket per-input clears or assume every ConPTY version implements synchronized publication.

### [2026-10-15] Windows pipe fixture stalled before its test body

- **Context / Symptom:** Native Windows tests launched from WSL timed out in the piped stdin round-trip with an alive child, no output and no entry into the fixture. The failure reproduced with both Rust 1.92 and the release-pinned 1.90.0, including while the parent kept stdin open.
- **Root Cause:** A CDB child-stack capture located the wait in `NtCreateFile` → Rust libtest's `terminfo::get_dbpath_for_term` / `TermInfo::from_env`, before the fixture ran. The interop environment supplied `TERM=xterm-color`. Libtest initializes its terminal even with `--color=never`; this was not evidence of a pipe EOF or backpressure defect.
- **Solution:** Cleared `TERM` only in the native PowerShell verification process (`$env:TERM=$null`). Removed all diagnostic waits, markers and test-argument changes, retaining the original five-second deadline and round-trip/backpressure assertions. The pinned native suite passed 428 tests with 2 ignored and the optimized `ev 0.1.4` build completed.
- **Prevention / Reference:** Keep native verification environments separate from WSL terminal metadata. Do not increase transport timeouts or weaken process assertions to compensate for a test harness stalled before its fixture.

### [2026-10-15] Performance harness could not measure the current composer and child scope

- **Context / Symptom:** Baseline runs failed first on the removed `Ctrl+K Actions` footer, then with `POSIX raw input mode not observed`, and finally with `approved workload resource scope was not observed; refusing a core-only measurement`. The final terminal matrix also reported idle output while its before-capture visibly said `Storage: autosave pending`.
- **Root Cause:** Readiness was pinned to obsolete wording. The low-latency reader bypassed the normal raw-mode snapshot, so a clean, immediate quit could end before it was recorded. A completed child lost its command line before reaping and was reclassified from `local_commands` to `app_helpers`, despite a nonzero recorded local-command RSS peak. The idle check started before deferred navigation persistence completed.
- **Solution:** Removed the wording dependency; sampled terminal modes without adding a timed wait after the measured readiness instant; retained previously observed scope for the same PID/creation-time identity; awaited committed storage before asserting zero idle output. Kept resource-scope, zero-output and restoration assertions intact.
- **Prevention / Reference:** `target/performance-{baseline,after}/results.json` both completed with `measured_local_targets_met`, including the real approved stream, 10 MiB output and 10,000-line diff. Scope identity must not regress when an exited process stops exposing argv; pending storage is work, not idle.

### [2026-10-15] UX smoke dependencies were unavailable in the system Python

- **Context / Symptom:** The real-PTY UX capture failed with `ModuleNotFoundError: No module named 'pyte'`; installing into the system interpreter was refused with `externally-managed-environment`.
- **Root Cause:** This host uses a distribution-managed Python environment; the smoke harness requires its own pinned `pyte` and `wcwidth` dependencies.
- **Solution:** Created `target/ux-venv` with `uv venv` and installed `pyte==0.8.2 wcwidth==0.2.13` using `uv pip install --python target/ux-venv/bin/python`. Ran terminal verification with that interpreter without changing system packages.
- **Prevention / Reference:** Use an isolated smoke environment; do not override the distribution's externally-managed protection. Rust LSP reference lookup also exited before serving requests on this host; on 2026-10-16, `rust-analyzer --version` identified the resolving rustup shim's missing component (`Unknown binary 'rust-analyzer'`). Scoped source inspection and compiler/runtime checks were used without changing global tools.

### [2026-10-15] Reserved partner drafts could not preserve valid escaped text

- **Context / Symptom:** PT05B's existing maximum-text round-trip failed with `Stored JSON exceeds its bounded size`.
- **Root Cause:** Schema 7 reserved only 64 KiB for all partner drafts, but four valid 4096-byte drafts can JSON-escape to nearly 100 KiB before adding other partners. Reusing that bound would reject previously valid data.
- **Solution:** Schema 8 enlarged the same reserved column through a transactional task-table rebuild, retaining all fields and deleted AUTOINCREMENT identities. The bounded capacity accommodates six complete draft sets without custom encoding or a reduced text limit.
- **Prevention / Reference:** Keep the existing maximum-escaped-text test and migration/old-binary-refusal coverage when changing persisted aggregate bounds.


### [2026-10-15] schema_upgrade_smoke couldn't drive the slash palette

- **Context / Symptom:** `scripts/schema_upgrade_smoke.py` was written against a pre-slash palette and kept failing on the real v0.1.3 binary: `previous executable has no visible action: Acceptance criteria`.
- **Root cause:** Three drifted assumptions: chooser rows render as `> /criteria Acceptance criteria` (slash-prefixed, `║` border); `criteria-edit` is a contextual choice that only exists inside the checklist overlay; and Ctrl+K+Home sent in one write raced the chooser opening, so the retained cursor hid the target row.
- **Solution:** `previous_action` now opens/resets the chooser with Ctrl+K, types the command's slash name into the filter, and requires the selected row to contain `> /<name>` before Enter.
- **Prevention / Reference:** When driving the palette from scripts, filter by typed command name instead of arrowing to labels, and pump between chooser open and filter typing.
### [2026-10-15] Source-packet dedup produced dangling citations

- **Context / Symptom:** A new `same_bytes_as` dedup for overlapping basket captures passed compile and a first-run assembly, but the focused test failed: a covered capture cited another *covered* capture (`same_bytes_as` pointed at an entry with no `text` field), so the referenced bytes were absent from the packet.
- **Root cause:** The first pass assigned each covered capture to the first enclosing capture *seen in input order*, not to a capture guaranteed to carry text. A narrow range pinned before a wider one could become the "representative" of a third range while itself being covered — a citation chain ending in a stub.
- **Solution:** Compute the maximal set first (captures not covered by any other), then assign every covered capture to the earliest *maximal* cover. The merge key stays `(root, path, source_sha256)` plus a byte-containment check, so identical text from different files or revisions never merges and chains are structurally impossible.
- **Prevention / Reference:** `context::tests::overlapping_captures_of_one_revision_deduplicate_but_keep_every_citation` covers the narrow-then-wide ordering, exact-duplicate pinning, and the same-text-different-path case. When adding reference-style packet fields, assert that every citation target is itself a text-bearing entry.

### [2026-10-14] Documented Windows install never started

- **Observed failure:** Following the README on Windows 11 (PowerShell 5.1, every execution-policy scope `Undefined`, i.e. Restricted) failed at `.\install-ev.ps1` with "running scripts is disabled on this system". The offline example also failed: relative `-Archive`/`-Checksums` paths were rejected as non-absolute.
- **Root cause:** The README ran the saved script directly, which the default client policy blocks; CI only invokes it with `-File` under a runner policy. `Get-AbsolutePath` resolved command-line paths against nothing but an absolute root.
- **Correction/proof:** The README runs the installer as a script block (`& ([scriptblock]::Create((irm …)))`), which execution policy does not gate, and documents a process-only `-ExecutionPolicy Bypass` for saved scripts. Command-line paths now resolve against the PowerShell location after validating the user's own components; PATH entries stay strictly absolute. The installer prints the exact user-PATH command. On the reporting machine, a sandboxed `LOCALAPPDATA` online install via the published v0.1.1 script installed `ev 0.1.1` with all nine sprites; offline install with relative paths and the ambiguous `bin. ` rejection were verified with the patched script.

### [2026-10-14] Native source-context smoke failed after dialogue-frame styling

- **Observed failure:** Native run `38500866295` passed SDK policy and all five native builds, then every installed-runtime row stopped at source capture/revalidation. The local actual-TUI reproduction reached exact payload inspection and failed in `panel_text`: `max() iterable argument is empty`.
- **Root cause:** The capture helper recognized rounded/single/ASCII frame corners, but the new active dialogue used double corners. It could not locate the panel; no incorrect payload or dispatch was established.
- **Correction/proof:** Recognize double corners when extracting the actual panel cells, without asserting a border style or changing exact payload/byte-count/source-identity checks. The corrected actual-TUI journey passed all ten context cases, including CJK/CRLF, redaction, independent captures, purge, changed/missing source, link substitution and missing stored artifacts. Eight execution and six recovery cases also passed. Evidence: `target/pre-011-{context-fixed,execution,recovery}`. Fresh native qualification is still required after pushing this correction.

### [2026-10-14] Original sprites still looked like large broken pixels

- **Root cause:** Correct 96×96 source files still passed through the half-cell renderer. Replacing illustrations with sprites did not remove that lossy character-grid conversion, and the old native gate excluded WSL/Windows Terminal.
- **Correction:** Automatic mode negotiates Sixel/Kitty and actual cell-pixel dimensions through the sole input reader, then preserves source pixels at integer scale. No native support/metrics or insufficient space means text, not a silently substituted mosaic. `--images cells` remains an explicit low-resolution option. Use F4 to inspect the actual renderer.
- **Observed surface:** Windows Terminal 1.24.12741.0 on WSL2 reported Sixel and 10×20 cell pixels. Dedicated-window captures show Eevee, all nine evolutions, both light/dark palettes, image-free overlays and restored portraits; the owned windows exited normally. Captures are under `target/windows-terminal-sprites{,-light,-dark}`.
- **Transport/input evidence:** `scripts/graphics_smoke.py --binary target/release/ev --assets-dir target/sprite-ui-smoke/sprites --output target/original-pixel-release` passed native Sixel, native Kitty, unsupported auto, explicit cells and off. Decoded payloads preserved each opaque source pixel at 4×; Kitty RGB was exact and Sixel differed by at most one channel unit due to percentage encoding. Split/late replies did not change the saved draft. PTY encoding evidence is not another GUI terminal certification.

### [2026-10-14] Browser OAuth returns to localhost with connection refused

- **Confirmed integration behavior / limit:** Codex browser login was selected by default even in WSL/SSH. Its callback belongs to the machine/network namespace running the CLI, not necessarily the desktop browser. Eevee's existing native handoff waits for the provider process to exit; it does not stop it merely because a browser opened. Its owned process group is terminated on cancellation, the ten-minute deadline, or native return. A refused browser callback alone does **not** establish that Eevee killed the listener early.
- **Reproduced WSL host diagnosis:** On the observed Windows 10.0.26200 / WSL2 NAT host, a healthy Linux `127.0.0.1:1455` listener received no connections from Windows localhost. Windows socket binds to both IPv4 and IPv6 port 1455 failed with `AccessDenied` / Winsock `10013`. Read-only `netsh interface ipv4 show excludedportrange protocol=tcp` showed the reserved range **1446–1545**, covering Codex's 1455 and fallback 1457. No early provider termination was established. A loopback bridge cannot bind a Windows-reserved port either. The selected resolution leaves reservations, firewall and WSL settings unchanged and uses official device authorization; no bridge is installed.
- **Preferred Codex recovery:** In **`/connect` → Codex → Connect**, choose **device authorization** and review `["login", "--device-auth"]`. This defaults on detected WSL/SSH. The official flow needs no localhost callback, but personal ChatGPT security settings or workspace policy must permit device login. It does not fix account eligibility or unrelated network failures; unavailable device authorization never silently retries browser/API-key login.
- **Explicit browser recovery:** Leave the native terminal login running while completing browser consent. If the browser refuses localhost and the CLI is still waiting, cancel with **Ctrl+C**, then start a fresh reviewed login rather than reusing its callback URL. Use a browser that can reach the CLI's own loopback, restore appropriate Windows↔WSL localhost forwarding, or for SSH establish a loopback-only tunnel from the browser machine using the **actual port printed by the provider** (`ssh -L 1455:localhost:1455 user@remote` for Codex's default). Do not expose the callback listener publicly, disable firewalls indiscriminately, or paste callback URLs/tokens into Eevee or tickets. WSL forwarding is environment-dependent, not universally broken. See [Microsoft's WSL networking guidance](https://learn.microsoft.com/en-us/windows/wsl/networking) and [official Codex remote-login guidance](https://learn.chatgpt.com/docs/auth#login-on-headless-devices).
- **Provider boundary:** OpenCode and Gemini retain their official native chooser contracts; Eevee does not invent device-auth flags for them. Gemini's provider-owned manual-code option can avoid a loopback dependency where its chosen auth route supports it. Claude SDK credentials remain separate. Every subsequent model request still needs independent Run approval.
- **Devin:** `/connect` → Devin ACP → Connect offers browser OAuth (`auth login`) or explicit manual token paste (`auth login --force-manual-token-flow`) for inaccessible loopback. Manual token paste is not device authorization; use only the official native Devin UI. No fallback or task replay. CLI 3000.11.3 reports internal ACP agent version `0.0.0-dev`, validated separately.
- **Regression coverage / commands:** `python3 scripts/auth_smoke.py --fixture --binary target/release/ev --output target/auth-fixture` uses no provider account or internet: a real PTY and local TCP listener exercise default device selection, unavailable-device refusal without fallback, explicitly selected browser login, foreground stdin, a listener kept alive until the fixture callback, post-return status refresh, Ctrl+C process/listener cleanup, empty model/execution journals, and final terminal restoration. This fixture does not prove human OAuth or Windows↔WSL forwarding. The opt-in official initiation/cancellation journey remains `python3 scripts/auth_smoke.py --live --binary target/release/ev --provider-runtime /path/to/pinned/runtime --output target/auth-live`; it suppresses browser launch, isolates provider homes and never completes human consent or saves native auth transcripts. New coverage is supplied for integrated verification, not recorded here as already exercised.
- **Real WSL device-flow proof:** `python3 scripts/auth_smoke.py --wsl-device --binary target/release/ev --codex /path/to/codex --output target/auth-wsl-device` launches the actual pinned Codex 0.152.1 through Eevee with isolated credentials and guest callback port 1455 deliberately occupied. The official device-code prompt was observed, Ctrl+C returned to Connections and reaped the login, no browser fallback/model request/credential file was created, and terminal modes were restored. Native output and the one-time code are not retained. This proves initiation/cancellation without a usable callback port, not human consent or completed account authentication. Codex may report interrupted device polling as a failed operation rather than a cancelled exit; neither is treated as authenticated.
- **Contract evidence:** Codex [0.152.1 login dispatch](https://github.com/openai/codex/blob/rust-v0.152.1/codex-rs/cli/src/main.rs) maps `--device-auth` directly to the device flow, while bare `login` selects browser OAuth. Its [pinned login implementation](https://github.com/openai/codex/blob/rust-v0.152.1/codex-rs/cli/src/login.rs) waits on `server.block_until_done()` and exits with an error on device-flow failure. Eevee retains those literal arguments and does not substitute methods after approval.

### Model picker has no selectable models or reasoning values

- Opening `/model` or `/effort` uses cached state and can automatically inspect metadata when empty, including before task creation. `/models-refresh` provides a separately reviewed refresh. Inspection sends no task prompt; an error retains the previous list visibly stale. Exact-ID entry remains available independently of list membership.
- An unauthenticated Devin 3000.11.3 session can report a current model with an empty option list. That default is not evidence of selectable models or entitlement; connect through the native login and explicitly refresh again. No guessed model list is substituted.
- ACP reasoning options can belong only to the model active during inspection. After selecting another model, refresh the catalogue for that selected model before choosing effort. Unsupported controls show an explanation and reject requests instead of ignoring the selection.
- Claude SDK effort choices are advertised request values, not observed applied settings; organization caps/downgrades may apply. Exact model selection uses official setter/context acknowledgment even when discovery omits that ID. Update the complete optional bridge to revision 3 as well as its SDK dependencies; an older bridge is refused before dispatch.

### [2026-10-14] Schema growth changed the quota smoke's payload budget

- **Context / Symptom:** The newly installed schema-6 binary passed installation and the first 20 workbench cases. The fixed 64-KiB quota scenario reclaimed tasks and saved its retained draft, but a subsequent navigation/draft save correctly reported `database or disk is full`.
- **Root Cause:** The old fixture treated total database capacity as payload capacity. New schema pages consumed that fixed total; even its post-reclamation state had insufficient headroom for the next saved state.
- **Solution:** Initialize the actual native schema, measure SQLite page count/size, then allocate a fixed 32-KiB payload budget above that cost. The test still fills the real quota, requires failure with a retained draft, explicitly deletes eligible tasks, retries, preserves an unrelated task and verifies the resumed draft. Production quota limits and error handling are unchanged.

### [2026-10-14] Replacing PATH hid the pinned provider launcher's Node runtime

- **Context / Symptom:** Live implementation acceptance stopped before any model call with `Version executable exited unsuccessfully`. Direct launch reproduced `env: node: No such file or directory`.
- **Root Cause:** The acceptance invocation replaced the host PATH with a minimal list. The isolated official npm Codex launcher requires the existing Node runtime, whose installation was outside those directories.
- **Solution:** Prepend the isolated provider runtime to the existing PATH instead of replacing it. Verified Codex 0.152.1 and Node 24.15.0; the real five-case controlled implementation/denial/journal/local-check/restart journey then passed. No preflight refusal was bypassed, and the standalone local core still requires no Node.

### [2026-10-14] Deleting the first draft before its insert acknowledgement resurrected text

- **Context / Symptom:** Real warm-start measurement typed `x`, deleted it immediately, exited and reopened. The visible editor had been empty, but the next launch restored `x` as a retained draft.
- **Root Cause:** Before the first insert acknowledgement, the task ID was still zero. Returning the editor to its original empty value no longer satisfied the dirty-task guard, so deletion did not advance the revision. The pending older snapshot remained authoritative.
- **Solution:** Treat queued/in-flight initial saves as durable task state for subsequent editor/navigation revisions. A late insert acknowledgement cannot supersede the newer empty editor.
- **Verification:** `clearing_first_draft_before_insert_ack_does_not_restore_deleted_text` failed before the correction and passed after. Actual native performance acceptance then completed all 30 type/delete/exit/reopen startup samples without restoring deleted text; the separate large-diff harness issue did not invalidate those recorded startup observations.

### [2026-10-14] A duplicated writer-lock descriptor outlived store shutdown

- **Context / Symptom:** Concurrent native process/storage tests observed a writer-lock refusal immediately after the storage worker had been dropped and joined.
- **Root Cause:** Closing the owning descriptor does not release an open-description file lock while a duplicate remains open. A concurrent fork can retain that description until exec, despite close-on-exec. A native descriptor-duplication experiment reproduced the lock surviving owner close and releasing only after explicit unlock.
- **Solution:** Close SQLite first, then explicitly unlock the stable sidecar lock during store-state teardown. Do not retry/steal a live writer's lock or weaken conflict detection.
- **Verification:** Added a real descriptor-duplication/store-reopen/save regression. The integrated suite passed 290 tests/two ignored after the shutdown correction. Quota acceptance now derives payload headroom from actual schema page cost rather than assuming schema 6 still fits the older schema's fixed empty-page budget.

### [2026-10-14] Native task-switch smoke raced its own navigation save

- **Context / Symptom:** Run `38425278423` passed all native builds, the three Windows ConPTY regressions, the full installed terminal suite and all four Unix smoke rows. Windows execution acceptance reached its eighth scenario, then remained on task cards with `Finish or retry task/context saves before switching; the current task was retained.`
- **Root Cause:** The harness sent palette/card navigation and the affirmative Enter without observing the intervening navigation-save acknowledgement. Windows correctly refused the task switch while that save was pending; the harness then waited for a switch it had not successfully requested.
- **Solution:** Wait for the actual task-card surface and `Storage: saved`, select the target, wait for its saved state, and only then press Enter. Applied the same prerequisite ordering to source-context task switching. No automatic retry, longer timeout, skipped case, or weakened task-ownership assertion was introduced.
- **Verification:** The corrected harness passed all eight local execution and ten context cases against the actual previous installed binary. The next native run must complete Windows execution, recovery and handoff acceptance; the previous run did not reach the final two suites there.

### [2026-10-14] ConPTY teardown raced asynchronous Windows Job termination

- **Context / Symptom:** Run `38419475447` passed the real ConPTY regressions, all 34 Windows terminal scenarios and installation/workbench/repository/context/execution suites. The abrupt-death recovery case then failed `TerminateProcess` with `WinError 5` after `Session.close()` closed the owning Job.
- **Root Cause:** Job teardown had already begun terminating the child, but its process handle had not yet signalled completion. A second termination request was denied during that interval; an immediate liveness check mistook termination-in-progress for failure.
- **Solution:** Accept the raced access-denied result only after `WaitForSingleObject` observes process completion within five seconds. Other errors and an unsignalled process still fail. Added a real native Job-close/ConPTY-close regression.
- **Verification:** Local six-case execution-recovery smoke passed, including abrupt death/no replay. Run `38425278423` passed all three native Windows ConPTY regressions, including Job-close teardown. Its later task-switch harness race is recorded separately; complete Windows recovery acceptance remains required.

### [2026-10-14] Raw ConPTY child inherited redirected CI standard handles

- **Context / Symptom:** Run `38417411635` failed both native transport regressions before any console output: `GetConsoleMode` raised `OSError: [WinError 6] The handle is invalid`. The child's traceback went directly to the CI log while the ConPTY reader received nothing.
- **Root Cause:** The new `STARTUPINFOEXW` omitted `STARTF_USESTDHANDLES`. With redirected parent streams, child standard handles could retain the runner's pipes instead of being initialized to the attached pseudoconsole. `bInheritHandles=False` alone does not select console standard handles.
- **Solution:** Set `STARTF_USESTDHANDLES` with all three standard-handle slots null, allowing ConPTY to populate console handles without changing or closing the parent's handles. Moved native transport regressions before compilation so this prerequisite fails immediately rather than after packaging.
- **Reference / Verification:** [Microsoft GetStdHandle attachment rules](https://learn.microsoft.com/en-us/windows/console/getstdhandle), [Alacritty's equivalent ConPTY startup isolation](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/windows/conpty.rs). Run `38419475447` passed both real Windows transport regressions and the complete 34-scenario installed terminal suite; local skips were not treated as native proof.

### [2026-10-13] Windows help return capture corrupted UTF-8 border characters

- **Context / Symptom:** Run `38412094582` failed `help back did not restore the invoking surface` at 100×30. `help-back.txt` contained `╰─────────���──────────╯`, wrapped the adjacent border and scrolled the screen; the help overlay itself had closed. The 120×36 journey and initial idle check had passed.
- **Root Cause:** Pinned pywinpty 2.0.15 used winpty-rs 0.4.0, whose native pipe reader called `MultiByteToWideChar` independently for each `ReadFile` chunk. A UTF-8 border split between reads became replacement characters before the harness received a Python string. Splitting `─` (`e2 94 80`) after its first byte reproduces the observed `���` signature. Later waits cannot restore discarded bytes.
- **Solution:** Replaced the harness dependency with `scripts/windows_conpty.py`, a native ConPTY raw-byte pipe transport. The existing incremental UTF-8 decoder now owns decoding on both platforms. Retained exact help-return surface equality, terminal restoration, input and idle assertions. Added real Windows split-byte Unicode, process/resize and cleanup regressions.
- **Verification:** Local macOS terminal smoke passed all 34 scenarios with the integrated reader, and actionlint passed. The two native Windows regressions are explicitly skipped on macOS; hosted Windows verification is required, not inferred from that skip.
- **Reference:** [Pinned lossy decoding implementation](https://github.com/andfoy/winpty-rs/blob/v0.4.0/src/pty/base.rs), [Microsoft ConPTY lifecycle](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session).

### [2026-10-13] Windows startup frames were mistaken for idle output

- **Context / Symptom:** Native run `38374725065` passed four Unix rows and SDK policy, but Windows installation reached the nested terminal suite and failed with `app emitted output while idle`.
- **Root Cause:** The captured initial ConPTY surface had the saved-storage header but not its footer; a fixed startup delay accepted a partially delivered frame. Separately, passive provider discovery unconditionally invalidated the UI, causing a full Windows repaint even while the connections drawer was hidden.
- **Solution:** Apply discovery results without invalidating hidden UI, wait for the complete saved home surface, and drain the bounded in-flight frame before starting the uninterrupted idle observation. Keep zero-output assertions and capture before/after screens plus the exact unexpected output on failure.
- **Prevention / Reference:** `passive_discovery_only_redraws_the_connections_drawer` covers visible versus hidden discovery. Local macOS installed-package acceptance passed 20 lifecycle cases and all 34 terminal scenarios; fresh hosted Windows qualification is required before release.

### [2026-10-13] Embedded Windows installer read uninitialized script-scoped state

- **Context / Symptom:** Native bootstrap succeeded, but `ev uninstall` failed under StrictMode because `$script:animation` had not been initialized.
- **Root Cause:** `-File` execution and invocation through a created ScriptBlock have different local/script scope boundaries. Initializers used local scope while helper functions explicitly read and wrote script scope.
- **Solution:** Initialize all shared animation and running-owner fields explicitly in script scope. Keep StrictMode and ownership checks enabled.
- **Verification:** Actual Windows PowerShell **5.1.26100.9444**, invoked from WSL, executed the full installer through an embedded ScriptBlock against a hash-owned removal fixture: executable/metadata removed, unrelated data retained, no transaction residue. This isolates the scope correction; native CI still proves real running-binary self-update/uninstall.

### [2026-10-13] macOS installer smoke lost its PTY before restoration inspection

- **Context / Symptom:** The animated bootstrap completed its process lifetime, but post-exit `tcgetattr(slave)` failed with `ENOTTY` on macOS.
- **Cause / Correction:** A directly session-leading shell can acquire and revoke the PTY at exit. The POSIX bootstrap smoke now uses a waiting Python session leader with the real installer shell as its child, matching ordinary shell invocation ownership. It retains the exact post-exit termios, cursor, animation, executable-hash and uninstall assertions; no terminal error is suppressed. Fresh macOS CI must verify the correction.

### [2026-10-13] Windows nested source failed Git inventory membership

- **Context / Symptom:** `native_source_identity_survives_json_without_path_normalization` failed capture of `src\資料.rs` with “Source is now ignored or absent”.
- **Root Cause:** Source visibility compared Git's slash-separated inventory bytes directly with the native Windows path.
- **Solution:** Translate Windows separators only at the literal Git query/membership boundary. Preserve native paths in source identities and JSON; Unix backslashes remain literal filename bytes. Borrow unchanged paths and allocate only when conversion is required.
- **Prevention / Reference:** The existing real-Git capture/JSON regression exercises nested Unicode paths and exact CRLF content; native Windows CI is required to qualify this correction.

### [2026-10-13] Cached smoke installation retained orphaned metadata

- **Context / Symptom:** Run `38367147072` failed the first Linux installation with `orphaned metadata exists ... target/native-smoke/installation/installed 空 格/bin/.ev-install`.
- **Root Cause:** Runtime installation state lived inside the cached Cargo target tree. The pinned rust-cache action recursively removes non-build files while retaining directory structure, allowing an empty ownership directory to survive without its executable.
- **Solution:** Keep all native smoke state and evidence in `$RUNNER_TEMP/ev-native-smoke`, outside Cargo's cache. Updated every installed-binary consumer and artifact-upload path together. Retain the installer's orphaned-metadata refusal; do not erase or adopt unknown installations.
- **Prevention / Reference:** [Pinned cache cleanup implementation](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/cleanup.ts); build caching must not restore mutable installation/test state.

### [2026-10-13] Execution warnings and cancellation disappeared in compact terminals

- **Context / Symptom:** Burst output could scroll a transient truncation notice away; at 60x18 a long follow-output header hid the warning. At 39x10 the general resize screen hid owned-work cancellation.
- **Root Cause:** Capture state was represented as ordinary output, and execution confirmations did not participate in the below-minimum layout.
- **Solution:** Added a typed capture-state event and a pinned warning row, rendered compact neutral cancellation, and refused approval below the review minimum. Actual recovery smoke exercised 10 MiB output with a 1 KiB spool, 60x18 warnings, and 39x10 cancellation/approval refusal.
- **Prevention / Reference:** `scripts/execution_recovery_smoke.py`; output truncation and cancellation are safety state, not disposable transcript lines.

### [2026-10-13] Search and nested command overlays lost input ownership

- **Context / Symptom:** Ctrl+K opened the palette while command-output search owned input. Returning through nested form/command overlays could restore an invisible command surface.
- **Root Cause:** The parent global shortcut ran before the search guard, and restoring an overlay frame did not reopen its execution state. Deferred form selection also left its picker open.
- **Solution:** Suppressed palette handling during search/confirmation, restored execution visibility with its frame, and closed the guarded form picker after the transition. Reproduced both focus failures in the real TUI and added actual smoke regressions.
- **Prevention / Reference:** `scripts/execution_smoke.py`; retain the invoking overlay rather than discarding the navigation stack.

### [2026-10-13] Post-commit artifact cleanup reported a fictional rollback

- **Context / Symptom:** Review found that physical spool cleanup failure after committed metadata deletion returned a generic error, leaving callers with obsolete evidence state.
- **Root Cause:** Transactional metadata mutation and fallible filesystem cleanup shared one error result.
- **Solution:** Returned committed metadata plus a typed cleanup warning. A regression exercises committed output purge and task deletion with failing artifact cleanup; UI now refreshes committed state without claiming physical erasure.
- **Prevention / Reference:** `committed_purge_and_deletion_return_metadata_when_artifact_cleanup_fails` in `src/store/tests.rs`.

### [2026-10-13] Smoke key bursts exceeded terminal typeahead capacity

- **Context / Symptom:** Sending 646 Delete events as one burst left 136 command bytes unchanged, causing the next edited command to retain an old suffix.
- **Root Cause:** Treating a long native key-event burst like atomic bracketed paste exceeded the observed input path's capacity.
- **Solution:** Sent Delete events in batches of 32 and waited for the exact remaining byte counter before continuing; retained full command and execution assertions.
- **Prevention / Reference:** `scripts/execution_smoke.py`; use acknowledged key events for editing and bracketed paste only for text.

### [2026-10-13] Credential-prefix matching redacted ordinary task names

- **Context / Symptom:** A direct redactor probe turned `task-card task-navigation mask-value` into `ta[REDACTED] ta[REDACTED] ma[REDACTED]`.
- **Root Cause:** The recognizable `sk-` credential prefix was matched inside ordinary words, risking unwanted task-text changes and rejection of non-secret project paths.
- **Solution:** Required a token-name boundary before prefix detection. The same direct probe preserved all three words; a regression covers ordinary task/project names alongside an actual recognizable credential.
- **Prevention / Reference:** Redaction remains best-effort. Exercise both secret removal and preservation of ordinary text; do not interpret masking as forensic erasure.

### [2026-10-13] Native CI was blocked before runner startup by account billing

- **Context / Symptom:** After successful five-row installation run `38282091481`, run `38285280853` failed every job with zero steps. The check annotation said: `The job was not started because recent account payments have failed or your spending limit needs to be increased. Please check the 'Billing & plans' section in your settings`.
- **Root Cause:** GitHub account billing/spending admission, not compilation, installer behavior, or a test failure.
- **Resolution status:** Historical admission blocker. Run [38354348498](https://github.com/kimmandoo/eevee/actions/runs/38354348498) subsequently started all five native rows; Linux and SDK policy passed, while macOS/Windows exposed real code/fixture failures. Diagnose those step logs rather than attributing current failures to billing. This does not identify which account-setting change resolved admission.
- **Prevention / Reference:** Check-run annotation `114932046529`; [blocked run](https://github.com/kimmandoo/eevee/actions/runs/38285280853). Earlier native installation evidence remains valid only for its own source revision and exercised capabilities.

### [2026-10-13] SQLite no-follow rejected macOS temporary-directory aliases

- **Context / Symptom:** Durable-store integration tests failed to initialize or export with `unable to open database file: /var/folders/.../workbench.sqlite3`.
- **Root Cause:** SQLite's `SQLITE_OPEN_NOFOLLOW` rejects symlinks in ancestor components too; macOS `/var` resolves through `/private/var`. Checking only the final database file did not make that explicit directory alias usable.
- **Solution:** Canonicalized the selected data directory (or read-only database parent) without lossy string conversion, then retained final-file regularity/reparse checks and SQLite no-follow. Alias paths now share the same writer lock. The corrected suite passed 68 tests, including directory-alias writer exclusion.
- **Prevention / Reference:** Keep project identity separate from sanitized display text, and qualify native filesystem behavior rather than applying POSIX pathname assumptions to every platform.

### [2026-10-13] Windows replacement failure can move the old executable

- **Context / Symptom:** Installer review found that the failure path only restored the executable after a successful `File.Replace` return.
- **Root Cause:** Windows documents `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177): replacement can fail after moving the original to the backup name. Treating all exceptions as no-effect could then delete the only old executable with staging cleanup.
- **Solution:** Checked for and restored the backup regardless of the success flag; used a move when the destination was absent and retained staging if rollback failed.
- **Prevention / Reference:** [ReplaceFileW failure states](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew). The regular native locked-file scenario exercises rollback; this rare partial OS failure was identified by static review, not reproduced locally.

### [2026-10-13] Pinned license generator required its CLI feature

- **Context / Symptom:** Installing `cargo-about 0.9.2` without explicit features reported no available binaries.
- **Root Cause:** This release gates its executable behind the `cli` feature.
- **Solution:** Used `cargo +1.90.0 install cargo-about --version 0.9.2 --locked --features cli` locally and in native packaging CI; generated target-qualified license data and complete notices successfully on the local host.
- **Prevention / Reference:** Keep the tool version/features pinned together; notice generation fails packaging on uncovered dependency licenses.

### [2026-10-13] Windows console reflow retained stale editor cells

- **Context / Symptom:** Native runs `38261999485` and `38265625588` failed strict help/back surface equality after 120×36 -> 39×10 -> 120×36 resizing; captures retained editor borders/footer characters and displaced the space after a decomposed combining glyph.
- **Root Cause:** Windows' legacy console cell output and reflow did not preserve the sparse renderer's assumed cell positions. Ordered ConPTY resize notifications and bounded output settling corrected the capture model but were insufficient to fix the actual stale cells.
- **Solution:** Composed displayed cell graphemes on Windows without rewriting draft text, and invalidated the terminal diff buffers for every native resize. Native run `38268855355` then passed the complete first interactive scenario: Unicode/literal paste, draft preservation, strict overlay/back equality, nine-form selection, confirmations, idle output, and clean process/console-mode exit.
- **Prevention / Reference:** Keep exact overlay restoration comparisons and actual native resize captures. Never infer a rendering fix from a text-only parser adjustment or silently normalize stored user text.

- **Indexed recurrence:** User-reported `CJK/combining paste not visible` also occurred in run `38271337325` after eight dark/light Windows cases passed. The indexed draft retained all 29 UTF-8 bytes; captured spaces contained letters from the old underlay. This was a distinct output failure with the same generic assertion, not proof of a repeated old input bug. Fresh state-changed Windows frames fixed the projection while idle stayed event-driven. Native run [38274013013](https://github.com/kimmandoo/eevee/actions/runs/38274013013), revision `9d49447`, passed all 34 Windows scenarios, including indexed, mono/ASCII, linear, and NO_COLOR modes.

### [2026-10-13] ConPTY did not relay alternate-screen escape markers

- **Context / Symptom:** After the corrected Windows interactive scenario exited with status 0 and identical console-mode bits, the generic VT assertion failed with `alternate screen was not left`; its transcript contained cursor/paste restoration but no CSI 1049 entry or exit.
- **Root Cause:** ConPTY projected native console buffers into VT output without relaying the alternate-screen marker expected by the POSIX harness. Absence of an external marker did not prove which native buffer was active.
- **Solution:** Added a native primary-buffer sentinel in the isolated ConPTY console, observed it hidden during the child, and required the exact sentinel to be restored through a fresh `CONOUT$` handle after normal/error/panic exit. Kept mode, cursor, and paste assertions; POSIX still requires the VT alternate-screen exit marker. Native run `38274013013` verified all three exit paths with original `(503, 7)` console modes and preserved primary contents.
- **Prevention / Reference:** Verify native state at the same console boundary used by the program; do not replace state restoration with a missing/not-throw assertion.

### [2026-10-12] Native Windows input lost combining text and paste boundaries

- **Context / Symptom:** Native CI run `38255345859` passed four POSIX hosts but failed Windows ConPTY with `CJK/combining paste not visible`: the editor showed `cafe` rather than `café`. The original Unicode payload was present in the recorded input events.
- **Root Cause:** Crossterm 0.29 represented genuine Alt-code text on VK_MENU release as a release event, which the app correctly ignored for ordinary releases. Its Windows console source also did not emit bracketed `Event::Paste`, so accepting all releases would neither prevent duplicate keys nor preserve pasted controls/newlines.
- **Solution:** Added one Windows console-record reader, enabled VT input, decoded bounded bracketed-paste envelopes before navigation, accepted only provenance-qualified Alt-code release text, and joined UTF-16 surrogate pairs. Repeated records retained repeat semantics; original input-mode bits were captured and restored after raw-mode cleanup. Native run `38274013013` verified literal Unicode paste and restoration in all 34 Windows scenarios; cross-target compilation alone was not used as runtime proof.
- **Prevention / Reference:** Kept literal-paste, Unicode CSI, Escape/modifier, and UTF-8-boundary/drain regressions. The real ConPTY harness retained the original combining-text assertion and console-mode before/after checks.

### [2026-10-12] macOS could not compile Windows ring without an MSVC SDK

- **Context / Symptom:** Full `cargo check --target x86_64-pc-windows-msvc` on macOS failed in ring's C compilation because `assert.h` was unavailable.
- **Root Cause:** Installing Rust's Windows standard library did not supply Microsoft's native C headers/toolchain.
- **Solution:** Type-checked and linted the actual Windows input module in a temporary dependency-minimal package; retained full native Windows CI for the real build and ConPTY acceptance. Did not remove TLS, weaken dependencies, or claim a full Windows cross-build.
- **Prevention / Reference:** Use the native `windows-2022` workflow row for full Windows evidence; isolated checks only validate Rust/API types.

### [2026-10-12] A named-pipe asset blocked startup before the first frame

- **Context / Symptom:** A real PTY launch with `133.png` as a FIFO stayed alive with zero startup output while no writer existed.
- **Root Cause:** Opening the path with a blocking `File::open` happened before checking the opened handle's regular-file metadata. Decode/read byte limits cannot bound a wait inside `open`.
- **Solution:** On Unix, open assets/manifests with `O_NONBLOCK`, then reject every nonregular handle before reading. The opened-handle check also covers path replacement races; ordinary PNG reads retain their byte limits. Added an isolated FIFO regression with no writer.
- **Prevention / Reference:** File-type checks must not themselves require an unbounded open; validate both the I/O boundary and the decoder boundary.

### [2026-10-12] Native image payload width skipped the remaining TUI

- **Context / Symptom:** In real Ghostty 1.3.1, the initial portrait and nine-image picker drew only the rows preceding the first image; later controls were missing and the old workspace remained visible behind the picker.
- **Root Cause:** `ratatui-image` 11.0.0 stored an entire escaped Kitty row, including base64 transmission, in one buffer cell. `ratatui-core` 0.1.2 measured that symbol as ordinary visible text and advanced its diff iterator past subsequent controls. An idle process sample showed the app waiting for input, ruling out a blocked image writer.
- **Solution:** After rendering each native image, set `CellDiffOption::ForcedWidth` on each encoded row's first cell to its physical image width. Keep the library's trailing-cell skip directives. Verified all nine native portraits and the complete picker in the real terminal; added a regression covering adjacent/later text and image removal.
- **Prevention / Reference:** Escape payload byte/display width is not terminal-cell geometry. Recheck the adapter when either pinned Ratatui or image dependency changes; text-only PTYs cannot qualify pixel rendering.

### [2026-10-12] Native VT Backspace arrived as Ctrl+H

- **Context / Symptom:** The real PTY editor smoke failed with `backspace split a combining grapheme`: sending the VT BS byte left the `café` draft unchanged.
- **Root Cause:** Crossterm represented BS as `Char('h')` with Control rather than `KeyCode::Backspace`; the modal editor intentionally ignored control shortcuts. Other terminals send DEL for Backspace.
- **Solution:** Normalize Ctrl+H to the existing grapheme-aware Backspace handler inside the editor only. Kept Control shortcuts suppressed elsewhere; verified the PTY scenario and added a combining-grapheme regression. Added Ctrl+N as a portable newline key because some terminals cannot distinguish Shift+Enter from Enter.
- **Prevention / Reference:** Exercise actual VT/ConPTY byte input in addition to reducer key-event tests.

### [2026-10-12] GUI verification required native Ghostty scripting

- **Context / Symptom:** System Events automation returned `osascript is not allowed assistive access. (-25211)`. `ghostty +new-window` was unsupported on macOS, and tab changes invalidated earlier CoreGraphics window IDs.
- **Root Cause:** GUI accessibility automation was not authorized, and the CLI action was not the macOS window-creation interface. Capturing an old window ID produced stale surfaces rather than the running app.
- **Solution:** Used Ghostty's own scripting dictionary to create isolated verification windows, activate their exact terminal IDs, and send terminal actions without changing accessibility permissions. Looked up the current CoreGraphics window ID by the dedicated verification title before each capture.
- **Prevention / Reference:** Capture only the dedicated app surface and qualify actual terminal/version behavior; do not infer a rendering failure from an inactive sibling tab or stale window snapshot.

### [2026-10-12] Fullscreen startup waited for a cursor-position reply

- **Context / Symptom:** The first real PTY launch exited with `eevee: The cursor position could not be read within a normal duration` before drawing the workspace.
- **Root Cause:** Ratatui 0.30.2 resolved to ratatui-core 0.1.2, whose `Terminal::clear()` first queries and then preserves the existing cursor position. A plain PTY automation driver does not answer the device-status request. Preserving that cursor is unnecessary for a newly entered fullscreen alternate buffer, and the query also introduces an avoidable terminal round trip.
- **Solution:** In `src/platform/terminal.rs`, clear the new backend directly and explicitly set the cursor to `(0, 0)` before the first draw. Keep failures visible and restore terminal modes; do not catch and ignore a cursor timeout. Rebuilt and launched the actual TUI successfully without a cursor-query responder.
- **Prevention / Reference:** For fullscreen initialization, use `Backend::clear()` plus explicit cursor positioning rather than cursor-preserving `Terminal::clear()`. The real PTY lifecycle probes verified no `ESC[6n` request and verified normal/error/panic restoration. This choice is specific to a new fullscreen buffer, not an inline viewport with existing content.

### [2026-10-12] macOS PTY cleanup checks needed a supervising session

- **Context / Symptom:** Reading the slave terminal attributes after its controlling session exited raised `termios.error: (25, 'Inappropriate ioctl for device')`. Fast-exit lifecycle probes also differed only in the terminal's `PENDIN` flag; a probe that waited without draining output timed out. Pyte's display conversion raised `IndexError: string index out of range` after overwriting wide-character continuation cells during a resize/edit sequence.
- **Root Cause:** macOS revokes the controlling PTY when its session leader exits. `PENDIN` is a kernel-managed pending-input flag rather than a user-facing raw/canonical mode. A PTY has bounded output capacity. The installed Pyte display helper did not tolerate retained empty continuation cells after a wide glyph was overwritten.
- **Solution:** The throwaway smoke driver kept a supervising process as the session leader, captured terminal attributes immediately after the application returned, and compared all attributes except `PENDIN`. It drained output while waiting and treated orphan empty continuation cells as blank when exporting the emulated display; valid wide-character continuations stayed zero-width. Normal exit, returned I/O error, and panic restored the terminal, including cursor visibility, bracketed paste, and alternate-screen state. Panic diagnostics appeared after leaving the alternate screen.
- **Prevention / Reference:** Keep application behavior separate from terminal-emulator behavior. Exercise real subprocesses, consume PTY output concurrently, and capture restoration before closing the controlling session. Screen images under `target/smoke/` are rasterized captures of real PTY output, not screenshots of a separate GUI terminal. The temporary lifecycle probe source was removed after verification.

### [2026-10-12] Themed SVG preview was not visually inspectable

- **Context / Symptom:** The static image preview of `docs/TUI_MOCKUP.svg` appeared entirely black/invisible, so it could not be used to assess the updated composition.
- **Root Cause:** The SVG uses CSS theme variables and `currentColor`; the standalone preview did not produce usable contrast. The exact preview-renderer cause was not established, and this was not an application TUI failure.
- **Solution:** Rendered the same SVG inside Chromium with explicit inherited theme variables, then overlaid the actual-source Eevee PNG and displayed all nine species images in a separate picker study. Inspected the resulting composition, confirmed all ten displayed images loaded, and checked SVG text bounds. Reported the static preview mismatch to the tool issue reporter.
- **Prevention / Reference:** Validate theme-dependent design assets in a theme-aware renderer rather than treating an unusable converter image as evidence of an application bug. `target/design-artwork/sanctuary-preview.png` is a local concept preview, not native terminal graphics verification or a checked-in franchise asset.

### [2026-10-15] Runner self-spawning tests failed after moving modules into folders

- **Context / Symptom:** After moving `runner.rs` to `src/workspace/runner.rs`, nine `workspace::runner::tests::*` cases failed at their first assertion while every other test passed.
- **Root Cause:** Those tests re-launch the test binary with a hard-coded test filter string `"runner::tests::native_fixture"`. The module path became `workspace::runner::tests::native_fixture`, so the child ran zero tests and produced no fixture output.
- **Solution:** Updated the filter strings to the new path. `store::tests::interrupted_migration_process_fixture` kept its path because store tests stayed at `store::tests`.
- **Prevention / Reference:** Before moving a module, `grep -rn '"[a-z_:]*tests::' src` to find self-referencing test filters. When extracting an inline `mod tests` into `tests.rs`, keep the original indentation and run `cargo fmt` instead of dedenting by hand, so multi-line string literals stay byte-identical.

### [2026-10-15] Installer always chose the ASCII ball despite a UTF-8 terminal

- **Context / Symptom:** With `LANG=C.UTF-8`, the new pixel-art installer animation still drew the ASCII fallback in a real PTY.
- **Root Cause:** `install.sh` sets `LC_ALL=C` on line 4 for predictable parsing, so the later UTF-8 check always saw `C`.
- **Solution:** Capture `user_locale=${LC_ALL:-${LC_CTYPE:-${LANG:-}}}` before forcing `LC_ALL=C` and decide the ball style from it. UTF-8 bytes still pass through `printf` unchanged under the C locale.
- **Prevention / Reference:** Any terminal-capability decision in the installers must use the user's environment captured before the script normalizes it. Windows PowerShell has the same class of issue with the console code page; `Test-BlockGlyphs` round-trips the glyphs through `[Console]::OutputEncoding` before drawing them.
