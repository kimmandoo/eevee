# Eevee Implementation Plan

Date: 2026-10-12
Status: Phase 1 implementation and verification are last recorded evidence, not verification of the current working tree: user-owned source/Cargo edits have not been reverified. Next: V01 visual refinement, then P01 native portability and ready artwork/input/local-workbench increments. All 31 planned implementation tasks remain pending.
Contract: [design rules](DESIGN_RULES.md) and [agent compatibility](AGENT_COMPATIBILITY.md). The latter distinguishes documented interfaces, local CLI observations, and runtime conformance still to be performed.

## Outcome

Build a Rust terminal workbench that preserves one task across Eevee and all eight evolutions. All nine forms must have functional role-specific surfaces and workflows, not cosmetic tabs. Prioritize observable evidence, safe execution, minimal model context, and a polished keyboard-first interface.

## Architecture

One native Rust binary and one Cargo package with these module boundaries; an optional provider SDK bridge stays outside the UI/core:

- `app`: reducer, typed events, task/form transitions, capability checks, independent task/request/storage/evidence states.
- `domain`: task, form, evidence, execution, finding, correlated approval/question, memory, recipe, agent profile, handoff.
- `ui`: responsive and linear layouts, theme tokens, nine workspaces, focus/scroll restoration, request previews, palette, overlays.
- `runtime`: bounded event channels, cancellable subprocesses, output sanitization/spooling.
- `store`: SQLite migrations, durable events/snapshots, output references, recovery.
- `repo`: Git baseline, working-tree identities, diff, external-change detection.
- `agent`: profile/version admission, request/context assembly, normalized events, and concrete Codex/Claude/ACP adapters.

The reducer owns mutable application state. Workers send typed events; render functions read state and do no I/O. Store writes run on a dedicated blocking worker. Large payloads are referenced rather than repeatedly cloned into events.

Connect Codex first through a release-pinned local app-server child. Add the official Claude Agent SDK through an optional TypeScript sidecar, then a shared ACP v1 client exercised against both OpenCode and Gemini. The SDK bridge is a deliberate compatibility exception, not a rewrite or mandatory runtime for local work. Unsupported CLIs use a reviewed manual handoff, not terminal scraping. See [agent compatibility](AGENT_COMPATIBILITY.md) for primary sources, exact protocol distinctions, auth/terms constraints, and acceptance scenarios.

Forms define work and expected evidence; agent profiles define execution and actual control boundaries. A profile is not a model, and a provider mode is not an evolution. Choose one task agent by default; a deliberate role override still requires capability admission. Provider-native commands are observed with origin/coverage, not rerun through Jolteon. Never promise that all tools are intercepted, that a permission prompt is a sandbox, or that selected excerpts are the agent's complete effective context.

Each run snapshots the profile/schema/cwd/configuration/policy and connection generation. Preserve provider IDs and permission option scopes. Separate stdout protocol from stderr; bound queues/output and reconcile deltas, final snapshots, and replay. Keep one foreground mutable run; local drafts are not queued provider messages. Cancellation requires an authoritative terminal outcome or an explicit unknown state. Resume never silently resends a mutating request.

Same-agent resume validates its native session and current policy. Cross-agent continuation creates a fresh session from a reviewed portable handoff; grants, credentials, hidden reasoning, and opaque provider identities do not migrate. Authentication remains provider-owned, billing source is explicit, and project hooks/plugins/MCP configuration require a trust preview before launch. Provider-owned persistent config effects require separate consent, including Codex 0.147.0's documented workspace-write trust update.

## Form workflows

| Form | Concrete interaction | Persisted result |
| --- | --- | --- |
| Eevee | Create/resume task, edit acceptance criteria, choose a form | Task and routing decision |
| Vaporeon | Search, inspect, select source ranges, assemble request context | Source-linked context manifest |
| Jolteon | Inspect approval, run local command, observe provider runs, cancel, revisit result | Execution evidence with origin and known coverage |
| Flareon | Send bounded change request, inspect returned edits, request validation | Change set and rationale |
| Espeon | Compare approaches, choose one, order dependent steps | Decision and executable task plan |
| Umbreon | Record hypothesis, request diagnostic command, attach supporting/refuting evidence | Diagnosis or unresolved hypotheses |
| Leafeon | Select bounded refactor, compare baseline/post-change behavior or measurements | Refactor change set and comparison |
| Glaceon | Review patch against criteria, inspect freshness, record findings | Review verdict and unresolved risks |
| Sylveon | Prepare handoff, preview documentation edits, approve/reject memory proposals | Handoff, docs change, curated memories |

A form may request a handoff but cannot activate another form or widen permissions without the user's action. Context selection is explicit and inspectable. Forms may use the same agent while receiving different scoped requests; Glaceon still starts from an independent review context. No workflow requires visiting every form or trying every provider.

## Incremental delivery order

The actionable backlog and per-task demos live in [`task.md`](../task.md); do not duplicate its completion checkboxes here. These refinements add native OS portability, actual Eevee/evolution images, predictable local input, durable state, and interruption/recovery before agents. They do not reopen or falsely extend Phase 1's recorded completion. Hand-drawn substitutes do not meet the artwork gate; native images and agent interoperability have separate OS/version/terminal-qualified gates.

| Gate | Backlog tasks | User-visible outcome |
| --- | --- | --- |
| Visual/interaction refinement | V01, V02A, V02B, V03, UX01 | Sanctuary, actual artwork, native/cell rendering, deliberate selection, accessible focus/editing |
| Native portability checkpoint | P01 | Native core build/shell evidence for each target OS/architecture/terminal tuple |
| Phase 2 | E01, E02, E03, C01, C02 | Durable tasks, safe switching and retention, source context without a model |
| Phase 3 | R01, R02, UX02, P02 | Approve/run/cancel, durable evidence, interruption recovery, native filesystem/process lifecycle |
| Phase 4 | A01, A02, A03, A04, A05, A06, A07 | Codex first; Claude Agent SDK; ACP with OpenCode/Gemini; reviewed cross-agent handoff |
| Phase 5 | U01, L01, G01 | Evidence-led diagnosis, behavior-preserving refinement, independent review |
| Phase 6 | S01, S02, S03 | Verifiable handoff, approved habits, untrusted-recipe preview |
| Phase 7 | Q01, Q02, P03 | Complete integrated workflows, visual/performance evidence, native distribution acceptance |

Choose the first ready task in backlog order. A blocked task stays pending but does not stop independent ready work; parallel work still requires clear interfaces and file ownership. UX01 depends on V01, not V03; V03 depends on V02A, not V02B; E01 depends on UX01 and follows the platform contract without waiting for access to every P01 runner. P02 requires completed P01 and validates storage across all targets. Missing native graphics hardware must not gate basic keyboard interaction, storage, or the cell-rendered picker. Prioritize ready UX01/E01 work when graphics is unavailable, while preserving actual artwork and native-image evidence for their full visual gates. Backend calls and durable-state promises remain unavailable until their own gates pass.

Immediate sequence and readiness:

1. V01 changes presentation only: centralize style tokens, reduce duplicate status text, create a single task/action card, and verify each supported layout in the real terminal.
2. P01 follows V01 and establishes native core build/shell portability for macOS arm64 and x86_64, Linux x86_64 and arm64, and Windows x86_64 using native Windows Terminal/ConPTY, not WSL. Windows ARM64 and other variants remain unclaimed. Record actual minimum OS, toolchain, and terminal per row rather than inventing versions; cross-compilation is not runtime proof. Require native CI build/test/clippy and real-terminal evidence per tuple. A missing host leaves its gate pending without blocking unrelated ready work.
3. V02A supplies source-tracked actual species images and an offline cell renderer; no implicit download or asset redistribution claim. V02B separately adds native image protocols through `ratatui-image`, with actual-terminal evidence, bounded discovery, caching, cleanup, and fallbacks.
4. V03 uses V02A's cell-capable image grid/list with a role/artifact/prerequisite preview; it need not wait for V02B. Selection is not a provider request or permission grant.
5. UX01 can proceed after V01: stable focus/selection across resize and overlays, safe editing/paste, and a linear text view. Once UX01 passes, E01 is the first storage increment; missing P01 hosts do not block local implementation or create a portability claim. Native images and providers each require their own capability records, not inherited OS support claims.


## Phase 1 — Native shell and visual foundation

1. Initialize the Rust binary and pin dependency versions supported by the chosen toolchain.
2. Implement terminal enter/restore guards for normal exit, errors, and panic; establish reducer and event loop.
3. Implement all nine form identities, palette tokens, responsive shell, keyboard navigation, and form picker.
4. Populate views only from real local task/repository state or honest empty states; no fabricated agent activity.

Acceptance: run the actual TUI at 120x36, 100x30, 70x24, and below the minimum; switch all forms; exercise help, drawers, input focus, cancel, and quit. Capture terminal screenshots and verify terminal restoration. Check light/dark, monochrome, ASCII, and CJK content. This phase alone is not the finished product.

## Phase 2 — Durable task state and repository evidence

1. E01: add versioned SQLite schema, transactional migrations, durable task/events, and recovery. Enforce an explicit single-writer conflict without silent takeover and monotonic acknowledged revisions.
2. E02: preserve per-task drafts; task switching cannot transfer an active run's ownership. Offer stay or cancel-and-wait before switching. Changing a completed task's goal/criteria requires an explicit reopen transition.
3. E03 follows and depends on E02: distinguish reversible archive from confirmed delete/purge, preview impact with neutral cancel focus, and redact identifiable secrets before persistence. Enforce configurable spool disk quotas/retention without silently evicting active evidence; missing artifacts remain explicit. Protect shared memory/profile references. Do not claim secure erase or deletion from providers.
4. C01/C02: capture pre-existing changes and source identities without altering user files; add source references, context selection, diff, provenance, and freshness. Revalidate selected context at dispatch after preview, including symlink/junction swaps; changed selections require a fresh preview.
5. Implement Eevee and Vaporeon's local interactions against real repository data.

Acceptance: create a task in a throwaway dirty repository, quit/reopen, and recover it. Cover old-schema upgrade/rollback, corrupt/newer databases without overwrite, and disk-full/locked database errors preserving drafts. Exercise writer conflicts, acknowledgment ordering, explicit reopening, archive/delete cancellation, stored-task/excerpt quotas, and missing artifacts. External file changes preserve unknown attribution and stale affected verification. Run/memory-dependent checks belong to R01/R02/S02 and composed Q01, not an impossible prerequisite for Phase 2: active-run switching, output retention, shared memories, and A02 dispatch-time context revalidation. Test transitions and identity boundaries, not copied UI text.

## Phase 3 — Controlled execution and Jolteon

1. Implement correlated approval records, neutral initial focus, and dispatch-time capability enforcement.
2. R01: add direct-argv execution without an implicit shell, explicit shell executable/dialect (including deliberate `.cmd`/`.bat` handling without silent wrappers), cwd, native owned process-tree cancellation, bounded output spooling, and terminal escape sanitization. Use owned POSIX process groups and Windows Job Objects or an equivalently verified ownership mechanism; reject safe-cancel actions when ownership is unavailable.
3. Associate runs with before/after code state; record origin, failures, partial output, and ambiguous coverage honestly. R02 inherits E03's redaction, retention, quota, and missing-evidence rules.
4. UX02: complete history/live-run navigation and interruption/save-failure recovery. During active work, quit/Ctrl+C offers stay or cancel-and-exit, waits for cancellation with bounded escalation, and records unknown outcomes rather than claiming rollback. A second key must not accidentally force termination.
5. P02 follows UX02 and depends on P01, C02, R02, and UX02. Prove OS filesystem/data-directory and native command lifecycle end to end: spaces/CJK, CRLF, case sensitivity, symlinks/junctions, Windows locked files, executable discovery, owned process-tree cancellation, and exit recovery.

Acceptance: execute real successful, failing, long-running, high-output, and child-spawning commands in a throwaway directory on each native target row. Denial starts no process; cancellation confirms owned children terminate or exposes an unknown outcome. Test explicit shell boundaries and unavailable ownership, interruption dialogs and terminal recovery. Terminal control sequences must not alter the host terminal. A file change during a run produces honestly ambiguous evidence; cancellation does not roll back side effects.

## Phase 4 — Capability-aware agent interoperability

1. A01: expose real dependency/version/configuration/auth/control preflight without a model request; do not execute unknown repository-supplied candidates before trust. Readiness is qualified by OS, version, and action; unsupported adapters remain unavailable, not simulated.
2. A02/A03: implement the version-qualified Codex app-server request/approval/event lifecycle, scoped planning, bounded edits, and deliberate verification. Admission repeats C02's dispatch-time context identity check, including symlink/junction changes and re-preview.
3. A04: integrate the official Claude Agent SDK with a small optional bridge; prove real approval/input/cancel/resume semantics and permitted authentication.
4. A05/A06: implement ACP once, then prove it against OpenCode and Gemini with different negotiated capabilities. Do not invent universal sandbox or session semantics.
5. A07: continue a task through a reviewed fresh cross-agent handoff; retain explicit same-agent resume and a manual fallback for other CLIs.

Acceptance: every advertised adapter/version/OS passes the [shared conformance gate](AGENT_COMPATIBILITY.md#shared-conformance-gate) with real permitted credentials and a disposable dirty repository. Include missing/incompatible dependencies, configuration trust, deny/no side effect, supported questions, pending-request cancellation, malformed/late/replayed events, abrupt disconnect, exact-session resume, handoff, and scoped usage accounting. A provider success is not task completion. Missing optional executables/auth or unverified confinement block the affected interoperability gate, not independently delivered local-core work; full integrated acceptance stays pending until every required row passes. Fixtures and docs do not establish runtime compatibility.

## Phase 5 — Diagnosis, refinement, and independent review

1. Implement Umbreon's hypothesis/evidence workflow and diagnostic handoff to Jolteon.
2. Implement Leafeon's bounded refactor requests and before/after comparison using real runs.
3. Implement Glaceon's separately scoped review request, findings, acceptance checklist, and stale-evidence warnings.
4. Link findings back to source ranges and executions; represent missing evidence as inconclusive.

Acceptance: diagnose a deliberately seeded bug, repair it through Flareon, verify through Jolteon, and review through Glaceon. Perform a behavior-preserving Leafeon refactor and verify it. Make an edit after verification and ensure review no longer presents that run as current. Keep regression tests for consumer-visible failures found during implementation.

## Phase 6 — Handoff, memory, and Evolution Stones

1. Implement Sylveon's evidence-linked handoff and previewable documentation changes.
2. Add task/project/personal memory scopes with explicit promotion, edit, delete, and exclusion controls. S02 inherits E03's pre-persistence redaction, retention/deletion previews, shared-reference protection, and limits on deletion claims.
3. Load and validate versioned TOML recipes; preview workflow, scope, and commands before applying.
4. Ensure recipes cannot expand permissions; do not add a marketplace or execute installation hooks.

Acceptance: end and resume a task using its handoff; approve and reject memory proposals and verify their exact scopes after restart. Import a recipe requesting unauthorized execution and confirm it cannot run. Excluded content must not enter persisted context artifacts.

## Phase 7 — End-to-end visual and performance acceptance

1. Q01 depends additionally on P02 and E03. Exercise all nine forms against real task evidence and report local-core, provider, and native-image acceptance separately. Missing optional provider access does not block delivered local-core work, but full Q01 stays pending until every required row passes.
2. Inspect actual terminal rendering across supported sizes/themes and linear mode: long paths, CJK, multiline input, permission/questions, unknown outcomes, save failure, stale evidence, stream scrolling, and image cleanup. Missing graphics hardware does not block basic input/storage/cell-picker work; it leaves the affected full visual gate pending.
3. Measure release startup, idle CPU/RSS, and key-to-render latency under the design workloads. Report native core, optional SDK bridge, external agents, and aggregate resources with scope; no hidden helper cost or incompatible usage totals.
4. Fix measured bottlenecks and broken contracts; then run formatting, linting, and the relevant test suite once after the integrated changes.
5. Update usage documentation, changelog, troubleshooting entries for resolved non-trivial issues, and checkpoint. Remove throwaway fixtures and scripts.

Acceptance: publish observed measurements and real run results, not targets as achievements. All nine roles, persistence, cancellation, action-specific controls, context boundaries, evidence freshness, agent handoff, and terminal restoration must work. Include a real assistive-terminal check before claiming that environment accessible. No unimplemented forms, fictitious compatibility badges, or fake fallbacks may be presented as complete.

P03 is the final distribution gate after P02, Q01, and Q02: produce versioned native archives and checksums; prove fresh install, upgrade, and uninstall per target row, preserving user data unless explicitly purged. Detect incompatible data schemas without destructive overwrite. Record build/run/install evidence separately from provider and graphics capabilities. Do not advertise unverified platform support or require Node or artwork for local-core installation. Release remains blocked until naming/artwork rights and provider authentication/branding/terms reviews pass.

## Explicit exclusions

No custom model runtime, autonomous multi-agent swarm, hosted synchronization, plugin marketplace, automatic commits/merges, agent-TUI scraping, or background model calls for pet animation. No new MCP server or generic plugin framework in this plan. No claim of canonical game evolution mechanics or affiliation. Public distribution requires naming/art review plus provider authentication/branding/terms review.

## Current next action

Implement **V01 — Sanctuary layout and visual hierarchy** from `task.md`. Start from the existing `src/ui.rs` and recorded Phase 1 terminal capture, reserve a bounded real-artwork area, refine spacing/hierarchy/empty states without inventing tasks or introducing storage, and prove the result in the real TUI across widths and color modes. P01 follows, then the first ready artwork/input tasks; prioritize UX01/E01 when native graphics is unavailable without dropping the full visual gate. E01 remains the first storage increment. Planning documents, downloaded preview images, SVGs, and CLI help observations do not implement these tasks or prove interoperability; historical Phase 1 evidence does not verify current source edits.
