# Tasks

## Active work

Section A is complete: V01, P01, V02A, V02B, V03, and UX01 passed their recorded gates. Native run [38274013013](https://github.com/kimmandoo/eevee/actions/runs/38274013013) passed all five OS/architecture rows, including the recurring indexed-color Windows paste/rendering failure. The active request is to finish the offline `ev` installation preview, then implement **all of sections B and C**, beginning with **E01 — Durable goal and save acknowledgement**. Their unchecked acceptance remains pending; storage/provider capabilities are not inferred from this request.

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
- **Change:** Added pinned `ratatui-image` with PNG-only decoding and no Chafa defaults. Used an OS/architecture/terminal-version-qualified native Kitty route with kernel cell-size discovery and one Crossterm input owner; no stdin-consuming query is sent. Cached resize/encoding outside rendering. `--images auto|cells|off` controls native/fallback behavior. Cleared/repositioned owned resources on form changes, overlays, resize, and exit, preserving text layout and terminal restoration.
- **Accept:** Show Eevee and all eight forms in an actual graphics-capable terminal, not only TestBackend or a text PTY. Exercise image-bearing overlays, repeated form switches, resize, dark/light surfaces, and quit without stale images. Validate each native protocol before advertising it; otherwise keep it disabled and use working cells mode. Exercise a missing pixel/cell report and unsupported/multiplexed paths without startup waits, stdin queries, stolen keys, or colored graphics under NO_COLOR.
- **Evidence:** Actual macOS arm64/Ghostty 1.3.1 native nine-image dark/light grids, all nine selected portraits, preview/inspector occlusion, font/viewport resize, CJK editing, and resource-free exit were captured. Missing kernel pixel reports and tmux-qualified rejection used working cells. Other protocols/native tuples remain disabled, not inferred supported.

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

**Gate A:** completed. The companion visual/input/artwork gate passed on the recorded macOS arm64/Ghostty terminal; P01 separately passed all five native core rows. Native pixels remain limited to the qualified Ghostty route, with cells/off elsewhere. No persistence, repository inspection, agent execution, or distribution acceptance is claimed.

## B — A useful local workbench before connecting a model

### E01 — Durable goal and save acknowledgement

- [ ] Make one task survive application exit.
- **Depends on:** UX01.
- **Change:** Add a SQLite store worker and bounded worker/input events without blocking rendering. Use a documented platform user-data directory with `--data-dir PATH` for explicit isolation. Store the current task goal, project directory, and selected form transactionally. Version the schema and use transactional migrations; refuse destructive open of corrupt/newer schemas. Enforce one writer per data store with an explicit busy/retry state, not silent takeover. Match saving/saved/save-failed to committed edit revisions; a late acknowledgement cannot label a newer draft saved. Quitting during a pending or failed save must not silently discard edits. No source indexing yet.
- **Accept:** In a temporary data directory, edit a goal, wait for the save acknowledgement, quit, and reopen in the same project. Recover exact text and form. Exercise read-only/disk-full/locked storage, two app instances, rapid edits with delayed acknowledgements, and quit-before-ack. Migrate an older fixture and interrupt a migration; keep the last valid committed database. Corrupt/newer schemas remain untouched with a recovery/export path for readable data, never an automatic reset.
- **Platform sequencing:** Apply the P01 path/terminal contract from the start, but do not block local storage implementation on access to every native runner. P02 depends on completed P01 and verifies this storage path across the entire target matrix before any multiplatform claim.

### E02 — Eevee task cards and resumption

- [ ] Turn the home screen into a real task sanctuary.
- **Depends on:** E01, UX01.
- **Change:** Create/select/resume multiple project-scoped tasks, edit acceptance criteria, and record an explicit next action. Keep task outcome, request lifecycle, save acknowledgement, and evidence freshness separate. Persist per-task drafts/navigation without cross-task leakage. Replace the session-only CTA with Create/Resume only now. Use actual saved timestamps/status; no fictional activity or completion from a form switch/process result. While a run is active, offer Stay or Cancel-and-wait before switching task/project; never silently reassign run ownership. Editing a completed task's goal/criteria requires an explicit reopen transition.
- **Accept:** Create two tasks with different goals/criteria/forms/drafts, switch, restart, and resume each without cross-contamination. A new project starts empty. Pending edits are resolved before switching; unmet criteria prevent completion and changed criteria reopen a completed task explicitly. Once R01/A02 ship, exercise switching with an active run/pending approval: remain on the original task until cancellation settles, invalidate old grants, and retain its partial journal. Q01 verifies the composed path.

### E03 — Task lifecycle, privacy, and bounded local storage

- [ ] Archive or remove tasks without accidental data loss or unbounded evidence storage.
- **Depends on:** E02.
- **Change:** Separate reversible archive/restore from delete/purge. Preview affected task records and currently owned artifacts; use neutral initial focus and explicit confirmation. Redact identifiable secrets before persistence and disclose limits; implement configurable local quotas/retention for existing task data without silently evicting active evidence. As those features ship, C02 applies the policy to excerpts, R01 reserves bounded spool capacity before execution and rejects deletion during active work, R02 represents purged/truncated output explicitly, and S02 preserves approved shared memories. Do not pre-create unused spool or memory modules in E03.
- **Accept:** Archive, restore, cancel deletion, confirm deletion, and restart; only the selected task's eligible local data disappears. Exercise disk-full/quota behavior against real stored task data, preserving drafts and responsive navigation; test identifiable-secret redaction before storage. C02/R01/R02/S02 verify their own retention integration when implemented, with Q01 covering the composition. Explain that local purge neither guarantees forensic secure erasure of SQLite/WAL/backups nor deletes provider sessions, exported files, or repository changes.

### C01 — Repository baseline and working diff

- [ ] Distinguish existing work from session changes.
- **Depends on:** E02.
- **Change:** Add repository discovery, initial tracked/untracked file identities, and a read-only working diff in a repo worker. Preserve dirty user files. Label external or ambiguous edits as unattributed; handle a non-Git directory as an explicit unsupported repository state without losing its task.
- **Accept:** Open a disposable dirty Git repository containing tracked/untracked edits; take the baseline, edit externally, and inspect the result. Existing user changes remain unchanged, new modifications are visible without false authorship, and the UI stays responsive while loading.

### C02 — Vaporeon source map and context basket

- [ ] Assemble a small, inspectable request context.
- **Depends on:** C01, E03.
- **Change:** Browse/search real files, inspect ranges, and pin/remove source excerpts with path/range/source identity. Respect ignore rules, explicit exclusions, binary/oversized-file limits, and symlink boundaries. Preview Eevee-sent content/bytes; label token estimates and provider-added context separately. Do not claim this basket is the full effective provider prompt or that agent-owned reads are constrained by an excerpt selection.
- **Accept:** Build a context basket in a fixture repository with CJK paths, a binary file, excluded credentials, and an out-of-root symlink. Inspect exact ranges, remove one, and modify another; show the stale reference. Excluded/out-of-scope content is absent from Eevee's stored basket/outgoing payload; disclose agent-added history/config/reads rather than asserting unverified global isolation.
- **Dispatch boundary:** Revalidate selected source identities and resolved containment after preview, including symlink/junction replacement. Changed content/scope invalidates the preview and requires review again; never silently send substituted bytes. A02 consumes this contract. Do not claim this controls independent agent-owned reads.
- **Retention integration:** Store excerpts under E03's quota/redaction/purge policy. Purge one task's excerpt without deleting the source file or another task's reference; inspect explicit unavailable evidence and refuse assembly from a missing artifact.

**Gate B:** create -> exit -> resume -> inspect sources works without a model, with explicit archive/purge and storage limits. This completes Phase 2 only when E01, E02, E03, C01, and C02 pass; later run/memory integration checks remain assigned to R02/S02/Q01.

## C — Real execution with visible evidence

### R01 — Jolteon approve, run, and cancel

- [ ] Execute one explicit command safely from the TUI.
- **Depends on:** C01, E03, UX01.
- **Change:** Add exact argv or explicit shell executable/dialect/text, cwd, scope, duration, and enforcement-source details. Direct argv must not silently invoke a shell; `.cmd`/`.bat` and shell metacharacters need an explicitly previewed shell route. Queue approvals with neutral initial focus and correlated IDs; enforce approval at dispatch and reject stale/double answers. Stream sanitized output through bounded buffers/quota-controlled spools under E03; record start/end, exit/signal, and before/after source identities. Own the process tree before exposing execution: POSIX process groups or Windows Job Objects/equivalent verified containment. Refuse a safe-cancel execution route when ownership cannot be established. Keep provider-observed runs distinct from Eevee-launched commands; never duplicate execution to manufacture evidence.
- **Accept:** Approve successful/failing commands, deny and prove no side effect, cancel a child-spawning command, and emit escape sequences/large output. Verify argv with spaces/CJK/metacharacters is delivered literally and explicit shell text uses the displayed dialect. Open an approval while typing, dismiss/reopen it, change its inputs, and attempt a stale/double answer; no focus theft or accidental execution. No surviving owned child, uncontrolled terminal operation, unbounded transcript, or invented success. Test quota exhaustion and retained partial output. Navigation cannot abandon active work; P02 repeats native lifecycle acceptance across all targets.
- **Retention integration:** Reserve bounded spool capacity before dispatch. At quota exhaustion, continue draining into a bounded discard path with a visible truncation boundary, or explicitly cancel under the owned-tree contract; never deadlock a child on undrained output or claim complete evidence. Refuse deleting/purging a task while it owns active work, and prove this refusal has no side effect.

### R02 — Execution journal and evidence freshness

- [ ] Make past results useful without overstating their validity.
- **Depends on:** R01, E02.
- **Change:** Persist executions and output references under E03's retention/quota policy; add Jolteon history and a reusable evidence inspector. Distinguish current, stale, ambiguous, failed, interrupted, truncated, and unavailable evidence. Missing/purged output cannot substantiate a verified result. On restart do not claim a previously running process is still owned/running; reconcile it as interrupted unless ownership is actually re-established.
- **Accept:** Run a check, restart and inspect its captured output, then change an input file and see stale status. Change a file during a run and see ambiguous status. Terminate the app during work and reopen without a false success or stuck running indicator. Purge retained output and exercise quota truncation: keep honest metadata, expose unavailable/partial evidence, and preserve artifacts still referenced by another task.

### UX02 — Activity, interruption, and recovery without lost context

- [ ] Keep long-running work inspectable and recoverable.
- **Depends on:** R02, UX01.
- **Change:** Add stable scroll anchors, search, explicit follow-output mode, a new-output indicator, and layer-specific error details. Retain drafts and selected artifacts during pending input, cancellation, connection loss, and save failure. Show cancelling/outcome-unknown states without automatically retrying a potentially mutating action.
- **Accept:** Scroll/search while a real command streams, open/close its pending request view, cancel it, and revisit partial output without jumping to the bottom or losing the draft. Exercise interrupted startup/resume and a failed save; the correct layer/remedy is visible, quit does not silently discard data, and no old command re-executes.
- **Exit contract:** Quit/Ctrl+C during work opens Stay or Cancel-and-exit with neutral focus; another keypress cannot silently force termination. Wait for owned-tree exit or authoritative provider cancellation, with bounded escalation and visible partial results. If external outcome cannot be established, record outcome unknown and require an explicit informed exit. Never imply cancellation rolled back edits. Exercise this together with unsaved/failed-save drafts, nested children, a hung cancellation, and terminal restoration.

### P02 — Native filesystem and process parity

- [ ] Prove local workbench behavior on every target platform, not just compilation.
- **Depends on:** P01, C02, R02, UX02.
- **Change:** Exercise platform user-data paths, repository identity/containment, executable discovery, shell selection, and owned-process-tree cancellation on the P01 matrix. Respect actual filesystem case sensitivity, symlinks/junctions, Windows file locking, and atomic storage semantics without lossy path normalization. Reject unsupported path encodings explicitly rather than merging distinct project identities. Qualify each agent and graphics capability separately by OS/version/terminal.
- **Accept:** On native target hosts, create/save/reopen tasks under space/CJK paths; inspect CRLF sources, distinct/case-aliased project paths, and links escaping the root. Test Windows locked files and POSIX permissions without loss of drafts. Verify executable paths with spaces, literal argv, explicit shell dialect, deny/no side effect, child/grandchild cancellation, quit under streaming, and normal/error restoration. Record host versions and results; do not replace native Windows execution with WSL or assume a provider supported on macOS exists elsewhere.

## D — Connect a real specialist, not nine chat sessions

### A01 — Agent preflight and truthful connection state

- [ ] Show real capabilities and prerequisites for each installed or configured agent.
- **Depends on:** UX02, C02.
- **Change:** Introduce a small versioned agent-profile/capability contract and connection drawer for Codex, Claude Agent SDK, and ACP profiles. Inspect executable/protocol versions, effective configuration/trust scope, authentication/billing source labels, and adapter availability without a model request. Establish executable/config trust before probing: even `--version` executes code, so a repository-supplied binary/hook requires deliberate consent. Qualify readiness per OS, executable version, and action; installed is not authenticated or supported. Missing/unknown is not supported. Show per-action Eevee-mediated/agent-policy/observed boundaries; never mutate global agent configs or silently load an untrusted project hook/plugin.
- **Accept:** Probe the installed Codex interface and missing executables/SDKs; preserve local-only use. A version mismatch, absent auth, or unknown enforcement capability disables the affected run with an actionable reason. No repository-supplied command or auth flow starts without trust/action; no credentials appear in profile snapshots or logs. Record observed versus merely documented capabilities.

### A02 — Espeon scoped planning request

- [ ] Complete the first real model round trip through the Codex app-server adapter.
- **Depends on:** A01, E02, C02.
- **Change:** Use the pinned installed-version app-server contract, not ANSI screen scraping, for normalized structured events, approval/questions, cancellation, and explicit thread lifecycle. Let Espeon submit a previewed context manifest and save a user-selected decision. Correlate task/run/generation/tool/request IDs, preserve provider-origin metadata, and do not count delta plus final snapshots twice. Keep one active request; a draft is not a queued submission.
- **Accept:** With real credentials, run a bounded design request, answer a real supported input request, deny/allow a supported operation, cancel a turn, and resume the explicit thread when supported. Reject a stale approval after cancellation, and surface malformed/fatal transport outcomes without fake completion or automatic resend. Inspect outgoing context plus provider-added-context disclosure. Auth/schema availability is a real blocker; mocks do not satisfy this gate.
- **Preview integrity:** Immediately before dispatch revalidate the C02 context manifest, task revision, profile/config, and grants. A changed file or replaced symlink/junction requires a new preview, not an unnoticed payload update. Test a source change between Preview and Run and prove no stale or substituted request is sent.

### A03 — Flareon bounded implementation

- [ ] Turn an approved requirement into an inspectable real patch.
- **Depends on:** A02, R02.
- **Change:** Request a change within the profile's verified edit boundary. Show exact enforcement provenance rather than assuming callback approval implies path/network isolation. Keep baseline/user changes visible, inspect the working diff, and offer deliberate Jolteon verification. Provider-native tool results are journaled with origin/coverage, not mislabelled as local execution. Never auto-commit/merge or silently widen scope.
- **Accept:** Make a real change in a disposable dirty repository, preserve user edits, review against the requirement, and run a chosen check. Deny a protected write and verify no side effect through the advertised route. If the profile cannot enforce a required boundary, refuse that controlled action or require an explicitly labelled external-policy workflow; no reassuring but false read-only label.

### A04 — Claude Agent SDK interoperability

- [ ] Run the same task UX through a supported Claude SDK adapter.
- **Depends on:** A03.
- **Change:** Add an optional, pinned TypeScript Agent SDK sidecar behind the Rust adapter contract. Use documented query/callback APIs rather than recreating its full private transport. Separate canUseTool approval, AskUserQuestion, and MCP elicitation; apply mandatory policy through actual tool/config controls, not only a callback bypassed by preapproved tools. Use the SDK's permitted auth/branding; no bundled claude.ai subscription login or undocumented credential reuse.
- **Accept:** In a disposable workspace with permitted credentials, preview/run, answer a question, deny/allow once, interrupt, and resume by explicit provider session ID. No queued follow-up starts after cancellation; no persistent rule is silently saved; cumulative usage is not double-counted. Verify project-hook/MCP configuration boundaries and show unsupported confinement honestly. The Rust UI/local-only path works without Node/SDK installed.

### A05 — ACP client with OpenCode as the first concrete peer

- [ ] Reuse a standard agent control protocol without assuming a standard sandbox.
- **Depends on:** A03.
- **Change:** Implement negotiated ACP v1 stdio JSON-RPC against OpenCode's documented `opencode acp` entrypoint. Advertise only implemented client callbacks; normalize prompt/update/permission/question/cancel/load semantics. Treat absent capabilities as unsupported, preserve provider option IDs, and gate every client filesystem/terminal request. Keep agent-owned tools/config effects separate from client mediation; ACP transport alone is not confinement.
- **Accept:** With a real compatible OpenCode installation, connect, run, answer supported permission/question requests, cancel with pending requests, and reload the correct cwd/session. Test unknown optional fields, unsupported version/capability, a late response, and a provider-owned tool path. No shared/global server assumptions, duplicate replayed artifacts, silent always-allow mapping, or stdout log contamination.

### A06 — Gemini ACP conformance and capability differences

- [ ] Prove the ACP implementation is not an OpenCode-specific adapter in disguise.
- **Depends on:** A05.
- **Change:** Add a version-qualified Gemini CLI profile using its documented ACP launch mode after checking the installed release. Reuse the ACP adapter; keep model/auth/session/question differences in capability/profile data, not UI-specific forks. Account for current account/product eligibility rather than promising a free subscription route.
- **Accept:** Run the same prompt/cancel/permission/context scenarios against real Gemini ACP. Exercise a capability present in one peer but absent in the other: the UI gives a correct reason and does not invoke the missing method. Record peer version, negotiated protocol, auth route, and actual restrictions. Missing executable/credentials block this task, not the local shell.

### A07 — Cross-agent handoff and honest external fallback

- [ ] Continue one Eevee task across agents without claiming portable sessions.
- **Depends on:** A04, A05, A06.
- **Change:** Preview switching agent/role policy at an idle boundary. Export/import a versioned task handoff containing approved facts, source references/excerpts, criteria, decisions, and unresolved work; never carry opaque provider session IDs, permissions, hidden reasoning, or credentials across agents. Keep imported results unattributed/unverified until checked. Provide a manual JSON/Markdown handoff for other CLIs instead of scraping or automating their TUI.
- **Accept:** Continue a Codex task with Claude Agent and an ACP peer using an explicitly accepted handoff; the old session remains available, grants reset, and the new request includes only reviewed context. Reject stale pending approvals and unknown/newer handoff schema. Manual export/import works without launching a CLI and cannot create verified execution evidence from user-supplied claims.

## E — Distinct diagnosis, refinement, and review

### U01 — Umbreon hypotheses and diagnosis

- [ ] Track explanations against real observations.
- **Depends on:** A02, R02.
- **Change:** Add a hypothesis board with supporting/refuting evidence links and explicit unresolved states. User-requested diagnostic commands use Jolteon's local approval/runner; provider-native diagnostics retain their actual origin and control coverage in the journal. A diagnosis alone does not edit source. Security findings use the same evidence discipline, not severity by guesswork.
- **Accept:** Investigate a seeded failure, reject one hypothesis using an actual run, and support a root cause with source/output references. Missing proof stays unresolved. Handoff to Flareon carries the diagnosis without re-sending the whole transcript.

### L01 — Leafeon bounded refactor and comparison

- [ ] Make improvement measurable and behavior-preserving.
- **Depends on:** A03, R02.
- **Change:** Select a hotspot, record the behavior contract and baseline, request a bounded refactor, and compare relevant validation/measurements. Reuse Flareon's patch and Jolteon's execution machinery, not a second editor or executor.
- **Accept:** Refactor a disposable example with a fixed behavioral contract; compare baseline/post-change output and relevant measurements. A failing check blocks a success claim. Do not claim a speedup from noise, missing baselines, or unrelated checks.

### G01 — Glaceon independent review desk

- [ ] Judge the patch against current evidence without editing it.
- **Depends on:** A03, R02.
- **Change:** Build a separate review context from requirements, patch, and selected observations. Add evidence-linked findings and accept/request-changes/inconclusive verdicts; show stale verification next to the affected criterion. Source-writing tools are unavailable to managed review requests.
- **Accept:** Review a seeded defect and an unverified change. Capture an actionable finding/inconclusive verdict, not a generic pass. Edit after a successful check and confirm the review shows stale evidence. Applying a fix requires a deliberate handoff, not Glaceon editing its own finding.

## F — Handoff, approved habits, and recipes

### S01 — Sylveon evidence-linked handoff

- [ ] Leave the next session a usable journal, not a vague summary.
- **Depends on:** G01, E02, A07.
- **Change:** Reuse A07's portable handoff schema to preview the goal, observed changes/results, decisions, unresolved risks, and exact next action; do not create a second export format. Include source/run references, distinguish model inferences, and preview any documentation write separately. No automatic declaration of completion.
- **Accept:** Close/reopen a task with one unresolved finding and resume from the recorded next action. Follow each result link to real evidence. Reject a proposed documentation edit and confirm the file is unchanged.

### S02 — Explicitly curated project and personal habits

- [ ] Let the companion learn only what the user approves.
- **Depends on:** S01, C02.
- **Change:** Offer Remember on a selected verified/user-authored note; preview task/project/personal scope, then approve or reject. Support inspection/edit/delete and honor exclusions. Do not silently infer a permanent habit from one action or add a numerical bond score.
- **Accept:** Approve one project memory and reject another; restart and inspect exact contents. Open a different project and confirm scope isolation, then delete the memory and verify it no longer enters context. Excluded secrets are never proposed from captured content.
- **Retention integration:** Use E03's ownership/purge rules; deleting one task must not remove approved shared memory, and deleting a memory must remove it from future request assembly while explaining that prior exports/provider history are outside local erasure.

### S03 — Evolution Stone recipe preview

- [ ] Reuse workflows without creating a permission bypass.
- **Depends on:** V03, R02, S01.
- **Change:** Load versioned TOML recipes describing form steps, context/scope, validation commands, and completion criteria. Show the recipe before applying it to a task. No install hooks, marketplace, automatic execution, or authority to expand existing grants.
- **Accept:** Preview a local bug-fix recipe, apply its plan without executing commands, and require normal approvals during use. Reject an invalid schema. A recipe requesting an ungranted write/run capability cannot perform that action.

## G — Integrated acceptance

### Q01 — Complete journey and visual state matrix

- [ ] Prove all nine workflows and their composed user experience.
- **Depends on:** V01, V02A, V02B, V03, UX01, UX02, E02, E03, C02, R02, P02, A03, A04, A05, A06, A07, U01, L01, G01, S01, S02, S03.
- **Change:** Exercise real nine-form artifacts plus the shared UX across Codex, Claude Agent SDK, and the two ACP profiles. Run the compatibility conformance scenarios, not a mandatory all-agent tour for normal users. Maintain separate native-local, graphics-terminal, and provider OS/version evidence matrices; an unavailable optional provider blocks its interoperability gate, not use or independent verification of the local core. Q01 remains pending until every required gate passes. Fix actual integration/visual defects; no fake backend or single-agent-only test qualifies as interoperability.
- **Accept:** Cover empty/missing dependency, ready/running, permission/question, cancellation/unknown outcome, save failure, stale evidence, and cross-agent handoff across supported sizes/themes and long CJK content. Verify keyboard focus, no accidental execution, bounded streams/storage, task switch and quit with pending work/drafts, resumed usage accounting, purge/missing artifacts, image cleanup, and terminal restoration. Exercise E02/R02/S02's deferred integration checks. Every claimed controlled action names and proves its enforcement boundary. Review naming/art rights and provider terms before distribution; unsupported OS/provider combinations remain explicitly unavailable rather than implied portable.

### Q02 — Measured native and token efficiency

- [ ] Measure the performance contract rather than advertise unverified speed.
- **Depends on:** Q01.
- **Change:** Benchmark the release binary on recorded hardware/workload: 60s local-shell idle CPU/RSS, 30 warm local startups, key-to-render latency under streaming, 10 MiB output, and a 10,000-line diff. Report core, optional SDK sidecar, and external-agent resource use separately and in aggregate. Inspect per-provider usage/cost scope and outgoing context; optimize measured bottlenecks.
- **Accept:** Report observations against the design targets with method/sample size and remaining misses, without hiding helper/agent costs inside a core-only claim. Queues stay bounded; no model request on form selection, no unsubmitted draft leaks, and no unrelated/full history in Eevee-sent context. Show unknown provider-added context/usage honestly; no savings percentage or merged cost total without compatible measured scopes.

### P03 — Native distribution, upgrade, and support evidence

- [ ] Make installation and upgrades reproducible on the supported platform matrix.
- **Depends on:** P02, Q01, Q02.
- **Change:** Produce versioned native archives and checksums for the P01 targets, with install/run/uninstall instructions and recorded OS/toolchain/terminal baselines. Separate verified core support from graphics and optional provider compatibility. Include no mandatory Node runtime, credentials, or franchise artwork in the core package. Keep user-data paths stable across upgrades and refuse incompatible schema downgrade without changing data. Do not advertise unverified packages or bypass rights/provider-term review.
- **Accept:** Download/unpack and run each package on a clean native target without Rust installed; verify checksums, help/version, the interactive local flow, and absence of account/art/network prerequisites. Upgrade from the previous supported data schema (fixture for the first release), resume a task, and attempt an incompatible downgrade without data loss. Uninstall leaves data intact unless the user deliberately invokes documented purge. Publish per-target build/runtime/install evidence plus graphics/provider limitations; unsigned/unnotarized artifacts and any OS trust prompt must be disclosed, never bypassed through a blanket security-disable instruction.
- **User-directed preview slice:** The current request selected `ev`, Apache-2.0, and verification packages/offline installers only. P01's five native runners now build checksum-qualified archives with dependency notices and install/upgrade/failure-preservation checks. Public Release/remote bootstrap, durable-schema upgrades, full parity/performance, signing/notarization, and rights/provider review remain gated; this does not complete P03 or silently waive P02/Q01/Q02.

## Phase status

Phase 1 and all six section-A tasks V01, P01, V02A, V02B, V03, and UX01 are complete on their recorded environments. Sections B-G and platform parity/distribution remain pending. Of 31 implementation tasks, 6 are checked and 25 remain unchecked. See `docs/WORK_CHECKPOINT.md` for exact evidence and continuation; do not infer other native protocols, GUI terminals, older OS versions, persistence, or provider support from the companion UI.
