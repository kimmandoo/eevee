# Tasks

## Active work

**25 of 31 tasks passed scoped acceptance**, including P02 and U01/L01/G01/S01/S02/S03. Schema-6 specialist workflows, approved scoped memory and plan-only recipes passed all five native build/installed-runtime rows at `d198fa5`; SDK policy also passed. All specified local performance workloads and real Codex/OpenCode scenarios completed. The user-authorized **0.1.0 core release** promotes those exact qualified artifacts. **A04/A06/full A07 remain unchecked pending permitted Claude credentials and completed eligible Gemini authentication/model conformance; full Q01/Q02/P03 retain those dependencies.** Core acceptance is not advertised as complete provider interoperability.

**Current request (2026-10-14):** Finish unchecked tasks through section F in backlog order, commit/push each accepted increment, then publish **0.1.1** from freshly qualified same-commit native packages. The unchecked items in that scope are **A04, A06 and A07**; sections E/F already passed their stated scoped demos. Permitted Claude SDK credentials and authenticated eligible Gemini ACP remain prerequisites until real probes establish otherwise. Do not reuse Claude subscription tokens, fake provider evidence, or release before the requested F completion.

Contract: [design rules](docs/DESIGN_RULES.md). Architecture and phase gates: [implementation plan](docs/IMPLEMENTATION_PLAN.md). Adapter/evidence contract: [agent compatibility](docs/AGENT_COMPATIBILITY.md). An unchecked task may have partial implementation, but its acceptance is not complete; consult its evidence note and the checkpoint.

## Completed baseline

- [x] English product plan covering Eevee and its eight canonical evolutions.
- [x] Phase 1 native Rust shell: terminal lifecycle, nine form identities, responsive navigation, themes, and session-local goal editing.
- [x] Phase 1 verification: 12 tests, warning-free Clippy, release build, real PTY interaction, and normal/error/panic restoration.
- [x] Refine the companion experience, visual contract, conceptual mockup, and dependency-ordered backlog. Documentation only; no new runtime features were delivered in this refinement.

## Delivery rules

- One task is complete only after its user-visible demo works in the real TUI. Build-only work, a mock response, or an attractive inactive control is not completion.
- Read existing patterns and touch the smallest relevant modules. Add runtime/store/repo/agent modules only when their task needs them; do not pre-create empty abstractions.
- Preserve keyboard operation, cancel/quit, terminal restoration, user changes, and honest evidence. New capability must not be inferred from a form or recipe.
- Exercise changed behavior, then run relevant tests/checks once after integration. UI verification uses actual terminal captures, not wording snapshots. Keep regression tests for real boundaries and failures.
- Update the changelog/checkpoint and commit each completed query increment. Record unfinished acceptance explicitly; never mark an entire later phase complete because one task passed.
- Never require every form in a user task. Shared task facts and selected context avoid repeated prompts and unnecessary model calls.
- A form, execution agent, model, and control boundary are separate choices. Never promise sandboxing, portable provider sessions, complete prompt visibility, or exact usage when the integration cannot establish them.
- Core baselines are qualified for the observed macOS arm64/x86_64, Linux x86_64/arm64, and Windows x86_64 OS/terminal tuples in README. Older OS versions, other GUI terminals, filesystem/process parity, and packages are not inferred supported. Windows acceptance uses native ConPTY, not WSL; Windows ARM64 remains unclaimed.
- Pick the first dependency-ready, unblocked task in backlog order. Record blocked acceptance explicitly and continue independent ready tasks; never skip a dependency or mark a blocked gate complete. Native graphics is not a prerequisite for keyboard navigation, durable storage, or the cell-rendered picker.

## A — Make Eevee feel like a companion

### V01 — Sanctuary layout and visual hierarchy

- [x] Delivered the sanctuary visual increment.
- **Depends on:** Phase 1.
- **Change:** Refine `src/ui.rs` with centralized theme tokens, spacious wide-screen gutters/padding, compact layouts, a single task/action card, and a contextual footer. Keep current backend/persistence limitations truthful. Put state in the header, identity in the rail, artifact in the center, and permissions/provenance in the inspector; remove duplicated paragraphs. Do not add persistence or new provider calls.
- **Accept:** Launch the real binary at 120x36, 100x30, 70x24, and 60x18; create/edit a session goal, open drawers, and quit. The primary action and selected text remain readable in dark/light/indexed/mono/ASCII modes; measure the app's own true-color contrast. Task/form/agent readiness and local-vs-provider action are identifiable without opening help. Capture layouts; no overlapping controls, duplicate state paragraphs, or idle redraws.
- **Evidence:** Real PTY size/theme/input/overlay/exit scenarios and actual Ghostty captures; measured nonblank true-color text contrast minima 7.14:1 dark and 5.19:1 light. Primary action stays pinned above scrollable artifacts; modal footer and inactive title readability were corrected.

### P01 — Native platform baseline and terminal lifecycle

- [x] Established the verified native core baseline on the target OS/architecture matrix.
- **Depends on:** V01.
- **Change:** Add native build/test/lint CI and target-qualified terminal smoke coverage for macOS arm64/x86_64, Linux x86_64/arm64, and Windows x86_64. Record actual minimum supported OS/toolchain and terminal versions. Isolate platform differences only where needed; preserve one Rust UI and local use without Node, account, or artwork. Keep build, interactive runtime, graphics protocol, and agent availability as distinct evidence columns.
- **Accept:** On every target, build the release binary and exercise real input, resize, CJK/paste, Ctrl+C, non-TTY help/errors, and normal/error/panic restoration using the existing text/mono/ASCII modes; image-specific checks belong to V02A/V02B. Include native Windows Terminal/ConPTY; WSL and cross-compilation do not prove Windows runtime support. Record artifacts and host/terminal versions. Missing native hardware/runner or terminal access leaves the affected row and P01 pending, never “supported” from compilation alone.
- **Evidence:** Native run [38274013013](https://github.com/kimmandoo/eevee/actions/runs/38274013013), revision `9d49447`, passed formatting, locked tests, warning-free Clippy, release builds, and 34 real PTY/ConPTY scenarios per row. README records observed OS/kernel/terminal baselines and Rust 1.90.0. Windows verified literal Unicode paste in all seven modes and native primary-buffer/console-mode restoration on normal/error/panic exits; no WSL, GUI-terminal, package, or provider claim follows.

### V02A — Actual nine-species assets and cell rendering

- [x] Showed real Eevee/evolution artwork from a local, source-tracked cache.
- **Depends on:** V01.
- **Change:** Map the nine species IDs in the design contract to one consistent transparent official-artwork PNG set. Support a local asset directory and an explicit user-initiated source download; record source/revision/hash/attribution without bundling franchise bytes by default. Bound PNG decode/allocation and render the same image as cached half-block cells or luminance ASCII/mono. Remove the hand-drawn `motif` path when cutting over; missing/corrupt art gets a truthful asset state, not a substitute mascot. Images never enter LLM context.
- **Accept:** Load all nine real images, switch forms offline, and capture actual cell-rendered portraits at wide/compact sizes. Verify aspect/alpha, NO_COLOR/ASCII, missing/corrupt/oversized files, and no network/model call on selection. Native pixel fidelity is not claimed by this task.
- **Evidence:** Downloaded and SHA256-verified all nine pinned sources, then reused the cache without downloading. Exercised 63 offline switches across seven terminal modes, missing/corrupt/oversized PNG states, alpha/aspect/mono boundaries, and a FIFO startup regression. No artwork bytes are bundled.

### V02B — Native terminal artwork and graphics lifecycle

- [x] Displayed the same source images at pixel fidelity on the qualified native terminal.
- **Depends on:** V02A.
- **Change:** Pinned PNG-only image rendering, bounded source/cache loading, integer native-pixel fitting and owned graphics cleanup. The current cutover negotiates Sixel/Kitty and cell-pixel metrics through the sole bounded input owner, retaining late/split response ownership. Auto never substitutes half-cell mosaics; explicit cells/off remain available.
- **Accept:** Show Eevee and all eight forms in an actual graphics-capable terminal, not only TestBackend or a text PTY. Exercise image-bearing overlays, repeated form switches, resize, dark/light surfaces, and quit without stale images. Validate each native protocol before advertising it. Exercise absent/invalid cell metrics and unsupported/multiplexed paths without a blocked first frame, stolen keys, response text in drafts or colored native graphics under NO_COLOR.
- **Evidence:** Historical macOS arm64/Ghostty 1.3.1 nine-image/portrait, overlay, resize and exit captures qualified the previous Kitty route. Current Windows Terminal 1.24.12741.0/WSL2 Sixel captures show Eevee/all nine forms, both palettes, image-free overlays and restored portraits. The five-mode final-release PTY smoke decodes original native pixels and proves exact draft retention through late/split replies, resize and cleanup; basic Kitty payload proof is not qualification of another GUI terminal.

### V03 — Evolution picker and deliberate handoff preview

- [x] Made choosing a form an understandable local interaction.
- **Depends on:** V02A.
- **Change:** At >=90x30, show a 3x3 grid of actual species thumbnails with a selected detail area; otherwise use a compact list and selected portrait. Show the role, expected artifact, current form, and prerequisite status. Arrows/1-9 preview; Enter selects; Esc restores the previous form. Keep task text intact. Previewing or selecting grants nothing and starts no work; unavailable provider actions are labelled unavailable rather than simulated.
- **Accept:** Select/cancel each form using only the keyboard at wide and narrow sizes. Confirm the goal and focus survive; selection and activation are distinct; all nine forms are reachable. Capture color/mono/cells and missing-art states without requiring V02B. After V02B ships, also verify native images are removed behind overlays instead of bleeding through; Q01 owns this composed check.
- **Evidence:** Grid/list selection, cancel, nested local preview, unchanged goal/focus, and absent-art states were exercised in the real PTY; native overlays and compact selection were inspected in Ghostty.

### UX01 — Predictable focus, editing, and accessible local navigation

- [x] Made the local shell usable without relying on artwork or memorized shortcuts.
- **Depends on:** V01.
- **Change:** Implement focus restoration, per-surface selection/scroll state, grapheme-aware cursor editing, and explicit local-apply versus request-preview actions. Suppress global shortcuts in text/search/answer fields. Add a linear text layout with the same local actions, optional portraits, and readable monochrome selection; do not claim assistive-tool support before testing it. Use the existing text picker without requiring artwork. Distinguish Esc=discard an editor draft from Esc=close a pending-request view; keep a visible hint. Below minimum size, retain draft/run state and expose safe quit/cancel rather than an inescapable resize screen.
- **Accept:** Complete create/edit/select/back/resize/quit using only keyboard navigation with CJK/combining text and multiline paste, including absent artwork. Pasted commands/newlines never execute or approve anything. Closing a drawer restores focus, resize below/above minimum preserves drafts/selection, and linear layout exposes the same actions. A dirty editor warns before destructive discard without requiring confirmation for an unchanged draft. Exercise a real assistive terminal setup before labelling that combination screen-reader compatible.
- **Evidence:** Real keyboard/CJK/combining/multiline paste, safe keep/discard/quit, local apply, bounded scrolling, linear layout, and below-minimum resize scenarios passed. Ctrl+N provides portable newline input; Shift+Enter is conditional on terminal reporting. No screen-reader compatibility is claimed.

**Gate A:** completed for the recorded visual/input/artwork surfaces; P01 separately passed all five native core rows. Current native-pixel rendering requires negotiated Sixel/Kitty and cell metrics; unsupported routes show text, with low-resolution cells only by explicit opt-in. No persistence, repository inspection, agent execution, or distribution acceptance is inferred from this visual gate.

## B — A useful local workbench before connecting a model

### E01 — Durable goal and save acknowledgement

- [x] Made one project goal and selected form survive application exit.
- **Depends on:** UX01.
- **Change:** Add a SQLite store worker and bounded worker/input events without blocking rendering. Use a documented platform user-data directory with `--data-dir PATH` for explicit isolation. Store the current task goal, project directory, and selected form transactionally. Version the schema and use transactional migrations; refuse destructive open of corrupt/newer schemas. Enforce one writer per data store with an explicit busy/retry state, not silent takeover. Match saving/saved/save-failed to committed edit revisions; a late acknowledgement cannot label a newer draft saved. Quitting during a pending or failed save must not silently discard edits. No source indexing yet.
- **Accept:** In a temporary data directory, edit a goal, wait for the save acknowledgement, quit, and reopen in the same project. Recover exact text and form. Exercise read-only/disk-full/locked storage, two app instances, rapid edits with delayed acknowledgements, and quit-before-ack. Migrate an older fixture and interrupt a migration; keep the last valid committed database. Corrupt/newer schemas remain untouched with a recovery/export path for readable data, never an automatic reset.
- **Platform sequencing:** Apply the P01 path/terminal contract from the start, but do not block local storage implementation on access to every native runner. P02 depends on completed P01 and verifies this storage path across the entire target matrix before any multiplatform claim.
- **Evidence:** Rust 1.90 passed 68 tests, warning-free Clippy and release build. Seven real PTY scenarios passed exact multiline/CJK/combining goal/form save-exit-resume, writer conflict/retry and project isolation, locked SQLite failure/retry, unchanged corrupt/newer databases, migration/read-only export, and quit immediately after apply. Tests exercised revision-late success/failure, real SQLITE_FULL save/migration rollback, read-only/alias exclusion, and subprocess-exit hot-journal recovery before migrating 101 committed legacy rows. P02 still owns full native parity.

### E02 — Eevee task cards and resumption

- [x] Turned the home screen into durable project-scoped task cards.
- **Depends on:** E01, UX01.
- **Change:** Create/select/resume multiple project-scoped tasks, edit acceptance criteria, and record an explicit next action. Keep task outcome, request lifecycle, save acknowledgement, and evidence freshness separate. Persist per-task drafts/navigation without cross-task leakage. Replace the session-only CTA with Create/Resume only now. Use actual saved timestamps/status; no fictional activity or completion from a form switch/process result. While a run is active, offer Stay or Cancel-and-wait before switching task/project; never silently reassign run ownership. Editing a completed task's goal/criteria requires an explicit reopen transition.
- **Accept:** Create two tasks with different goals/criteria/forms/drafts, switch, restart, and resume each without cross-contamination. A new project starts empty. Pending edits are resolved before switching; unmet criteria prevent completion and changed criteria reopen a completed task explicitly. Once R01/A02 ship, exercise switching with an active run/pending approval: remain on the original task until cancellation settles, invalidate old grants, and retain its partial journal. Q01 verifies the composed path.
- **Evidence:** Rust 1.90 passed 77 tests, warning-free Clippy and release build. Nine real PTY scenarios included two distinct tasks at 120x36 and 60x18, exact goals/criteria/next actions/forms, retained per-field drafts, saved focus/layout, switching/restart, unmet-criteria refusal, neutral completion, and explicit reopen before goal changes. Schema 0/1→2 migrations and task/selection transaction boundaries passed; R01's active-run switching composition remains assigned below.

### E03 — Task lifecycle, privacy, and bounded local storage

- [x] Added reversible archive/restore, explicit scoped deletion, privacy and bounded task storage.
- **Depends on:** E02.
- **Change:** Separate reversible archive/restore from delete/purge. Preview affected task records and currently owned artifacts; use neutral initial focus and explicit confirmation. Redact identifiable secrets before persistence and disclose limits; implement configurable local quotas/retention for existing task data without silently evicting active evidence. As those features ship, C02 applies the policy to excerpts, R01 reserves bounded spool capacity before execution and rejects deletion during active work, R02 represents purged/truncated output explicitly, and S02 preserves approved shared memories. Do not pre-create unused spool or memory modules in E03.
- **Accept:** Archive, restore, cancel deletion, confirm deletion, and restart; only the selected task's eligible local data disappears. Exercise disk-full/quota behavior against real stored task data, preserving drafts and responsive navigation; test identifiable-secret redaction before storage. C02/R01/R02/S02 verify their own retention integration when implemented, with Q01 covering the composition. Explain that local purge neither guarantees forensic secure erasure of SQLite/WAL/backups nor deletes provider sessions, exported files, or repository changes.
- **Evidence:** Rust 1.90 passed 90 tests, warning-free Clippy and release build; eleven real PTY scenarios passed, including compact neutral deletion preview, archive/restore/restart, explicit retention purge, before-storage credential masking, preserved repository/export bytes, and a real 64-KiB database quota failure with retained draft and explicit reclamation/retry. Inspected the actual 60x18 deletion surface. Later context/spool integrations remain assigned to their owning tasks.

### C01 — Repository baseline and working diff

- [x] Added an immutable session baseline and read-only raw working diff with unattributed external changes.
- **Depends on:** E02.
- **Change:** Add repository discovery, initial tracked/untracked file identities, and a read-only working diff in a repo worker. Preserve dirty user files. Label external or ambiguous edits as unattributed; handle a non-Git directory as an explicit unsupported repository state without losing its task.
- **Accept:** Open a disposable dirty Git repository containing tracked/untracked edits; take the baseline, edit externally, and inspect the result. Existing user changes remain unchanged, new modifications are visible without false authorship, and the UI stays responsive while loading.
- **Evidence:** Rust 1.90 passed 103 tests, warning-free Clippy and release build. Three real PTY scenarios passed: dirty tracked/untracked baselines and external edits at 120x36/60x18 with byte-exact file/index preservation, plus non-Git unsupported inspection with durable task restart. Inspected the compact CJK raw diff. Native installed-binary CI now includes the scenario, but execution remains billing-blocked.

### C02 — Vaporeon source map and context basket

- [x] Assembled persisted, inspectable source context with source and stored-reference revalidation.
- **Depends on:** C01, E03.
- **Change:** Browse/search real files, inspect ranges, and pin/remove source excerpts with path/range/source identity. Respect ignore rules, explicit exclusions, binary/oversized-file limits, and symlink boundaries. Preview Eevee-sent content/bytes; label token estimates and provider-added context separately. Do not claim this basket is the full effective provider prompt or that agent-owned reads are constrained by an excerpt selection.
- **Accept:** Build a context basket in a fixture repository with CJK paths, a binary file, excluded credentials, and an out-of-root symlink. Inspect exact ranges, remove one, and modify another; show the stale reference. Excluded/out-of-scope content is absent from Eevee's stored basket/outgoing payload; disclose agent-added history/config/reads rather than asserting unverified global isolation.
- **Dispatch boundary:** Revalidate selected source identities and resolved containment after preview, including symlink/junction replacement. Changed content/scope invalidates the preview and requires review again; never silently send substituted bytes. A02 consumes this contract. Do not claim this controls independent agent-owned reads.
- **Retention integration:** Store excerpts under E03's quota/redaction/purge policy. Purge one task's excerpt without deleting the source file or another task's reference; inspect explicit unavailable evidence and refuse assembly from a missing artifact.
- **Evidence:** Ten real PTY scenarios passed with CJK/CRLF, credential/binary/ignore/link exclusions, exact serialized payload bytes, restart, independent task references, purge, changed/missing sources, and a deleted stored reference. No provider dispatch was implemented or claimed.

**Gate B:** create -> exit -> resume -> inspect sources works without a model, with explicit archive/purge and storage limits. This completes Phase 2 only when E01, E02, E03, C01, and C02 pass; later run/memory integration checks remain assigned to R02/S02/Q01.

## C — Real execution with visible evidence

### R01 — Jolteon approve, run, and cancel

- [x] Executed explicitly approved local commands with owned-tree cancellation and bounded capture.
- **Depends on:** C01, E03, UX01.
- **Change:** Add exact argv or explicit shell executable/dialect/text, cwd, scope, duration, and enforcement-source details. Direct argv must not silently invoke a shell; `.cmd`/`.bat` and shell metacharacters need an explicitly previewed shell route. Queue approvals with neutral initial focus and correlated IDs; enforce approval at dispatch and reject stale/double answers. Stream sanitized output through bounded buffers/quota-controlled spools under E03; record start/end, exit/signal, and before/after source identities. Own the process tree before exposing execution: POSIX process groups or Windows Job Objects/equivalent verified containment. Refuse a safe-cancel execution route when ownership cannot be established. Keep provider-observed runs distinct from Eevee-launched commands; never duplicate execution to manufacture evidence.
- **Accept:** Approve successful/failing commands, deny and prove no side effect, cancel a child-spawning command, and emit escape sequences/large output. Verify argv with spaces/CJK/metacharacters is delivered literally and explicit shell text uses the displayed dialect. Open an approval while typing, dismiss/reopen it, change its inputs, and attempt a stale/double answer; no focus theft or accidental execution. No surviving owned child, uncontrolled terminal operation, unbounded transcript, or invented success. Test quota exhaustion and retained partial output. Navigation cannot abandon active work; P02 repeats native lifecycle acceptance across all targets.
- **Retention integration:** Reserve bounded spool capacity before dispatch. At quota exhaustion, continue draining into a bounded discard path with a visible truncation boundary, or explicitly cancel under the owned-tree contract; never deadlock a child on undrained output or claim complete evidence. Refuse deleting/purging a task while it owns active work, and prove this refusal has no side effect.
- **Evidence:** Rust 1.90 passed 155 tests, warning-free Clippy and a release build. Eight execution PTY scenarios and six recovery PTY scenarios exercised neutral approval/denial, literal CJK/metacharacter argv, explicit shell failure, child/grandchild cancellation, 10 MiB output with a 1 KiB spool, and save-aware task/form/quit guards. Windows APIs typechecked; native runtime acceptance remains P02.

### R02 — Execution journal and evidence freshness

- [x] Persisted execution metadata/output and exposed stale, ambiguous, interrupted, partial, and unavailable evidence.
- **Depends on:** R01, E02.
- **Change:** Persist executions and output references under E03's retention/quota policy; add Jolteon history and a reusable evidence inspector. Distinguish current, stale, ambiguous, failed, interrupted, truncated, and unavailable evidence. Missing/purged output cannot substantiate a verified result. On restart do not claim a previously running process is still owned/running; reconcile it as interrupted unless ownership is actually re-established.
- **Accept:** Run a check, restart and inspect its captured output, then change an input file and see stale status. Change a file during a run and see ambiguous status. Terminate the app during work and reopen without a false success or stuck running indicator. Purge retained output and exercise quota truncation: keep honest metadata, expose unavailable/partial evidence, and preserve artifacts still referenced by another task.
- **Evidence:** Actual restart never replayed a command; missing/purged output remained unavailable, another task retained its evidence, source changes during/after execution produced ambiguous/stale states, and abrupt application death resumed as interrupted without restored ownership.

### UX02 — Activity, interruption, and recovery without lost context

- [x] Preserved drafts, output anchors, search/follow state, and explicit recovery across execution interruptions.
- **Depends on:** R02, UX01.
- **Change:** Add stable scroll anchors, search, explicit follow-output mode, a new-output indicator, and layer-specific error details. Retain drafts and selected artifacts during pending input, cancellation, connection loss, and save failure. Show cancelling/outcome-unknown states without automatically retrying a potentially mutating action.
- **Accept:** Scroll/search while a real command streams, open/close its pending request view, cancel it, and revisit partial output without jumping to the bottom or losing the draft. Exercise interrupted startup/resume and a failed save; the correct layer/remedy is visible, quit does not silently discard data, and no old command re-executes.
- **Exit contract:** Quit/Ctrl+C during work opens Stay or Cancel-and-exit with neutral focus; another keypress cannot silently force termination. Wait for owned-tree exit or authoritative provider cancellation, with bounded escalation and visible partial results. If external outcome cannot be established, record outcome unknown and require an explicit informed exit. Never imply cancellation rolled back edits. Exercise this together with unsaved/failed-save drafts, nested children, a hung cancellation, and terminal restoration.
- **Evidence:** Real streaming/search/follow, neutral navigation/quit, locked SQLite cancellation with journal-only retry, compact sticky truncation warnings, and 39x10 cancellation/approval refusal passed. Nested command-overlay restoration and search ownership were reproduced and corrected. The core terminal suite also passed all 34 scenarios.

### P02 — Native filesystem and process parity

- [x] Prove local workbench behavior on every target platform, not just compilation.
- **Depends on:** P01, C02, R02, UX02.
- **Change:** Exercise platform user-data paths, repository identity/containment, executable discovery, shell selection, and owned-process-tree cancellation on the P01 matrix. Respect actual filesystem case sensitivity, symlinks/junctions, Windows file locking, and atomic storage semantics without lossy path normalization. Reject unsupported path encodings explicitly rather than merging distinct project identities. Qualify each agent and graphics capability separately by OS/version/terminal.
- **Accept:** On native target hosts, create/save/reopen tasks under space/CJK paths; inspect CRLF sources, distinct/case-aliased project paths, and links escaping the root. Test Windows locked files and POSIX permissions without loss of drafts. Verify executable paths with spaces, literal argv, explicit shell dialect, deny/no side effect, child/grandchild cancellation, quit under streaming, and normal/error restoration. Record host versions and results; do not replace native Windows execution with WSL or assume a provider supported on macOS exists elsewhere.
- **Current native evidence:** [Run 38435667550](https://github.com/kimmandoo/eevee/actions/runs/38435667550), source `d198fa5f691689ffb58991e0b7b0a28124c32a13`, passed all eleven jobs: five native builds, five complete installed-package journeys and SDK policy. Actual Windows ConPTY regressions, space/CJK installation and task persistence, source/path revalidation, literal argv/shell boundaries, denial/cancellation/quit, interrupted/stale/failed-save recovery, handoff and specialist/memory/recipe consent all passed. Host/version/capture evidence is attached as `native-*`; no WSL substitute or provider/graphics parity is claimed.

## D — Connect a real specialist, not nine chat sessions

### A01 — Agent preflight and truthful connection state

- [x] Showed observed prerequisites, explicit executable trust, and unavailable/unknown agent capabilities without a model request.
- **Depends on:** UX02, C02.
- **Change:** Introduce a small versioned agent-profile/capability contract and connection drawer for Codex, Claude Agent SDK, and ACP profiles. Inspect executable/protocol versions, effective configuration/trust scope, authentication/billing source labels, and adapter availability without a model request. Establish executable/config trust before probing: even `--version` executes code, so a repository-supplied binary/hook requires deliberate consent. Qualify readiness per OS, executable version, and action; installed is not authenticated or supported. Missing/unknown is not supported. Show per-action Eevee-mediated/agent-policy/observed boundaries; never mutate global agent configs or silently load an untrusted project hook/plugin.
- **Accept:** Probe the installed Codex interface and missing executables/SDKs; preserve local-only use. A version mismatch, absent auth, or unknown enforcement capability disables the affected run with an actionable reason. No repository-supplied command or auth flow starts without trust/action; no credentials appear in profile snapshots or logs. Record observed versus merely documented capabilities.
- **Evidence:** Rust checks passed 168 tests (two subprocess helpers ignored), warning-free Clippy, and release build. Five real Linux/WSL PTY scenarios exercised missing profiles/local-only editing, repository-executable neutral/cancel/undersize refusal and deliberate consent, executable replacement invalidation, withheld credential-like stdout, and the installed Codex `0.152.1` version probe. Inspected 120x36/60x18 captures; eleven existing workbench scenarios also passed. Known configuration declarations are non-authoritative; no provider auth/status, protocol handshake, model request, SDK runtime, or native Windows acceptance is claimed. Codex `0.147.0` remains a documented reference only; the observed mismatch and all absent adapters keep model actions disabled.
- **D continuation:** Profiles now disclose implemented Codex Plan/controlled Implement, optional pinned SDK and shared ACP1 rather than claiming absent adapters. Runtime/policy/auth gates remain separate. Existing five-case preflight smoke passed again; Gemini's observed Node startup exceeded three seconds, so only that profile now has a disclosed ten-second bounded probe.
- **Authentication continuation:** **F2** exposes provider, action and method lists; compact review starts on Back, with full Details available. Codex browser/device OAuth, OpenCode native chooser and Gemini's official Google/API/Vertex native chooser retain neutral 80×24 review, bounded terminal handoff, no token-store copying/automatic task replay, and fresh Run consent after return. “Use for task” opens the separate provider-switch review. Gemini's pinned ACP auth emitted raw OAuth prompts on JSON stdout; its replacement is explicitly external native CLI handoff, which can continue into chat and does not claim an auth-only sandbox. Runtime initiation/cancellation evidence and remaining human-auth limits are in the checkpoint.

### A02 — Espeon scoped planning request

- [x] Completed real scoped Codex app-server planning with reviewed context, questions/approvals, cancellation/resume, and explicitly saved decisions.
- **Depends on:** A01, E02, C02.
- **Change:** Use the pinned installed-version app-server contract, not ANSI screen scraping, for normalized structured events, approval/questions, cancellation, and explicit thread lifecycle. Let Espeon submit a previewed context manifest and save a user-selected decision. Correlate task/run/generation/tool/request IDs, preserve provider-origin metadata, and do not count delta plus final snapshots twice. Keep one active request; a draft is not a queued submission.
- **Accept:** With real credentials, run a bounded design request, answer a real supported input request, deny/allow a supported operation, cancel a turn, and resume the explicit thread when supported. Reject a stale approval after cancellation, and surface malformed/fatal transport outcomes without fake completion or automatic resend. Inspect outgoing context plus provider-added-context disclosure. Auth/schema availability is a real blocker; mocks do not satisfy this gate.
- **Preview integrity:** Immediately before dispatch revalidate the C02 context manifest, task revision, profile/config, and grants. A changed file or replaced symlink/junction requires a new preview, not an unnoticed payload update. Test a source change between Preview and Run and prove no stale or substituted request is sent.
- **Evidence:** Rust 1.92.0 passed 183 tests (two subprocess helpers ignored), warning-free Clippy, and release build. `scripts/planning_smoke.py --live` passed nine actual-TUI scenarios on Linux/WSL x86_64 using installed Codex `0.152.1`, its generated experimental schemas, ChatGPT account authentication and advertised `gpt-5.6-sol`/low default. Exercised changed source and same-content escaping symlink refusal before dispatch, fatal malformed transport without success/retry, actual blocking question and literal CJK answer, reviewed decision save/restart, explicit same-thread resume and cancellation at 39x10, stale response rejection, command denial and allow-once with observed tool status/output, no automatic reconnect/replay, and neutral undersize safe exit with provider cancellation before storage-aware quit/terminal restoration. Inspected 120x36/60x18/39x10 surfaces; fixed the reproduced stale last-page consent footer. Five preflight and eleven workbench regressions passed. Selected decisions use existing Next action storage with unverified provider provenance; no schema migration. Native Windows/macOS provider acceptance, A03 patches, and other adapters remain unclaimed.
- **D preservation:** The nine-case live planning smoke passed again after common provider routing, role/switch/journal/handoff integration. A02 Plan behavior is preserved; Implement is a distinct reviewed operation, never a silent widening.

### A03 — Flareon bounded implementation

- [x] Produced a reviewed real bounded patch while preserving dirty user work and separating provider/local evidence.
- **Depends on:** A02, R02.
- **Change:** Request a change within the profile's verified edit boundary. Show exact enforcement provenance rather than assuming callback approval implies path/network isolation. Keep baseline/user changes visible, inspect the working diff, and offer deliberate Jolteon verification. Provider-native tool results are journaled with origin/coverage, not mislabelled as local execution. Never auto-commit/merge or silently widen scope.
- **Accept:** Make a real change in a disposable dirty repository, preserve user edits, review against the requirement, and run a chosen check. Deny a protected write and verify no side effect through the advertised route. If the profile cannot enforce a required boundary, refuse that controlled action or require an explicitly labelled external-policy workflow; no reassuring but false read-only label.
- **Evidence:** `scripts/implementation_smoke.py --live` passed five real Codex 0.152.1 TUI scenarios: deny preserved tracked/untracked/index bytes; a substituted symlink toward a protected external file was refused unchanged; an approved selected-file replacement preserved the user's dirty comment and retained native provenance; `w` reviewed the patch and separately approved `j` produced `EEVEE_LOCAL_CHECK_OK`; restart retained provider journals without replay or local-evidence relabelling. POSIX route admits 1–8 existing regular UTF-8 files ≤12 KiB each, with exact before/after consent, protected/excluded/unselected/link/stale rejection. Other platforms refuse controlled writes. Schema 5 adds append-only quota-backed complete/partial provider records, deletion/purge preview and storage-only retry.

### A04 — Claude Agent SDK interoperability

- [ ] Run the same task UX through a supported Claude SDK adapter.
- **Depends on:** A03.
- **Change:** Add an optional, pinned TypeScript Agent SDK sidecar behind the Rust adapter contract. Use documented query/callback APIs rather than recreating its full private transport. Separate canUseTool approval, AskUserQuestion, and MCP elicitation; apply mandatory policy through actual tool/config controls, not only a callback bypassed by preapproved tools. Use the SDK's permitted auth/branding; no bundled claude.ai subscription login or undocumented credential reuse.
- **Accept:** In a disposable workspace with permitted credentials, preview/run, answer a question, deny/allow once, interrupt, and resume by explicit provider session ID. No queued follow-up starts after cancellation; no persistent rule is silently saved; cumulative usage is not double-counted. Verify project-hook/MCP configuration boundaries and show unsupported confinement honestly. The Rust UI/local-only path works without Node/SDK installed.
- **Implemented / blocker:** Optional official SDK 0.3.296 / embedded CLI 2.1.296 / Node ≥24 bridge, strict tool/config policy, correlated questions/permissions, decline/cancel-only MCP elicitation, one query/no follow-up queue, explicit resume and non-summed cumulative usage are implemented. Six policy TAP tests and real `--inspect` passed. Explicit cloud credentials now include Bedrock bearer/web-identity and opted-in AWS SSO/profile/workload or Vertex ADC/workload (`EEVEE_CLAUDE_CLOUD_AUTH=provider-chain`); provider effects are disclosed and the effective SDK backend is checked before prompt release. SDK configuration remains isolated and claude.ai subscription OAuth is not reused. Missing permitted credentials still block live question/deny/allow/interrupt/resume acceptance; module/policy checks do not complete it.

### A05 — ACP client with OpenCode as the first concrete peer

- [x] Exercised the shared negotiated ACP1 client against real OpenCode without claiming a standard sandbox.
- **Depends on:** A03.
- **Change:** Implement negotiated ACP v1 stdio JSON-RPC against OpenCode's documented `opencode acp` entrypoint. Advertise only implemented client callbacks; normalize prompt/update/permission/question/cancel/load semantics. Treat absent capabilities as unsupported, preserve provider option IDs, and gate every client filesystem/terminal request. Keep agent-owned tools/config effects separate from client mediation; ACP transport alone is not confinement.
- **Accept:** With a real compatible OpenCode installation, connect, run, answer supported permission/question requests, cancel with pending requests, and reload the correct cwd/session. Test unknown optional fields, unsupported version/capability, a late response, and a provider-owned tool path. No shared/global server assumptions, duplicate replayed artifacts, silent always-allow mapping, or stdout log contamination.
- **Evidence:** OpenCode 1.17.12's real prompt was rejected by its free service's newer-release requirement, so qualification moved to isolated native **1.18.0 `acp --pure`**, not an auth/model fallback. Real TUI completed a Codex-handoff-derived planning message with `end_turn`, denied an exact permission response, allowed once and observed `EEVEE_ACP_TOOL_OUTPUT` in a completed provider-native journal, explicitly loaded the same cwd/session, cancelled a pending permission and rejected a stale answer. Negotiated metadata-only `session/list` also worked without creating/prompting a session. ACP frame/capability/optional-field/late-response/final-snapshot boundaries passed Rust regressions. Unsupported questions/fs/terminal requests are refused; native provider tools/config/MCP remain external policy; controlled ACP implementation is disabled.

### A06 — Gemini ACP conformance and capability differences

- [ ] Prove the ACP implementation is not an OpenCode-specific adapter in disguise.
- **Depends on:** A05.
- **Change:** Add a version-qualified Gemini CLI profile using its documented ACP launch mode after checking the installed release. Reuse the ACP adapter; keep model/auth/session/question differences in capability/profile data, not UI-specific forks. Account for current account/product eligibility rather than promising a free subscription route.
- **Accept:** Run the same prompt/cancel/permission/context scenarios against real Gemini ACP. Exercise a capability present in one peer but absent in the other: the UI gives a correct reason and does not invoke the missing method. Record peer version, negotiated protocol, auth route, and actual restrictions. Missing executable/credentials block this task, not the local shell.
- **Implemented / blocker:** Gemini **0.28.0 `--experimental-acp`** shares the model adapter and negotiates ACP1/loadSession. Real no-model comparison showed OpenCode's `session/list` succeeds while Gemini omits it and the method is not sent. Missing configured auth produced an actual `session/new` **401**, not proof that an API key is the only route. Login now uses the reviewed official native CLI for Google OAuth/browser/manual-code, API key or Vertex, because this release's ACP authentication mixes native text/code prompts into JSON stdout. Its external native/chat boundary is disclosed and Eevee sends no task automatically. Authenticated model/permission/cancel/context scenarios still need completed human authentication and current eligibility.
- **Actual TUI difference:** The final composed nine-case live smoke visibly showed Gemini's absent `sessionCapabilities.list` and **Method NOT invoked**, then the genuine API-key 401 without model dispatch. A separate three-case Gemini TUI run also verified local navigation, safe quit and exact terminal restoration after both outcomes.

### A07 — Cross-agent handoff and honest external fallback

- [ ] Continue one Eevee task across agents without claiming portable sessions.
- **Depends on:** A04, A05, A06.
- **Change:** Preview switching agent/role policy at an idle boundary. Export/import a versioned task handoff containing approved facts, source references/excerpts, criteria, decisions, and unresolved work; never carry opaque provider session IDs, permissions, hidden reasoning, or credentials across agents. Keep imported results unattributed/unverified until checked. Provide a manual JSON/Markdown handoff for other CLIs instead of scraping or automating their TUI.
- **Accept:** Continue a Codex task with Claude Agent and an ACP peer using an explicitly accepted handoff; the old session remains available, grants reset, and the new request includes only reviewed context. Reject stale pending approvals and unknown/newer handoff schema. Manual export/import works without launching a CLI and cannot create verified execution evidence from user-supplied claims.
- **Implemented / partial acceptance:** Strict schema-1 curated JSON/Markdown, create-only reviewed export/import, explicit idle provider/role switch, original per-provider sessions, grant reset, unverified claims and exact current-task/source dispatch binding are implemented. Four real manual TUI cases passed with no providers on PATH, including session/secret exclusion, equivalent JSON/Markdown, import without execution/evidence/completion mutation and newer-schema refusal. Real Codex→OpenCode continuation completed with reviewed portable facts; a regression found and removed generated session provenance from all adapters' outer task metadata, not merely the handoff. Full Codex→Claude→ACP acceptance remains blocked by A04/A06 credentials and is intentionally unchecked.

## E — Distinct diagnosis, refinement, and review

### U01 — Umbreon hypotheses and diagnosis

- [x] Track explanations against real observations.
- **Depends on:** A02, R02.
- **Change:** Add a hypothesis board with supporting/refuting evidence links and explicit unresolved states. User-requested diagnostic commands use Jolteon's local approval/runner; provider-native diagnostics retain their actual origin and control coverage in the journal. A diagnosis alone does not edit source. Security findings use the same evidence discipline, not severity by guesswork.
- **Accept:** Investigate a seeded failure, reject one hypothesis using an actual run, and support a root cause with source/output references. Missing proof stays unresolved. Handoff to Flareon carries the diagnosis without re-sending the whole transcript.
- **Observed acceptance (2026-10-14):** Native `workflow_smoke.py` rejected overflow using an actual failed check, linked the subtraction defect to captured source/output, retained missing evidence as unresolved, and previewed only the selected diagnosis for Flareon without writing or sending.

### L01 — Leafeon bounded refactor and comparison

- [x] Make improvement measurable and behavior-preserving.
- **Depends on:** A03, R02.
- **Change:** Select a hotspot, record the behavior contract and baseline, request a bounded refactor, and compare relevant validation/measurements. Reuse Flareon's patch and Jolteon's execution machinery, not a second editor or executor.
- **Accept:** Refactor a disposable example with a fixed behavioral contract; compare baseline/post-change output and relevant measurements. A failing check blocks a success claim. Do not claim a speedup from noise, missing baselines, or unrelated checks.
- **Observed acceptance (2026-10-14):** A disposable addition example retained its contract across a bounded refactor. A failed post-change check blocked comparison; distinct ordered baseline/post-change runs used the same executable, argv and cwd. Actual outputs/elapsed times were shown without a speedup claim.

### G01 — Glaceon independent review desk

- [x] Judge the patch against current evidence without editing it.
- **Depends on:** A03, R02.
- **Change:** Build a separate review context from requirements, patch, and selected observations. Add evidence-linked findings and accept/request-changes/inconclusive verdicts; show stale verification next to the affected criterion. Source-writing tools are unavailable to managed review requests.
- **Accept:** Review a seeded defect and an unverified change. Capture an actionable finding/inconclusive verdict, not a generic pass. Edit after a successful check and confirm the review shows stale evidence. Applying a fix requires a deliberate handoff, not Glaceon editing its own finding.
- **Observed acceptance (2026-10-14):** Native evidence-linked findings, coverage-gated acceptance and stale-source invalidation passed. Two explicitly approved real Codex 0.152.1 reviews used distinct non-resumable sessions, excluded the implementation-only next action, identified the seeded subtraction defect and preserved all project/Git bytes. SDK live review remains unclaimed; managed ACP review is refused.

## F — Handoff, approved habits, and recipes

### S01 — Sylveon evidence-linked handoff

- [x] Leave the next session a usable journal, not a vague summary.
- **Depends on:** G01, E02, A07.
- **Change:** Reuse A07's portable handoff schema to preview the goal, observed changes/results, decisions, unresolved risks, and exact next action; do not create a second export format. Include source/run references, distinguish model inferences, and preview any documentation write separately. No automatic declaration of completion.
- **Accept:** Close/reopen a task with one unresolved finding and resume from the recorded next action. Follow each result link to real evidence. Reject a proposed documentation edit and confirm the file is unchanged.
- **Observed acceptance (2026-10-14):** Reopened the unresolved finding and exact next action; followed its stored source and actual run output. Rejecting the separate documentation preview preserved existing bytes. Explicit schema-1 export retained labeled inference/unverified evidence references, not grants or completion. A07's separately blocked full provider chain is not claimed.

### S02 — Explicitly curated project and personal habits

- [x] Let the companion learn only what the user approves.
- **Depends on:** S01, C02.
- **Change:** Offer Remember on a selected verified/user-authored note; preview task/project/personal scope, then approve or reject. Support inspection/edit/delete and honor exclusions. Do not silently infer a permanent habit from one action or add a numerical bond score.
- **Accept:** Approve one project memory and reject another; restart and inspect exact contents. Open a different project and confirm scope isolation, then delete the memory and verify it no longer enters context. Excluded secrets are never proposed from captured content.
- **Retention integration:** Use E03's ownership/purge rules; deleting one task must not remove approved shared memory, and deleting a memory must remove it from future request assembly while explaining that prior exports/provider history are outside local erasure.
- **Observed acceptance (2026-10-14):** Actual approval/rejection, exact-content inspection after restart, separate context selection, deletion removing future context, cross-task/project scope isolation and task purge preserving shared project/personal notes passed. Exclusion, update/revision and dispatch-revalidation regressions passed.

### S03 — Evolution Stone recipe preview

- [x] Reuse workflows without creating a permission bypass.
- **Depends on:** V03, R02, S01.
- **Change:** Load versioned TOML recipes describing form steps, context/scope, validation commands, and completion criteria. Show the recipe before applying it to a task. No install hooks, marketplace, automatic execution, or authority to expand existing grants.
- **Accept:** Preview a local bug-fix recipe, apply its plan without executing commands, and require normal approvals during use. Reject an invalid schema. A recipe requesting an ungranted write/run capability cannot perform that action.
- **Observed acceptance (2026-10-14):** The native all-page preview defaulted to staying; explicit approval persisted only the plan/criteria/next action. Proposed commands did not run, no model was called, and unknown capability/schema fields were rejected.

## G — Integrated acceptance

### Q01 — Complete journey and visual state matrix

- [ ] Prove all nine workflows and their composed user experience.
- **Depends on:** V01, V02A, V02B, V03, UX01, UX02, E02, E03, C02, R02, P02, A03, A04, A05, A06, A07, U01, L01, G01, S01, S02, S03.
- **Change:** Exercise real nine-form artifacts plus the shared UX across Codex, Claude Agent SDK, and the two ACP profiles. Run the compatibility conformance scenarios, not a mandatory all-agent tour for normal users. Maintain separate native-local, graphics-terminal, and provider OS/version evidence matrices; an unavailable optional provider blocks its interoperability gate, not use or independent verification of the local core. Q01 remains pending until every required gate passes. Fix actual integration/visual defects; no fake backend or single-agent-only test qualifies as interoperability.
- **Accept:** Cover empty/missing dependency, ready/running, permission/question, cancellation/unknown outcome, save failure, stale evidence, and cross-agent handoff across supported sizes/themes and long CJK content. Verify keyboard focus, no accidental execution, bounded streams/storage, task switch and quit with pending work/drafts, resumed usage accounting, purge/missing artifacts, image cleanup, and terminal restoration. Exercise E02/R02/S02's deferred integration checks. Every claimed controlled action names and proves its enforcement boundary. Review naming/art rights and provider terms before distribution; unsupported OS/provider combinations remain explicitly unavailable rather than implied portable.
- **Observed / remaining (2026-10-14):** Five complete native rows passed at `d198fa5`, including 15 specialist cases per host and composed task/context/execution/recovery/purge checks. The local specialist journey also passed dark/light/indexed/mono/ASCII/linear modes with compact/undersized and Unicode states. Actual Codex implementation/review and Codex→OpenCode continuation passed. Required live Claude/Gemini rows remain unavailable; full Q01 is not checked.

### Q02 — Measured native and token efficiency

- [ ] Measure the performance contract rather than advertise unverified speed.
- **Depends on:** Q01.
- **Change:** Benchmark the release binary on recorded hardware/workload: 60s local-shell idle CPU/RSS, 30 warm local startups, key-to-render latency under streaming, 10 MiB output, and a 10,000-line diff. Report core, optional SDK sidecar, and external-agent resource use separately and in aggregate. Inspect per-provider usage/cost scope and outgoing context; optimize measured bottlenecks.
- **Accept:** Report observations against the design targets with method/sample size and remaining misses, without hiding helper/agent costs inside a core-only claim. Queues stay bounded; no model request on form selection, no unsubmitted draft leaks, and no unrelated/full history in Eevee-sent context. Show unknown provider-added context/usage honestly; no savings percentage or merged cost total without compatible measured scopes.
- **Measured local slice (2026-10-14):** All specified release workloads completed on M1 Pro/macOS arm64. Warm-start p95 89.55 ms; 60-second idle 0.826% of one core / 7.44 MiB core RSS; streaming input p95 12.61 ms. A 10 MiB stream retained exactly 1 MiB and visibly discarded 9 MiB; a 10,000-line diff opened in 137.66 ms without changing repository bytes. See the implementation plan for scope/method. Full Q02 remains pending on Q01 and unavailable provider-resource/conformance rows.
- **Additional provider measurement:** Two real independent Codex reviews reported native core, app-owned preflight helpers, app-server descendants, measurement harness and simultaneous aggregate CPU/RSS separately. The implementation plan records the exact windows and sample counts. SDK resources and compatible provider-added context/token/cost totals remain unavailable, not zero or invented savings.

### P03 — Native distribution, upgrade, and support evidence

- [ ] Make installation and upgrades reproducible on the supported platform matrix.
- **Depends on:** P02, Q01, Q02.
- **Change:** Produce versioned native archives and checksums for the P01 targets, with install/run/uninstall instructions and recorded OS/toolchain/terminal baselines. Separate verified core support from graphics and optional provider compatibility. Include no mandatory Node runtime, credentials, or franchise artwork in the core package. Keep user-data paths stable across upgrades and refuse incompatible schema downgrade without changing data. Do not advertise unverified packages or bypass rights/provider-term review.
- **Accept:** Download/unpack and run each package on a clean native target without Rust installed; verify checksums, help/version, the interactive local flow, and absence of account/art/network prerequisites. Upgrade from the previous supported data schema (fixture for the first release), resume a task, and attempt an incompatible downgrade without data loss. Uninstall leaves data intact unless the user deliberately invokes documented purge. Publish per-target build/runtime/install evidence plus graphics/provider limitations; unsigned/unnotarized artifacts and any OS trust prompt must be disclosed, never bypassed through a blanket security-disable instruction.
- **User-authorized 0.1.0 slice:** The latest request explicitly authorizes public release and commit/push, superseding the earlier offline-preview-only decision. Five native packages carry dependency notices and no franchise image bytes. Bootstrap, read-only update check, embedded-installer update, owned uninstall, rolling-ball progress and offline inputs are implemented; the release workflow requires all current native and SDK-policy jobs to pass. This bounded core release does not silently complete P03's remaining integrated/schema/performance/provider acceptance or signing/notarization.
- **Published / verified (2026-10-14):** [v0.1.0](https://github.com/kimmandoo/eevee/releases/tag/v0.1.0) promoted `d198fa5`'s successful five-target packages without rebuild or repeated release smoke. All eight public assets downloaded anonymously and all seven checksum entries matched. Public macOS installation without compiler/provider runtime, version/update check, 15 specialist cases and three actual schema migration/refusal/resume cases passed. Signing/notarization and the full dependent provider/integrated gates remain unclaimed.

## Phase status

Phase 1, all six section-A tasks, all five section-B tasks, R01/R02/UX02/P02, A01–A03, A05 and U01/L01/G01/S01/S02/S03 passed their scoped gates: **25 of 31 implementation tasks checked; 6 remain unchecked**. A04/A06/full A07 still lack permitted Claude/Gemini authentication; full Q01/Q02/P03 therefore remain pending rather than inheriting local-core acceptance. The separately authorized 0.1.0 core release uses the successful exact-commit native artifacts, not an older green fallback.
