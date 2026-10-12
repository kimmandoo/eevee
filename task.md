# Tasks

## Active work

Build the Eevee workbench in small, runnable increments. The next implementation task is **V01 — Sanctuary layout and visual hierarchy**. This refinement was requested after Phase 1; visual work now precedes the first storage increment. Do not start all tasks at once.

Contract: [design rules](docs/DESIGN_RULES.md). Architecture and phase gates: [implementation plan](docs/IMPLEMENTATION_PLAN.md). Adapter/evidence contract: [agent compatibility](docs/AGENT_COMPATIBILITY.md). Each unchecked item is planned, not implemented.

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

## A — Make Eevee feel like a companion

### V01 — Sanctuary layout and visual hierarchy

- [ ] Deliver the next visual increment.
- **Depends on:** Phase 1.
- **Change:** Refine `src/ui.rs` with centralized theme tokens, spacious wide-screen gutters/padding, compact layouts, a single task/action card, and a contextual footer. Keep current backend/persistence limitations truthful. Put state in the header, identity in the rail, artifact in the center, and permissions/provenance in the inspector; remove duplicated paragraphs. Do not add persistence or new provider calls.
- **Accept:** Launch the real binary at 120x36, 100x30, 70x24, and 60x18; create/edit a session goal, open drawers, and quit. The primary action and selected text remain readable in dark/light/indexed/mono/ASCII modes; measure the app's own true-color contrast. Task/form/agent readiness and local-vs-provider action are identifiable without opening help. Capture layouts; no overlapping controls, duplicate state paragraphs, or idle redraws.

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

### UX01 — Predictable focus, editing, and accessible local navigation

- [ ] Make the local shell usable without relying on artwork or memorized shortcuts.
- **Depends on:** V03.
- **Change:** Implement focus restoration, per-surface selection/scroll state, grapheme-aware cursor editing, and explicit local-apply versus request-preview actions. Suppress global shortcuts in text/search/answer fields. Add a linear text layout with the same local actions, optional portraits, and readable monochrome selection; do not claim assistive-tool support before testing it.
- **Accept:** Complete create/edit/select/back/resize/quit using only keyboard navigation with CJK/combining text and multiline paste. Pasted commands/newlines never execute or approve anything. Closing a drawer restores focus, resize preserves drafts/selection, and linear layout exposes the same actions. Exercise a real assistive terminal setup before labelling that combination screen-reader compatible.

**Gate A:** a polished, honest local companion shell. No persistence or agent execution claimed.

## B — A useful local workbench before connecting a model

### E01 — Durable goal and save acknowledgement

- [ ] Make one task survive application exit.
- **Depends on:** V01.
- **Change:** Add a SQLite store worker and bounded worker/input events without blocking rendering. Use a documented platform user-data directory with `--data-dir PATH` for explicit isolation. Store the current task goal, project directory, and selected form transactionally. Show saving/saved/save-failed only from actual worker results; quitting during a pending or failed save must not silently discard edits. No source indexing yet.
- **Accept:** In a temporary data directory, edit a goal, wait for the save acknowledgement, quit, and reopen in the same project. Recover the exact text and form. Exercise read-only storage, rapid successive edits, and quit-before-ack; keep the latest committed value and never label a failed write saved.

### E02 — Eevee task cards and resumption

- [ ] Turn the home screen into a real task sanctuary.
- **Depends on:** E01, UX01.
- **Change:** Create/select/resume multiple project-scoped tasks, edit acceptance criteria, and record an explicit next action. Keep task outcome, request lifecycle, save acknowledgement, and evidence freshness separate. Replace the session-only CTA with Create/Resume only now. Use actual saved timestamps/status; no fictional activity or completion from a form switch/process result.
- **Accept:** Create two tasks with different goals/criteria/forms, switch between them, restart, and resume each without cross-contamination. A new project starts empty. Pending edits are resolved before switching; a task with unmet criteria cannot silently become complete.

### C01 — Repository baseline and working diff

- [ ] Distinguish existing work from session changes.
- **Depends on:** E02.
- **Change:** Add repository discovery, initial tracked/untracked file identities, and a read-only working diff in a repo worker. Preserve dirty user files. Label external or ambiguous edits as unattributed; handle a non-Git directory as an explicit unsupported repository state without losing its task.
- **Accept:** Open a disposable dirty Git repository containing tracked/untracked edits; take the baseline, edit externally, and inspect the result. Existing user changes remain unchanged, new modifications are visible without false authorship, and the UI stays responsive while loading.

### C02 — Vaporeon source map and context basket

- [ ] Assemble a small, inspectable request context.
- **Depends on:** C01.
- **Change:** Browse/search real files, inspect ranges, and pin/remove source excerpts with path/range/source identity. Respect ignore rules, explicit exclusions, binary/oversized-file limits, and symlink boundaries. Preview Eevee-sent content/bytes; label token estimates and provider-added context separately. Do not claim this basket is the full effective provider prompt or that agent-owned reads are constrained by an excerpt selection.
- **Accept:** Build a context basket in a fixture repository with CJK paths, a binary file, excluded credentials, and an out-of-root symlink. Inspect exact ranges, remove one, and modify another; show the stale reference. Excluded/out-of-scope content is absent from Eevee's stored basket/outgoing payload; disclose agent-added history/config/reads rather than asserting unverified global isolation.

**Gate B:** create -> exit -> resume -> inspect sources works without a model. This completes Phase 2 only when E01, E02, C01, and C02 pass.

## C — Real execution with visible evidence

### R01 — Jolteon approve, run, and cancel

- [ ] Execute one explicit command safely from the TUI.
- **Depends on:** C01, UX01.
- **Change:** Add exact argv/explicit shell text, cwd, scope, duration, and enforcement-source details. Queue approvals with neutral initial focus and correlated IDs; enforce approval at dispatch and reject stale/double answers. Stream sanitized output through bounded buffers/spooled files; record start/end, exit/signal, and before/after source identities. Support owned-process-group cancellation from the first exposed path. Keep provider-observed runs distinct from Eevee-launched commands; never duplicate execution to manufacture evidence.
- **Accept:** Approve successful/failing commands, deny and prove no side effect, cancel a child-spawning command, and emit escape sequences/large output. Open an approval while typing, dismiss/reopen it, change its inputs, and attempt a stale/double answer; no focus theft or accidental execution. No orphan child, uncontrolled terminal operation, unbounded transcript, or invented success. Navigation cannot abandon active work.

### R02 — Execution journal and evidence freshness

- [ ] Make past results useful without overstating their validity.
- **Depends on:** R01, E02.
- **Change:** Persist executions and output references; add Jolteon history and a reusable evidence inspector. Distinguish current, stale, ambiguous, failed, and interrupted evidence. On restart do not claim a previously running process is still owned/running; reconcile it as interrupted unless ownership is actually re-established.
- **Accept:** Run a check, restart and inspect its captured output, then change an input file and see stale status. Change a file during a run and see ambiguous status. Terminate the app during work and reopen without a false success or stuck running indicator.

### UX02 — Activity, interruption, and recovery without lost context

- [ ] Keep long-running work inspectable and recoverable.
- **Depends on:** R02, UX01.
- **Change:** Add stable scroll anchors, search, explicit follow-output mode, a new-output indicator, and layer-specific error details. Retain drafts and selected artifacts during pending input, cancellation, connection loss, and save failure. Show cancelling/outcome-unknown states without automatically retrying a potentially mutating action.
- **Accept:** Scroll/search while a real command streams, open/close its pending request view, cancel it, and revisit partial output without jumping to the bottom or losing the draft. Exercise interrupted startup/resume and a failed save; the correct layer/remedy is visible, quit does not silently discard data, and no old command re-executes.

## D — Connect a real specialist, not nine chat sessions

### A01 — Agent preflight and truthful connection state

- [ ] Show real capabilities and prerequisites for each installed or configured agent.
- **Depends on:** UX02, C02.
- **Change:** Introduce a small versioned agent-profile/capability contract and connection drawer for Codex, Claude Agent SDK, and ACP profiles. Inspect executable/protocol versions, effective configuration/trust scope, authentication/billing source labels, and adapter availability without a model request. Missing/unknown is not supported. Show per-action Eevee-mediated/agent-policy/observed boundaries; never mutate global agent configs or silently load an untrusted project hook/plugin.
- **Accept:** Probe the installed Codex interface and missing executables/SDKs; preserve local-only use. A version mismatch, absent auth, or unknown enforcement capability disables the affected run with an actionable reason. No repository-supplied command or auth flow starts without trust/action; no credentials appear in profile snapshots or logs. Record observed versus merely documented capabilities.

### A02 — Espeon scoped planning request

- [ ] Complete the first real model round trip through the Codex app-server adapter.
- **Depends on:** A01, E02, C02.
- **Change:** Use the pinned installed-version app-server contract, not ANSI screen scraping, for normalized structured events, approval/questions, cancellation, and explicit thread lifecycle. Let Espeon submit a previewed context manifest and save a user-selected decision. Correlate task/run/generation/tool/request IDs, preserve provider-origin metadata, and do not count delta plus final snapshots twice. Keep one active request; a draft is not a queued submission.
- **Accept:** With real credentials, run a bounded design request, answer a real supported input request, deny/allow a supported operation, cancel a turn, and resume the explicit thread when supported. Reject a stale approval after cancellation, and surface malformed/fatal transport outcomes without fake completion or automatic resend. Inspect outgoing context plus provider-added-context disclosure. Auth/schema availability is a real blocker; mocks do not satisfy this gate.

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

### S03 — Evolution Stone recipe preview

- [ ] Reuse workflows without creating a permission bypass.
- **Depends on:** V03, R02, S01.
- **Change:** Load versioned TOML recipes describing form steps, context/scope, validation commands, and completion criteria. Show the recipe before applying it to a task. No install hooks, marketplace, automatic execution, or authority to expand existing grants.
- **Accept:** Preview a local bug-fix recipe, apply its plan without executing commands, and require normal approvals during use. Reject an invalid schema. A recipe requesting an ungranted write/run capability cannot perform that action.

## G — Integrated acceptance

### Q01 — Complete journey and visual state matrix

- [ ] Prove all nine workflows and their composed user experience.
- **Depends on:** V01, V02A, V02B, V03, UX01, UX02, E02, C02, R02, A03, A04, A05, A06, A07, U01, L01, G01, S01, S02, S03.
- **Change:** Exercise real nine-form artifacts plus the shared UX across Codex, Claude Agent SDK, and the two ACP profiles. Run the compatibility conformance scenarios, not a mandatory all-agent tour for normal users. Fix actual integration/visual defects; no fake backend or single-agent-only test qualifies as interoperability.
- **Accept:** Cover empty/missing dependency, ready/running, permission/question, cancellation/unknown outcome, save failure, stale evidence, and cross-agent handoff across supported sizes/themes and long CJK content. Verify keyboard focus, no accidental execution, bounded streams, resumed usage accounting, image cleanup, and terminal restoration. Every claimed controlled action names and proves its enforcement boundary. Review naming/art rights and provider terms before distribution.

### Q02 — Measured native and token efficiency

- [ ] Measure the performance contract rather than advertise unverified speed.
- **Depends on:** Q01.
- **Change:** Benchmark the release binary on recorded hardware/workload: 60s local-shell idle CPU/RSS, 30 warm local startups, key-to-render latency under streaming, 10 MiB output, and a 10,000-line diff. Report core, optional SDK sidecar, and external-agent resource use separately and in aggregate. Inspect per-provider usage/cost scope and outgoing context; optimize measured bottlenecks.
- **Accept:** Report observations against the design targets with method/sample size and remaining misses, without hiding helper/agent costs inside a core-only claim. Queues stay bounded; no model request on form selection, no unsubmitted draft leaks, and no unrelated/full history in Eevee-sent context. Show unknown provider-added context/usage honestly; no savings percentage or merged cost total without compatible measured scopes.

## Phase status

Phase 1 remains complete. Visual/UX refinements and Phases 2-7 remain pending. All 27 implementation tasks above are unchecked; writing this plan did not implement them. See `docs/WORK_CHECKPOINT.md` for the exact continuation point and separate documentation/runtime evidence.
