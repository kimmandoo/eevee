# Eevee Implementation Plan

Date: 2026-10-12
Status: Phase 1 implemented and verified. Next: V01 visual refinement, then actual-artwork tasks V02A/V02B, V03, and the Phase 2 local-workbench increments. Later phases remain pending.
Contract: `docs/DESIGN_RULES.md`.

## Outcome

Build a Rust terminal workbench that preserves one task across Eevee and all eight evolutions. All nine forms must have functional role-specific surfaces and workflows, not cosmetic tabs. Prioritize observable evidence, safe execution, minimal model context, and a polished keyboard-first interface.

## Architecture

One binary, one Cargo package, these module boundaries:

- `app`: reducer, typed events, task/form transitions, capability checks.
- `domain`: task, form, evidence, execution, finding, approval, memory, recipe.
- `ui`: responsive layout, theme tokens, nine workspaces, palette, overlays.
- `runtime`: bounded event channels, cancellable subprocesses, output sanitization/spooling.
- `store`: SQLite migrations, durable events/snapshots, output references, recovery.
- `repo`: Git baseline, working-tree identities, diff, external-change detection.
- `agent`: one real provider adapter and scoped request assembly.

The reducer owns mutable application state. Workers send typed events; render functions read state and do no I/O. Store writes run on a dedicated blocking worker. Large payloads are referenced rather than repeatedly cloned into events.

Initial integration: Codex CLI as an external coding backend, subject to an installed-version capability check. Before implementing its adapter, inspect its actual structured-output, cancellation, approval, and resume interfaces and record the supported version. Do not invent CLI flags or parse human-oriented terminal prose. If permission mediation is unavailable, offer explicit unmanaged operation or stop that managed workflow; never claim enforcement that is not available. A missing executable gets an actionable prerequisite screen, never simulated agent output. Additional providers are out of initial scope.

## Form workflows

| Form | Concrete interaction | Persisted result |
| --- | --- | --- |
| Eevee | Create/resume task, edit acceptance criteria, choose a form | Task and routing decision |
| Vaporeon | Search, inspect, select source ranges, assemble request context | Source-linked context manifest |
| Jolteon | Inspect approval, run command, stream output, cancel, revisit result | Execution evidence |
| Flareon | Send bounded change request, inspect returned edits, request validation | Change set and rationale |
| Espeon | Compare approaches, choose one, order dependent steps | Decision and executable task plan |
| Umbreon | Record hypothesis, request diagnostic command, attach supporting/refuting evidence | Diagnosis or unresolved hypotheses |
| Leafeon | Select bounded refactor, compare baseline/post-change behavior or measurements | Refactor change set and comparison |
| Glaceon | Review patch against criteria, inspect freshness, record findings | Review verdict and unresolved risks |
| Sylveon | Prepare handoff, preview documentation edits, approve/reject memory proposals | Handoff, docs change, curated memories |

A form may request a handoff but cannot activate another form or widen permissions without the user's action. Context selection is explicit and inspectable. Forms may use the same provider while receiving different scoped requests.

## Incremental delivery order

The actionable backlog and per-task demos live in [`task.md`](../task.md); do not duplicate its completion checkboxes here. The 2026-10-12 refinement prioritizes actual Eevee/evolution images and visual clarity before storage. This adds V01, V02A, V02B, and V03 after the completed Phase 1; it does not reopen or falsely extend Phase 1's completion. The user's artwork clarification supersedes the original hand-drawn-motif proposal.

| Gate | Backlog tasks | User-visible outcome |
| --- | --- | --- |
| Visual refinement | V01, V02A, V02B, V03 | Spacious sanctuary, real species artwork, native/cell rendering, deliberate evolution selection |
| Phase 2 | E01, E02, C01, C02 | Resume a real task and assemble source context without a model |
| Phase 3 | R01, R02 | Approve/run/cancel commands and inspect durable, freshness-aware evidence |
| Phase 4 | A01, A02, A03 | Inspect real provider capabilities, make a scoped plan, implement a bounded change |
| Phase 5 | U01, L01, G01 | Evidence-led diagnosis, behavior-preserving refinement, independent review |
| Phase 6 | S01, S02, S03 | Verifiable handoff, approved habits, untrusted-recipe preview |
| Phase 7 | Q01, Q02 | Complete real workflows, visual state coverage, observed performance |

Default execution follows backlog order. Explicit dependencies allow later parallel work only when interfaces and file ownership are clear; do not parallelize merely because forms have different names. Each increment must leave a usable, honest application. Backend calls and durable-state promises remain unavailable until their own acceptance gates pass.

Next four increments:

1. V01 changes presentation only: centralize style tokens, reduce duplicate status text, create a single task/action card, and verify each supported layout in the real terminal.
2. V02A replaces hand-drawn symbols with source-tracked actual species images and an offline cell renderer; no implicit download, asset redistribution claim, or native-image claim.
3. V02B adds native image protocols through `ratatui-image`, with actual-terminal validation, bounded discovery, caching, cleanup, and compatibility fallbacks.
4. V03 turns form selection into an image grid/list with a role/artifact/prerequisite preview. It remains a local action, not a provider request or permission grant.


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

1. Implement approval records and capability enforcement at dispatch.
2. Add direct-argv execution, explicit shell mode, cwd, process-group cancellation, bounded output spooling, and terminal escape sanitization.
3. Associate runs with before/after code state; record failures and partial output honestly.
4. Complete Jolteon's history and live-run workspace.

Acceptance: execute real successful, failing, long-running, high-output, and child-spawning commands in a throwaway directory. Deny an approval and prove no process started. Cancel and confirm children terminate. Inject terminal control sequences and verify they cannot alter the host terminal. Exercise a file change during a run and show ambiguous evidence.

## Phase 4 — Real agent connection and scoped implementation

1. Inspect and document the installed Codex CLI interface; implement one supported protocol/version.
2. Connect structured events to the task reducer, cancellation, output capture, and provider status.
3. Assemble requests from selected source ranges, task constraints, form instructions, and evidence references only.
4. Complete Flareon and Espeon interactions; show external-provider permission limitations prominently.

Acceptance: with real credentials, plan and implement a small change in a disposable repository, inspect its diff, request an actual verification run, and resume the task. Exercise missing executable, failed authentication, malformed provider events, and interrupted runs without claiming success. Lack of credentials blocks this acceptance; mocks do not replace it.

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

1. Exercise all nine forms in one real repository task; confirm every role yields its specified artifact.
2. Inspect actual terminal rendering across supported sizes and color modes, including long paths, CJK, multiline errors, approval overlays, and stream scrolling.
3. Measure release startup, idle CPU/RSS, key-to-render latency, and bounded memory under the workloads in the design contract.
4. Fix measured bottlenecks and broken contracts; then run formatting, linting, and the relevant test suite once after the integrated changes.
5. Update usage documentation, changelog, troubleshooting entries for resolved non-trivial issues, and checkpoint. Remove throwaway fixtures and scripts.

Acceptance: publish observed measurements and real run results, not targets as achievements. All nine roles, persistence, cancellation, permissions, evidence freshness, and terminal restoration must work. No unimplemented forms or fake fallbacks may be presented as complete.

## Explicit exclusions

No custom model runtime, autonomous multi-agent swarm, hosted synchronization, plugin marketplace, automatic commits/merges, or background model calls for pet animation. No claim of canonical game evolution mechanics or affiliation. Public distribution requires a naming/art review.

## Current next action

Implement **V01 — Sanctuary layout and visual hierarchy** from `task.md`. Start from the existing `src/ui.rs` and actual Phase 1 terminal capture, reserve a bounded real-artwork area, refine spacing/hierarchy/empty states without inventing tasks or introducing storage, and prove the result in the real TUI across widths and color modes. V02A/V02B/V03 follow; E01 remains the first storage increment. Do not infer that planning, downloaded preview images, or the SVG mockup has implemented any of these tasks.
