# Eevee Implementation Plan

Date: 2026-10-13
Status: 25 of 31 tasks passed their scoped gates, including P02 and U01/L01/G01/S01/S02/S03. Schema-6 specialist, memory and recipe journeys passed all five native installed-runtime rows at `d198fa5`; all eleven build/smoke/policy jobs passed. Local real independent Codex review and measured release workloads also passed. Full A04/A06/A07 and therefore Q01/Q02/P03 remain pending on permitted Claude/Gemini authentication and their required conformance rows. The separately authorized 0.1.0 core release promotes the exact qualified packages.
Contract: [design rules](DESIGN_RULES.md) and [agent compatibility](AGENT_COMPATIBILITY.md). The latter distinguishes documented interfaces, local CLI observations, and runtime conformance still to be performed.

## Outcome

Build a Rust terminal workbench that preserves one task across Eevee and all eight evolutions. All nine forms must have functional role-specific surfaces and workflows, not cosmetic tabs. Prioritize observable evidence, safe execution, minimal model context, and a polished keyboard-first interface.

## Architecture

One native Rust binary and one Cargo package with these module boundaries; an optional provider SDK bridge stays outside the UI/core:

- `app`: reducer, typed events, task/form transitions, capability checks, independent task/request/storage/evidence states.
- `domain`: task, form, evidence, execution, finding, correlated approval/question, memory, recipe, agent profile, handoff.
- `ui`: responsive and linear layouts, theme tokens, nine workspaces, focus/scroll restoration, request previews, palette, overlays.
- `runner`: owned subprocesses, correlated approvals, bounded events, output sanitization/spooling.
- `store`: SQLite migrations, durable events/snapshots, output references, recovery.
- `repo`: Git baseline, working-tree identities, diff, external-change detection.
- `context`: bounded source catalogue/range capture, exclusion/containment checks, exact payload assembly and source revalidation.
- `agent` / `provider`: profile/version admission and the minimal routed request/event contract; concrete `codex`, `claude` and shared `acp` workers own transport/policy details.
- `handoff`: strict version-1 curated facts/source identities/decisions, create-only JSON/Markdown I/O, review and dispatch-binding validation; never session/grant portability.
- `artwork`: bounded local PNG/source-manifest loading, install-time or explicit hash-pinned original sprite downloads, shared integer pixel fitting, and cached nearest-neighbor half cells. Online installers prepare all nine forms before publication; offline archives never download, and startup reads the same platform `eevee/sprites` cache without networking.
- `native`: capability-negotiated Sixel/Kitty, original-pixel integer geometry, cached bounded encodings, physical-cell diff widths and owned placement/resource cleanup. Automatic mode never substitutes character mosaics.
- `input`: one terminal-input owner and shared bounded VT/graphics decoder; blocking poll/SIGWINCH on POSIX, Unicode console records on Windows. Capability discovery expires after 500ms while late/split replies remain filtered from editing.
- `terminal`: capture/restore terminal modes and graphics cleanup on normal/error/panic exit.

The reducer owns mutable application state. Workers send typed events; render functions perform no I/O and report only derived viewport scroll bounds back to state. Store writes run on a dedicated blocking worker. Large payloads are referenced rather than repeatedly cloned into events.

Connect Codex first through a local app-server child whose installed identity is reviewed and revalidated. Explicit setup/update resolves current stable provider releases; admission requires the actual protocol, capabilities and policy acknowledgments rather than an exact release-number allowlist. Add the official Claude Agent SDK through an optional TypeScript sidecar, then a shared ACP v1 client exercised against both OpenCode and Gemini. The SDK bridge is a deliberate compatibility exception, not a rewrite or mandatory runtime for local work. Unsupported CLIs use a reviewed manual handoff, not terminal scraping. See [agent compatibility](AGENT_COMPATIBILITY.md) for primary sources, exact protocol distinctions, auth/terms constraints, and acceptance scenarios.

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

Current implementation: Codex 0.152.1 Plan plus POSIX controlled replacement of 1–8 existing regular UTF-8 files ≤12 KiB, native tool-origin schema-5 journals, optional Node ≥24 / official SDK 0.3.296 bridge, OpenCode 1.18.0 `acp --pure` and Gemini 0.28.0 `--experimental-acp` on one ACP1 client, negotiated metadata-only session-list inspection, idle agent/operation switches and schema-1 manual/cross-agent handoff. Actual dirty-patch/local-check and OpenCode prompt/permission/cancel/load gates passed. SDK module/policy inspection and Gemini handshake/capability refusal do not replace their missing authenticated scenarios. Full Q01/P03 remain gated; no provider/native-image/platform parity or commercial entitlement is inferred.

Authentication continuation: explicit official login/status is implemented separately from model requests, rather than treating missing login UX as only an external credential blocker. Codex browser/device OAuth and OpenCode native auth selection use dedicated interfaces. Gemini 0.28.0's actual ACP authentication emitted interactive text on JSON stdout; the implementation instead hands the terminal to its official CLI with explicit external-workflow/configuration/chat disclosure, without collecting codes or sending task prompts. Each flow has reviewed effects, bounded owned terminal handoff, and fresh-request consent after return. Claude SDK retains the subscription restriction while supporting explicit permitted cloud provider chains. Human OAuth completion and credentialed SDK/Gemini model conformance remain separate evidence gates.

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

### Observed local release performance — 2026-10-14

`scripts/performance_smoke.py`, `target/performance-complete/results.json`: Apple M1 Pro (8 logical CPUs, 16 GiB), macOS 27.0.1 arm64, Rust 1.90.0 release, Python 3.12.15/pyte 0.8.2/wcwidth 0.2.13/psutil 7.0.0, native PTY 120×36, monochrome, images off. Binary SHA-256 `94a6644b171eb9802bacdea1cb805c4c717f01637469ebaba490b8c02affecc0`. These are local observations, not all-platform claims.

| Workload / method | Observed | Design target / outcome |
| --- | --- | --- |
| Warm start to complete saved frame, 30 launches | p95 89.55 ms | <200 ms; met |
| Idle saved local home, 60.008 s | 0.826% of one core; 7.44 MiB core peak RSS; zero terminal output | <1% / <80 MiB; met |
| Key-to-visible-render while streaming, 60 keys | p95 12.61 ms | <50 ms; met |
| Actual 10 MiB local output | 1 MiB retained / 9 MiB discarded; truncation visible | bounded output; met |
| 10,000-line diff, 5,000 removals + 5,000 additions | overview 137.66 ms; End-to-last-line 14.02 ms | repository/Git bytes unchanged |

Resource scope is explicit rather than hidden in the core figures:

| Window / scope | CPU (% of one core) | Sampled peak RSS (MiB) |
| --- | ---: | ---: |
| Idle / measurement harness | 28.904 | 45.61 |
| Idle / application + harness | 29.730 | 53.05 |
| Streaming / core | 2.660 | 7.97 |
| Streaming / local command | 3.485 | 14.19 |
| Streaming / observed application total | 6.145 | 22.13 |
| Streaming / measurement harness | 40.590 | 47.31 |
| Streaming / application + harness | 46.735 | 68.67 |

The sampler includes observed process descendants; short-lived/inaccessible helpers can be missed, CPU is a lower bound and RSS is a sampled simultaneous sum with possible shared-page double counting. The desktop terminal compositor is excluded. SDK/external-provider scopes were unavailable in this local-shell workload and are null, not zero; there is no token/cost or savings claim. Actual Codex/OpenCode context/permission/continuation checks and separate fresh-review checks passed, but do not substitute for missing Claude/Gemini interoperability or resource measurement. Full Q02 remains pending on Q01.

Two additional actual Codex 0.152.1 independent review requests were measured on the same host/binary (`target/review-resource-measured/resources-summary.json`). Approval-to-completion windows were 9.97 s / 192 samples and 8.46 s / 163 samples, with a requested 20-ms sampling interval. Both reviews again preserved project bytes and used separate fresh threads.

| Observed scope | Review 1 CPU (%) | Review 1 RSS (MiB) | Review 2 CPU (%) | Review 2 RSS (MiB) |
| --- | ---: | ---: | ---: | ---: |
| Native core | 0.484 | 8.52 | 0.476 | 8.55 |
| Other app-owned helpers, including version preflight | 0.928 | 58.08 | 0.697 | 43.70 |
| Owned Codex app-server launcher and descendants | 7.980 | 220.25 | 7.907 | 229.70 |
| Observed application total | 9.392 | 228.77 | 9.080 | 238.22 |
| Measurement harness | 49.657 | 41.08 | 49.585 | 42.17 |
| Application + measurement harness | 59.048 | 269.84 | 58.665 | 280.39 |

CPU uses one-core percentages. Aggregate RSS is the sampled simultaneous sum, not the sum of independently timed scope peaks. Provider/version-preflight helper costs remain visible and included, not hidden inside a core-only claim. SDK resources remain unavailable. Provider-added effective context, comparable token/cost totals and savings remain unknown; these resource observations do not complete the blocked interoperability gates.

P03 remains the full distribution gate after P02, Q01, and Q02: prove native installation, update, uninstall, durable-schema continuity and measured integrated behavior. The user separately authorizes a scoped 0.1.0 local-core release after all five current native rows and SDK policy checks pass. That release includes verified archives/notices, pinned HTTPS bootstrap, embedded-installer self-update and owned uninstall without data purge; it does not claim full provider conformance, performance acceptance, signing/notarization, or artwork/trademark clearance.

## Explicit exclusions

No custom model runtime, autonomous multi-agent swarm, hosted synchronization, plugin marketplace, automatic commits/merges, agent-TUI scraping, or background model calls for pet animation. No new MCP server or generic plugin framework in this plan. No claim of canonical game evolution mechanics or affiliation. Public distribution requires naming/art review plus provider authentication/branding/terms review.

## Completed request — native provider connectivity (v0.1.7)

v0.1.7 delivered native model-provider connectivity and model discovery from qualified source `0a43c0e`, with all five native build/install rows and public package/runtime acceptance. The following was its accepted implementation sequence; its immutable tag remains unchanged.

1. Add native ChatGPT, OpenAI Responses, Anthropic Messages, Google Gemini and configurable OpenAI-compatible connections alongside existing CLI/SDK/ACP adapters. Resolve user-scoped endpoint/model settings and environment-held keys without project commands. Existing ChatGPT session credentials may be used read-only by the explicitly disclosed native route; never send them to a custom endpoint or persist/copy them into task data.
2. Keep model discovery independent from request eligibility: optional reasoning metadata, exact configured/custom IDs, searchable lists, retained snapshots on refresh errors, and provider-side validation rather than stale CLI allowlists. Preserve explicit model identity and reject silent substitution.
3. Keep immutable payload review and explicit Run, cancellation ownership and truthful observed-model reporting. Native requests expose no tools or source-writing capability; implementation replies are proposals, not applied edits. Existing controlled-agent editing remains a separate route.
4. Exercise native account discovery and real response/cancellation in the actual terminal at wide/compact sizes; run unit/policy/workbench/terminal acceptance. Update binding contracts and failures with observed evidence.
5. Commit the candidate, qualify exact-commit Check and five native build/test/install rows, publish only their immutable artifacts, verify public hashes and installed runtime, and record completion in the checkpoint. Do not advance PT06A or claim unrelated authenticated provider conformance.

## Completed request — inline sending and remembered models (v0.1.8)

All five steps completed at immutable v0.1.8 source `53f3fd0`: exact-commit native qualification, public publication, installed runtime, schema safety and actual GPT-6.1/low restart/direct-send proof passed. See `WORK_CHECKPOINT.md` for commands, run IDs and counts. No PT06A work was started.

1. Make ordinary composer Enter the explicit send action after matching durable save and immutable local preparation, with no routine second popup. Retain approval for commands, writes, write-enabled sessions, project executables, external helpers and unmanaged connections; repeats/paste never dispatch.
2. Show one Codex provider, with native no-tool responses and CLI tool/edit methods below it. Preserve their distinct credential audiences, session ownership and explicit method selection.
3. Persist only nonsecret last-used provider/method and per-route exact model/effort. Migrate Schema 9→10 transactionally, respect explicit `EV_PROVIDER`, block fallback dispatch during failed/pending hydration, preserve drafts on failure and await preference commit on quit.
4. Exercise actual TUI direct sends, real connection/permission boundaries, no forced effort popup, immediate-quit/restart preferences, competing startup providers and old-binary refusal. Keep API/account qualification limits truthful.
5. Commit the complete v0.1.8 source; qualify Check plus five native build/test/install rows at that exact commit. Publish only their immutable artifacts, verify all public hashes and installed runtime, and record completion. Do not retag v0.1.7 or advance PT06A.

## Completed request — PT06 party routing and v0.1.9 release

1. PT06A: attribute new runs and provider journal records with a partner id plus name/form snapshot; older records display `partner unknown`; attribution never enters packets or handoffs. (done `43c7834`)
2. PT06B: key provider request state by partner; route events/questions/approvals by (task, partner, generation); keep work with its owner across lead switches; refuse late events for other or released partners. (done `797edab`)
3. PT06C: name the asking partner in run approval/cancel, permission/approval and question dialogs without hiding choices at 60×18. (done `9863913`)
4. Bump to 0.1.9, push main, require Check plus five native build/test/install rows at that exact commit, tag `v0.1.9`, verify public assets and checksums, and record completion. (done: qualification `38841536756`, publication `38842493846`)

## Completed request — shallower model and attachment UX (released in v0.1.9)

All four steps completed: final-source Rust checks and six targeted actual-TUI journeys passed, with additional workbench/context/terminal and isolated install/update evidence recorded in `WORK_CHECKPOINT.md`. No release was requested or published; PT06A remained unstarted.

1. Reuse install animation for self-update release lookup and downloads; preserve locale, static redirected output and read-only checks.
2. Compact model setup into model → supported reasoning level → retained editor, with distinct current/focus styling and optional complete metadata/error details. Add exact-first search, non-destructive return to the originating model search, direct exact-ID/effort arguments and direct ready-provider activation through existing reviews.
3. Give `@` and source browsing direct bounded preview/attach/remove controls; filter paths locally with case-insensitive scoped abbreviations and keep saved identities/ranges visible beside the draft. Return after durable attachment acknowledgment without losing the draft or weakening exclusions, secret review or permission boundaries.
4. Exercise actual wide/compact terminal workflows and install/update animation, then run integrated Rust checks and update session evidence. Do not advance PT06A or publish a release.

## Completed request — EV03B owning-form redirect (not released)

1. Resumed the uncommitted EV03B command routing, tests, compact history layout and smoke-driver migrations rather than skipping to EV07.
2. Kept command preparation with Jolteon and source-edit setup with Flareon/Leafeon. Offered the existing deliberate form-switch path after refusal without replaying the action, changing grants or covering an existing neutral review.
3. Passed Rust 1.90.0 all-target tests (629 passed, 2 ignored), fmt, strict Clippy and release build. Passed six actual-TUI redirect cases at 120×36/60×18, eight execution cases, six recovery cases and fifteen workflow cases.
4. Updated DESIGN_RULES, task status, changelog, troubleshooting and checkpoint. No schema/strategy change or release; EV07 is next when requested.

## Completed request — wave 2 Partner basics, one task per commit

1. EV07 — **Done:** top Return to Eevee picker action and guarded `/eevee`, byte-preserved task/draft/evidence payloads, explicit cancellation before switching, unknown-outcome holds. Rust 1.90.0: 632 passed / 2 ignored, fmt/strict Clippy/release build passed; actual TUI passed 16 cases at 120×36/60×18 and the execution regression passed 8.
2. PK01 — **Done:** personal Unicode nickname editor, transactional reserved-column uniqueness/bounds/clear, personal-only and task-backed restart, committed identity snapshots and zero automatic packet flavor. Rust 1.90.0: 644 passed / 2 ignored, fmt/strict Clippy/release build passed; actual TUI passed eight journey groups including all nine forms at both sizes, boundary/duplicate/clear and real loopback payload/receipt isolation.
3. EV01A — **Done:** one ordered evidence rule table, stable local trigger identities, guarded current source/run/review facts and cached off-render evaluation. Rust 1.90.0: 658 passed / 2 ignored, fmt/strict Clippy/release build passed. Actual TUI passed 18 cases at both sizes covering every rule, no match, Glaceon → Jolteon after source edit/refresh, accepted review, response coexistence and no automatic work/packet insertion.
4. EV01B — **Done:** textual picker recommendation with unchanged explicit selection, identity-specific durable task-local dismissal and version-only Schema 11 migration. Rust 1.90.0: 665 passed / 2 ignored, fmt/strict Clippy/release build passed. Actual TUI passed eight journey groups at both sizes including restart, form-only changes, new exit-7 run resurfacing and unchanged Unicode draft/evidence; actual Schema 10→11 migration/refusal/resume passed all three cases without old-binary mutation.
5. EV02 — **Done:** transient stored-state/capability record, normal next-key/paste delivery, exact Companion reopening and cached guarded hypothesis counts after context settlement. Rust 1.90.0: 680 passed / 2 ignored, fmt/strict Clippy/release build passed. Actual TUI passed 16 journey groups at 120×36/60×18 in monochrome Unicode/ASCII: three switch pairs matched stored counts/diffs, input/carets stayed exact, and four deliberately requested loopback responses produced no automatic extra send/run. Actual execution regression passed all eight journeys.

Each task was implemented in existing modules, verified with relevant tests and actual TUI, documented and committed independently. This request is complete; no next implementation step is active. EV01C remains in wave 3; publication and replacement of the installed command remain unrequested.

## Active request — release wave 2, then EV10A

1. Prepare v0.1.10 with Schema 11 compatibility notes, retain the released v0.1.9 schema fixture and locally verify the candidate.
2. Commit/push the candidate to main; require same-commit Check and all five native build/test/offline-install rows to succeed in one attempt. Tag only that qualified commit; promote its exact immutable packages through the existing publication workflow.
3. Verify the public version/assets/checksums and exercise the published Linux binary using disposable data, including Schema 10→11 compatibility. Record publication before changing feature code.
4. Implement EV10A: a reusable pure retained-output location parser for Rust/cargo, Python tracebacks, Node/TypeScript and generic path:line[:column], exposed in the existing run view without reading extra spool bytes or opening/pinning files.
5. Prove real failing cargo test and pytest locations plus no-match behavior, actual TUI at 120×36/60×18, update contracts/task/changelog/checkpoint and commit EV10A independently. EV10B remains the next task, not part of this request.

