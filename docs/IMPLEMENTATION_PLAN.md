# Eevee Implementation Plan

Date: 2026-10-12
Status: Phase 1 implemented and verified. Next: V01 visual refinement, then actual-artwork tasks V02A/V02B, V03, UX01, and Phase 2 local-workbench increments. All 27 planned implementation tasks remain pending.
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

The actionable backlog and per-task demos live in [`task.md`](../task.md); do not duplicate its completion checkboxes here. The 2026-10-12 refinements prioritize actual Eevee/evolution images, visual clarity, and predictable local input before storage, then interruption/recovery UX before agents. They do not reopen or falsely extend Phase 1's completion. The artwork clarification supersedes hand-drawn substitutes; multi-agent interoperability now has concrete, separately gated implementation tasks.

| Gate | Backlog tasks | User-visible outcome |
| --- | --- | --- |
| Visual/interaction refinement | V01, V02A, V02B, V03, UX01 | Sanctuary, actual artwork, native/cell rendering, deliberate selection, accessible focus/editing |
| Phase 2 | E01, E02, C01, C02 | Resume a real task and assemble source context without a model |
| Phase 3 | R01, R02, UX02 | Approve/run/cancel, inspect durable evidence, browse streams, recover without lost drafts |
| Phase 4 | A01, A02, A03, A04, A05, A06, A07 | Codex first; Claude Agent SDK; ACP with OpenCode/Gemini; reviewed cross-agent handoff |
| Phase 5 | U01, L01, G01 | Evidence-led diagnosis, behavior-preserving refinement, independent review |
| Phase 6 | S01, S02, S03 | Verifiable handoff, approved habits, untrusted-recipe preview |
| Phase 7 | Q01, Q02 | Complete real workflows, visual state coverage, observed performance |

Default execution follows backlog order. Explicit dependencies allow later parallel work only when interfaces and file ownership are clear; do not parallelize merely because forms have different names. Each increment must leave a usable, honest application. Backend calls and durable-state promises remain unavailable until their own acceptance gates pass.

Next five increments:

1. V01 changes presentation only: centralize style tokens, reduce duplicate status text, create a single task/action card, and verify each supported layout in the real terminal.
2. V02A replaces hand-drawn symbols with source-tracked actual species images and an offline cell renderer; no implicit download, asset redistribution claim, or native-image claim.
3. V02B adds native image protocols through `ratatui-image`, with actual-terminal validation, bounded discovery, caching, cleanup, and compatibility fallbacks.
4. V03 turns form selection into an image grid/list with a role/artifact/prerequisite preview. It remains a local action, not a provider request or permission grant.
5. UX01 closes the local interaction gate: stable focus/selection across resize and overlays, safe editing/paste, and a linear text view. It does not introduce agent execution; E01 remains the first storage task.


## Phase 1 — Native shell and visual foundation

1. Initialize the Rust binary and pin dependency versions supported by the chosen toolchain.
2. Implement terminal enter/restore guards for normal exit, errors, and panic; establish reducer and event loop.
3. Implement all nine form identities, palette tokens, responsive shell, keyboard navigation, and form picker.
4. Populate views only from real local task/repository state or honest empty states; no fabricated agent activity.

Acceptance: run the actual TUI at 120x36, 100x30, 70x24, and below the minimum; switch all forms; exercise help, drawers, input focus, cancel, and quit. Capture terminal screenshots and verify terminal restoration. Check light/dark, monochrome, ASCII, and CJK content. This phase alone is not the finished product.

## Phase 2 — Durable task state and repository evidence

1. Add SQLite schema/migrations, task creation, acceptance criteria, events, and recovery.
2. Capture pre-existing changes and source identities without altering user files.
3. Add source references, context selection, diff view, provenance labels, and verification freshness.
4. Implement Eevee and Vaporeon's local interactions against real repository data.

Acceptance: create a task in a throwaway Git repository with dirty tracked and untracked files; quit/reopen and recover it. Edit a file externally and confirm attribution remains unknown and affected verification becomes stale. Tests cover transitions, recovery, and identity boundaries, not copied UI text.

## Phase 3 — Controlled execution and Jolteon

1. Implement correlated approval records, neutral initial focus, and dispatch-time capability enforcement.
2. Add direct-argv execution, explicit shell mode, cwd, process-group cancellation, bounded output spooling, and terminal escape sanitization.
3. Associate runs with before/after code state; record origin, failures, partial output, and ambiguous coverage honestly.
4. Complete Jolteon's history/live-run workspace, follow-output/search behavior, focus-preserving pending requests, and interruption/save-failure recovery.

Acceptance: execute real successful, failing, long-running, high-output, and child-spawning commands in a throwaway directory. Deny an approval and prove no process started. Cancel and confirm children terminate. Inject terminal control sequences and verify they cannot alter the host terminal. Exercise a file change during a run and show ambiguous evidence.

## Phase 4 — Capability-aware agent interoperability

1. A01: expose real dependency/version/configuration/auth/control preflight without a model request; unsupported adapters remain unavailable, not simulated.
2. A02/A03: implement the version-qualified Codex app-server request/approval/event lifecycle, scoped planning, bounded edits, and deliberate verification.
3. A04: integrate the official Claude Agent SDK with a small optional bridge; prove real approval/input/cancel/resume semantics and permitted authentication.
4. A05/A06: implement ACP once, then prove it against OpenCode and Gemini with different negotiated capabilities. Do not invent universal sandbox or session semantics.
5. A07: continue a task through a reviewed fresh cross-agent handoff; retain explicit same-agent resume and a manual fallback for other CLIs.

Acceptance: every advertised adapter/version passes the [shared conformance gate](AGENT_COMPATIBILITY.md#shared-conformance-gate) with real permitted credentials and a disposable dirty repository. Include missing/incompatible dependencies, configuration trust, deny/no side effect, supported questions, pending-request cancellation, malformed/late/replayed events, abrupt disconnect, exact-session resume, handoff, and scoped usage accounting. A provider success is not task completion. Missing executables/auth or unverified confinement block the affected gate; fixtures and docs do not establish runtime compatibility.

## Phase 5 — Diagnosis, refinement, and independent review

1. Implement Umbreon's hypothesis/evidence workflow and diagnostic handoff to Jolteon.
2. Implement Leafeon's bounded refactor requests and before/after comparison using real runs.
3. Implement Glaceon's separately scoped review request, findings, acceptance checklist, and stale-evidence warnings.
4. Link findings back to source ranges and executions; represent missing evidence as inconclusive.

Acceptance: diagnose a deliberately seeded bug, repair it through Flareon, verify through Jolteon, and review through Glaceon. Perform a behavior-preserving Leafeon refactor and verify it. Make an edit after verification and ensure review no longer presents that run as current. Keep regression tests for consumer-visible failures found during implementation.

## Phase 6 — Handoff, memory, and Evolution Stones

1. Implement Sylveon's evidence-linked handoff and previewable documentation changes.
2. Add task/project/personal memory scopes with explicit promotion, edit, delete, and exclusion controls.
3. Load and validate versioned TOML recipes; preview workflow, scope, and commands before applying.
4. Ensure recipes cannot expand permissions; do not add a marketplace or execute installation hooks.

Acceptance: end and resume a task using its handoff; approve and reject memory proposals and verify their exact scopes after restart. Import a recipe requesting unauthorized execution and confirm it cannot run. Excluded content must not enter persisted context artifacts.

## Phase 7 — End-to-end visual and performance acceptance

1. Exercise all nine forms against real task evidence; separately run every advertised agent profile through the shared conformance matrix.
2. Inspect actual terminal rendering across supported sizes/themes and linear mode: long paths, CJK, multiline input, permission/questions, unknown outcomes, save failure, stale evidence, stream scrolling, and image cleanup.
3. Measure release startup, idle CPU/RSS, and key-to-render latency under the design workloads. Report native core, optional SDK bridge, external agents, and aggregate resources with scope; no hidden helper cost or incompatible usage totals.
4. Fix measured bottlenecks and broken contracts; then run formatting, linting, and the relevant test suite once after the integrated changes.
5. Update usage documentation, changelog, troubleshooting entries for resolved non-trivial issues, and checkpoint. Remove throwaway fixtures and scripts.

Acceptance: publish observed measurements and real run results, not targets as achievements. All nine roles, persistence, cancellation, action-specific controls, context boundaries, evidence freshness, agent handoff, and terminal restoration must work. Include a real assistive-terminal check before claiming that environment accessible. No unimplemented forms, fictitious compatibility badges, or fake fallbacks may be presented as complete.

## Explicit exclusions

No custom model runtime, autonomous multi-agent swarm, hosted synchronization, plugin marketplace, automatic commits/merges, agent-TUI scraping, or background model calls for pet animation. No new MCP server or generic plugin framework in this plan. No claim of canonical game evolution mechanics or affiliation. Public distribution requires naming/art review plus provider authentication/branding/terms review.

## Current next action

Implement **V01 — Sanctuary layout and visual hierarchy** from `task.md`. Start from the existing `src/ui.rs` and actual Phase 1 terminal capture, reserve a bounded real-artwork area, refine spacing/hierarchy/empty states without inventing tasks or introducing storage, and prove the result in the real TUI across widths and color modes. V02A/V02B/V03/UX01 follow; E01 remains the first storage increment. Planning documents, downloaded preview images, SVGs, and CLI help observations do not implement these tasks or prove interoperability.
