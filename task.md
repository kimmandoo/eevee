# Tasks

## Active work

Build the Eevee workbench in small, runnable increments. The next implementation task is **V01 — Sanctuary layout and visual hierarchy**. This refinement was requested after Phase 1; visual work now precedes the first storage increment. Do not start all tasks at once.

Contract: [design rules](docs/DESIGN_RULES.md). Architecture and phase gates: [implementation plan](docs/IMPLEMENTATION_PLAN.md). Each unchecked item is planned, not implemented.

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

## A — Make Eevee feel like a companion

### V01 — Sanctuary layout and visual hierarchy

- [ ] Deliver the next visual increment.
- **Depends on:** Phase 1.
- **Change:** Refine `src/ui.rs` with centralized theme tokens, spacious wide-screen gutters/padding, compact layouts, a single task/action card, and a contextual footer. Keep current backend/persistence limitations truthful. Put state in the header, identity in the rail, artifact in the center, and permissions/provenance in the inspector; remove duplicated paragraphs. Do not add persistence or new provider calls.
- **Accept:** Launch the real binary at 120x36, 100x30, 70x24, and 60x18; create/edit a session goal, open drawers, and quit. The primary action and selected text remain readable in dark/light/indexed/mono/ASCII modes. Capture each layout and verify no overlapping controls or idle redraws.

### V02A — Actual nine-species assets and cell rendering

- [ ] Show real Eevee/evolution artwork from a local, source-tracked cache.
- **Depends on:** V01.
- **Change:** Map the nine species IDs in the design contract to one consistent transparent official-artwork PNG set. Support a local asset directory and an explicit user-initiated source download; record source/revision/hash/attribution without bundling franchise bytes by default. Bound PNG decode/allocation and render the same image as cached half-block cells or luminance ASCII/mono. Remove the hand-drawn `motif` path when cutting over; missing/corrupt art gets a truthful asset state, not a substitute mascot. Images never enter LLM context.
- **Accept:** Load all nine real images, switch forms offline, and capture actual cell-rendered portraits at wide/compact sizes. Verify aspect/alpha, NO_COLOR/ASCII, missing/corrupt/oversized files, and no network/model call on selection. Native pixel fidelity is not claimed by this task.

### V02B — Native terminal artwork and graphics lifecycle

- [ ] Display the same source images at pixel fidelity on supported terminals.
- **Depends on:** V02A.
- **Change:** Add a compatible pinned `ratatui-image` version with PNG-only decoding and no default Chafa runtime dependency. Implement bounded protocol/cell-size discovery with one input owner; cache resize/encoding work outside rendering. `--images auto|cells|off` controls native/fallback behavior. Clear/reposition image resources on form changes, overlays, resize, and exit; preserve the text layout and existing terminal restoration.
- **Accept:** Show Eevee and all eight forms in an actual graphics-capable terminal, not only TestBackend or a text PTY. Exercise image-bearing overlays, repeated form switches, resize, dark/light surfaces, and quit without stale images. Validate each native protocol before advertising it; otherwise keep it disabled and use working cells mode. Exercise a no-response probe and an unsupported/multiplexed terminal path without startup hangs, stolen keys, or colored graphics under NO_COLOR.

### V03 — Evolution picker and deliberate handoff preview

- [ ] Make choosing a form an understandable local interaction.
- **Depends on:** V02B.
- **Change:** At >=90x30, show a 3x3 grid of actual species thumbnails with a selected detail area; otherwise use a compact list and selected portrait. Show the role, expected artifact, current form, and prerequisite status. Arrows/1-9 preview; Enter selects; Esc restores the previous form. Keep task text intact. Previewing or selecting grants nothing and starts no work; unavailable provider actions are labelled unavailable rather than simulated.
- **Accept:** Select/cancel each form using only the keyboard at wide and narrow sizes. Confirm the goal and focus survive; selection and activation are distinct; all nine forms are reachable. Capture color/mono/native/cells states and verify images are removed behind overlays instead of bleeding through.

**Gate A:** a polished, honest local companion shell. No persistence or agent execution claimed.

## B — A useful local workbench before connecting a model

### E01 — Durable goal and save acknowledgement

- [ ] Make one task survive application exit.
- **Depends on:** V01.
- **Change:** Add a SQLite store worker and bounded worker/input events without blocking rendering. Use a documented platform user-data directory with `--data-dir PATH` for explicit isolation. Store the current task goal, project directory, and selected form transactionally. Show saving/saved/save-failed only from actual worker results; quitting during a pending or failed save must not silently discard edits. No source indexing yet.
- **Accept:** In a temporary data directory, edit a goal, wait for the save acknowledgement, quit, and reopen in the same project. Recover the exact text and form. Exercise read-only storage, rapid successive edits, and quit-before-ack; keep the latest committed value and never label a failed write saved.

### E02 — Eevee task cards and resumption

- [ ] Turn the home screen into a real task sanctuary.
- **Depends on:** E01, V03.
- **Change:** Create/select/resume multiple project-scoped tasks, edit acceptance criteria, and record an explicit next action. Replace the session-only CTA with Create/Resume only now. Use actual saved timestamps/status; no fictional activity. Avoid assigning completion from a form switch or process result.
- **Accept:** Create two tasks with different goals/criteria/forms, switch between them, restart, and resume each without cross-contamination. A new project starts empty. Pending edits are resolved before switching; a task with unmet criteria cannot silently become complete.

### C01 — Repository baseline and working diff

- [ ] Distinguish existing work from session changes.
- **Depends on:** E02.
- **Change:** Add repository discovery, initial tracked/untracked file identities, and a read-only working diff in a repo worker. Preserve dirty user files. Label external or ambiguous edits as unattributed; handle a non-Git directory as an explicit unsupported repository state without losing its task.
- **Accept:** Open a disposable dirty Git repository containing tracked/untracked edits; take the baseline, edit externally, and inspect the result. Existing user changes remain unchanged, new modifications are visible without false authorship, and the UI stays responsive while loading.

### C02 — Vaporeon source map and context basket

- [ ] Assemble a small, inspectable request context.
- **Depends on:** C01.
- **Change:** Browse/search real files, inspect ranges, and pin/remove source excerpts with path/range/source identity. Respect ignore rules, explicit exclusions, binary/oversized-file limits, and symlink boundaries. Preview included content/bytes; label token estimates and keep them separate from provider usage. Do not inject all files or role prompts.
- **Accept:** Build a context basket in a fixture repository with CJK paths, a binary file, excluded credentials, and a symlink outside the approved root. Inspect exact included ranges, remove one, and modify another; the stale reference is visible and excluded/out-of-scope content is absent from persisted context and outgoing request previews.

**Gate B:** create -> exit -> resume -> inspect sources works without a model. This completes Phase 2 only when E01, E02, C01, and C02 pass.

## C — Real execution with visible evidence

### R01 — Jolteon approve, run, and cancel

- [ ] Execute one explicit command safely from the TUI.
- **Depends on:** C01, V03.
- **Change:** Add a command request card with exact argv or explicit shell text, cwd, scope, and duration of permission. Enforce approval at dispatch. Stream sanitized output through bounded buffers/spooled files; record start/end, exit/signal status, and before/after source identities. Support cancellation of the owned process group from the first exposed execution path. No provider required.
- **Accept:** Approve successful/failing commands, deny a command and prove it never ran, cancel a child-spawning command, and emit terminal escape sequences plus a large stream. No orphan child, arbitrary terminal control, unbounded transcript, or synthesized success; the input loop remains usable. Changing forms cannot start or abandon active work.

### R02 — Execution journal and evidence freshness

- [ ] Make past results useful without overstating their validity.
- **Depends on:** R01, E02.
- **Change:** Persist executions and output references; add Jolteon history and a reusable evidence inspector. Distinguish current, stale, ambiguous, failed, and interrupted evidence. On restart do not claim a previously running process is still owned/running; reconcile it as interrupted unless ownership is actually re-established.
- **Accept:** Run a check, restart and inspect its captured output, then change an input file and see stale status. Change a file during a run and see ambiguous status. Terminate the app during work and reopen without a false success or stuck running indicator.

## D — Connect a real specialist, not nine chat sessions

### A01 — Provider preflight and truthful connection state

- [ ] Show what the installed Codex integration can actually support.
- **Depends on:** R01, C02.
- **Change:** Inspect the installed CLI version and documented machine-readable/cancellation/approval/resume interfaces. Implement a connection/preflight surface and record the supported protocol. Missing executable, credentials, or enforceable capability mediation must be explicit. Unmanaged operation is separately labelled and deliberately accepted; never imply sandboxing from prompts.
- **Accept:** Exercise a supported installation plus missing executable/authentication prerequisites. The UI shows an actionable real state, not a fake response. Record actual supported commands/protocol rather than guessed flags; no credential text enters logs.

### A02 — Espeon scoped planning request

- [ ] Complete the first real model round trip.
- **Depends on:** A01, E02, C02.
- **Change:** Add the real adapter's structured events, request lifecycle, cancellation, and error handling. Let Espeon submit a selected-context design question, compare alternatives, and save a user-selected decision/ordered plan. Reuse the same task journal; do not start a provider process per form merely to switch views.
- **Accept:** With real credentials, submit a bounded question against a disposable repository and save the chosen approach. Cancel another request; exercise a provider failure. Inspect the exact outgoing context and recorded result. Authentication/protocol availability is a real blocker; a mock cannot satisfy this gate.

### A03 — Flareon bounded implementation

- [ ] Turn an approved requirement into an inspectable real patch.
- **Depends on:** A02, R02.
- **Change:** Request a change within approved paths using the actual provider capability boundary. Keep the baseline/user changes visible, show the working diff, and offer an explicit verification handoff to Jolteon. Never auto-commit, auto-merge, or silently widen the edit scope.
- **Accept:** Implement a small real change in a disposable dirty repository. Review the diff against the requirement, verify user edits were preserved, and run the chosen check via Jolteon. An out-of-scope request is rejected in managed mode; unmanaged limitations remain prominent.

## E — Distinct diagnosis, refinement, and review

### U01 — Umbreon hypotheses and diagnosis

- [ ] Track explanations against real observations.
- **Depends on:** A02, R02.
- **Change:** Add a hypothesis board with supporting/refuting evidence links and explicit unresolved states. Diagnostic execution goes through Jolteon approvals; a diagnosis alone does not edit source. Security findings use the same evidence discipline, not severity by guesswork.
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
- **Depends on:** G01, E02.
- **Change:** Preview a handoff containing the goal, observed changes/results, decisions, unresolved risks, and exact next action. Include source/run references, distinguish model inferences, and preview any documentation write separately. No automatic declaration of completion.
- **Accept:** Close/reopen a task with one unresolved finding and resume from the recorded next action. Follow each result link to real evidence. Reject a proposed documentation edit and confirm the file is unchanged.

### S02 — Explicitly curated project and personal habits

- [ ] Let the companion learn only what the user approves.
- **Depends on:** S01, C02.
- **Change:** Offer Remember on a selected verified/user-authored note; preview task/project/personal scope, then approve or reject. Support inspection/edit/delete and honor exclusions. Do not silently infer a permanent habit from one action or add a numerical bond score.
- **Accept:** Approve one project memory and reject another; restart and inspect exact contents. Open a different project and confirm scope isolation, then delete the memory and verify it no longer enters context. Excluded secrets are never proposed from captured content.

### S03 — Evolution Stone recipe preview

- [ ] Reuse workflows without creating a permission bypass.
- **Depends on:** V03, R02, S01.
- **Change:** Load versioned TOML recipes describing form steps, context/scope, validation commands, and completion criteria. Show the recipe before applying it to a task. No install hooks, marketplace, automatic execution, or authority to expand existing grants.
- **Accept:** Preview a local bug-fix recipe, apply its plan without executing commands, and require normal approvals during use. Reject an invalid schema. A recipe requesting an ungranted write/run capability cannot perform that action.

## G — Integrated acceptance

### Q01 — Complete journey and visual state matrix

- [ ] Prove all nine workflows and their composed user experience.
- **Depends on:** V01, V02A, V02B, V03, E02, C02, R02, A03, U01, L01, G01, S01, S02, S03.
- **Change:** Exercise a real task through intake, context, design, diagnosis, implementation, verification, refactor, review, and handoff. This is an acceptance exercise, not a mandatory user route. Fix discovered integration/visual defects; no fake backend qualifies.
- **Accept:** Every form yields its real artifact; restart/resume and cancellation preserve state. Capture ready/running/approval/blocked/complete/cancelled and save-failure states across supported sizes/themes, including long CJK text. No misleading capabilities, lost evidence, orphaned processes, or clipped primary actions. Review naming/art rights before public distribution; an internal acceptance result is not legal clearance.

### Q02 — Measured native and token efficiency

- [ ] Measure the performance contract rather than advertise unverified speed.
- **Depends on:** Q01.
- **Change:** Benchmark the release binary on recorded hardware and workload: 60s idle CPU/RSS, 30 warm startups, local key-to-render latency under streaming, 10 MiB output, and a 10,000-line diff. Inspect context assembly and provider-reported usage separately from estimates. Optimize only measured bottlenecks.
- **Accept:** Report observations against `docs/DESIGN_RULES.md` targets with method/sample size and remaining misses. Buffers/queues remain bounded, form switches cause no model request, and unrelated files/full history are absent from scoped requests. No token-saving percentage without a defined measured baseline.

## Phase status

Phase 1 remains complete. V01/V02A/V02B/V03 and Phases 2-7 remain pending. No implementation task above was completed by writing this backlog. See `docs/WORK_CHECKPOINT.md` for the exact continuation point and existing verification evidence.
