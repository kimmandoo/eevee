# Eevee Design Rules

Status: Phase 1 native terminal shell implemented and verified. Experience and visual refinements below are planned in `task.md`; they are not claims about the current binary.

## Product

Eevee is a local-first terminal workbench for LLM-assisted development. One partner retains the task, evidence, and decisions while specialized forms expose the right tools. It is not another full-screen chat log or nine permanently running agents.

Use Eevee and its eight existing evolutions; do not invent additional species. Species and types follow the original concept. Software responsibilities below are product interpretations, not claims about canonical abilities. Forms are reversible workbench states, not a simulation of the games' evolution restrictions.

### Experience: a partner, not a themed dashboard

- **Sanctuary** is Eevee's home: one active task card, its completion criteria, and one recommended next action. With no task, show one invitation to create a goal, not nine empty specialist dashboards.
- **Evolution** changes the tools for the same task. Keep the task identity, selected evidence, decisions, and user-approved scope across forms. A specialist never starts a second, disconnected conversation by default.
- **Field journal** is the task's inspectable evidence history: observations, decisions, executions, reviews, and handoffs. Cosmetic form selection is not evidence. Opening the journal performs no model call.
- **Bond** means explicitly approved project/personal habits that save repeated instructions. No affection score, XP, hunger, streaks, random unlocks, or inaccessible forms.
- **Evolution Stones** are task recipes, not collectible currency. All nine forms remain locally selectable; recipes do not imply every canonical species evolves through a stone.
- Keep original species names/types and visual cues, but use plain developer verbs for actions. Do not rename a failing command as a successful “attack” or conceal errors behind character dialogue.
- The companion occupies the rail, not the workspace. Personality comes from the actual species artwork, restrained elemental UI accents, and a short factual status line. Never insert unsolicited chat or interrupt execution with an animation.

### First useful journey

1. Open Eevee, create a bounded task, and define what would count as done.
2. Select Vaporeon to inspect relevant code and pin source ranges into a context basket.
3. Select a specialist for the actual problem: Espeon for a design choice, Umbreon for a failure hypothesis, or Flareon for an already-scoped change.
4. Inspect a handoff card before any model request or command: goal, selected context, allowed scope, requested action, and expected result.
5. Let Jolteon run an explicitly approved verification. Glaceon then reviews the patch against the requirements and the observed result.
6. Sylveon prepares a source-linked handoff. Closing/reopening returns to the same task and unresolved next action, not a fresh chat.

This is a route, not a mandatory tour through every form. Skip irrelevant forms; never issue model calls just to fill a progress trail. The smaller local-only milestone is create task -> exit -> resume -> inspect sources; it must be useful before a provider is connected.

## Forms and ownership

| Form | Type | Exclusive responsibility | Main surface | Default capabilities | Exit evidence |
| --- | --- | --- | --- | --- | --- |
| Eevee | Normal | Intake, scope, routing, and resumption | Goal, acceptance criteria, task status, next action | Read task state and repository metadata; propose handoffs | Bounded task and selected specialist |
| Vaporeon | Water | Discovery and context assembly | Source map, references, selected context | Read/search repository and explicitly requested sources | Source-linked findings, constraints, and context manifest |
| Jolteon | Electric | Command execution and runtime verification | Commands, live output, exit status, timing | Run approved commands; no source editing | Command, cwd, code-state identity, output, and exit result |
| Flareon | Fire | Code implementation and fixes | Working diff, editable scope, implementation decisions | Edit approved source paths; request execution from Jolteon | Patch tied to requirements; verification requested |
| Espeon | Psychic | Architecture, planning, and dependency analysis | Decision options, interfaces, ordered plan | Read evidence; edit designated planning artifacts | Chosen approach, tradeoffs, acceptance criteria, dependencies |
| Umbreon | Dark | Failure diagnosis and security assessment | Reproduction, hypotheses, risks, supporting evidence | Read/search; request approved diagnostic runs from Jolteon | Supported root cause or explicit unknown; actionable risk findings |
| Leafeon | Grass | Maintainability, simplification, and measured efficiency improvements | Complexity hotspots, bounded refactor diff, before/after measurements | Read and edit approved refactor scope; request measurements from Jolteon | Behavior-preserving patch and relevant validation/measurement |
| Glaceon | Ice | Independent review and release-readiness assessment | Findings, stale verification, acceptance checklist | Read diff and evidence; no source editing | Findings with evidence; accept, request changes, or inconclusive |
| Sylveon | Fairy | User handoff, documentation, and curated memory | Outcome, remaining risks, memory proposals | Read evidence; edit approved docs; propose durable memories | Evidence-linked handoff and explicitly accepted memories |

### Boundaries

- Eevee routes; Espeon designs. Intake must not silently become an architecture exercise.
- Vaporeon gathers context; Umbreon tests explanations of a failure or risk.
- Flareon changes behavior; Leafeon improves structure or efficiency within an agreed behavioral contract.
- Jolteon records execution, not correctness judgments. Glaceon judges evidence and never silently fixes its own findings.
- Sylveon records verified conclusions, not invented success or inferred user preferences.
- One active form per task by default. Switching forms is local UI/state work and must not trigger a model call by itself.
- Form changes require deliberate selection or acceptance of a recommendation. They never grant additional permissions automatically.
- Shared state does not imply shared prompts: specialist requests contain only relevant evidence and constraints.
- Independence in Glaceon means a separate review context with the requirements, patch, and observed evidence; it does not imply a different model or guaranteed correctness.

### Signature interactions

| Form | Selected object -> deliberate action | Result shown to the user |
| --- | --- | --- |
| Eevee | Task card -> create, resume, edit criteria, choose next form | One current task and a visible next action |
| Vaporeon | File/range -> inspect, pin, exclude | Context basket with provenance and included byte/token estimates |
| Jolteon | Command request -> inspect, approve/deny, cancel | Live output plus actual exit state; never a synthetic success |
| Flareon | Requirement + approved paths -> request a change | Reviewable working diff with pre-existing changes kept distinct |
| Espeon | Design question -> compare alternatives, choose approach | Decision card with constraints and ordered steps |
| Umbreon | Failure -> add hypothesis, request diagnostic run, attach evidence | Supported/refuted/unresolved hypotheses, not confidence theater |
| Leafeon | Bounded hotspot -> compare baseline, refactor, measure | Behavior/measurement comparison with unchanged requirements |
| Glaceon | Change set -> inspect criteria, findings, verification age | Accept / request changes / inconclusive with evidence links |
| Sylveon | Verified task facts -> preview handoff, propose a memory | Editable handoff and separately approved durable knowledge |

An action card distinguishes three operations: local navigation (no model call), a provider request (selected context leaves the process), and a command/write operation (capability check). Display unavailable prerequisites with a reason, not a working-looking button. A running request keeps its owning form; activating another form requires completion or explicit cancellation, never an implicit cancel or second worker.

## Task and evidence model

A task owns its goal, acceptance criteria, current form, step state, decisions, source references, changes, executions, findings, permissions, and handoff. Forms exchange a compact envelope: goal, scope, evidence references, unresolved items, requested action, and completion condition.

Separate observed facts, model inferences, and user decisions. Persist provenance for each. A command result contains cwd, sanitized arguments, start/end time, exit/signal status, output references, and before/after code-state identities. A passed command is not proof of every requirement.

Capture the initial working tree without overwriting user changes. Git metadata and baseline file identities distinguish pre-existing changes from session changes; concurrent external edits remain unattributed rather than falsely assigned to the agent. A code-state identity covers relevant tracked and untracked inputs, not only HEAD. Changes during a run make its association ambiguous; changes after a run make its verification stale. Display unknown coverage explicitly.

Task states: ready, running, awaiting approval, blocked, complete, cancelled. Completion requires the explicit acceptance checklist and visible unresolved risks; do not infer completion from a process exit alone.

## Permissions and recipes

- Capabilities are enforced at the execution boundary, not merely described in prompts.
- Reading, writing, command execution, network access, and durable memory are separate capabilities. An external CLI without enforceable restrictions must be labelled unmanaged; do not present it as sandboxed.
- Approvals show exact arguments or shell text, cwd, affected scope where known, and permission duration. Unknown command effects remain unknown.
- Imported recipes are untrusted configuration. Loading one neither runs commands nor grants permissions.
- Evolution Stones are versioned TOML recipes specifying form workflow, allowed scope, context selection, validation commands, and completion criteria. A recipe cannot widen existing grants.
- Memory is task-local, project-local, or personal. Promotion to project/personal memory requires explicit approval and supports inspection, editing, and deletion.
- Treat terminal output, repository text, and external sources as untrusted content. They cannot modify grants or issue application commands. Strip unsafe terminal control sequences before rendering.

## Visual contract

The visual language is an evolution sanctuary: quiet rounded panels, actual Eevee/evolution artwork in a bounded portrait area, restrained elemental accents, and a luminous-looking active focus without animation-dependent information. Developer evidence remains the visual priority.

| Form | Accent family | UI accent cue |
| --- | --- | --- |
| Eevee | Warm sand | Twin ears / home |
| Vaporeon | Aqua | Fin / flowing line |
| Jolteon | Gold | Angular spark |
| Flareon | Coral | Soft flame |
| Espeon | Lavender | Forehead gem |
| Umbreon | Amber against neutral surfaces | Ring |
| Leafeon | Sage | Leaf |
| Glaceon | Ice blue | Diamond |
| Sylveon | Rose with pale blue detail | Ribbon |

Use actual Eevee and evolution images, not hand-drawn ASCII substitutes. Prefer one consistent set of transparent `official-artwork` PNGs exposed by PokéAPI, with user-provided image files supported. This intentionally supersedes the earlier original-motif direction. Artwork retains its own colors; elemental colors belong to the surrounding UI. Before public distribution, review naming/art rights and avoid affiliation claims; source availability is not rights clearance.

Planned spacious layout at 120x36: one-cell outer margin; three-row header; one-row gap; a 26-row body with a 22-column companion rail, one-column gutter, 64-column workspace, one-column gutter, and 30-column inspector; one-row gap; three-row contextual footer; one-cell bottom margin. Compact layouts reduce spacing before hiding useful content. Header shows task identity, full form name, operational state, and pending approvals; provenance belongs in the inspector, not every pane.

- >=120 columns: three panes. 90-119: hide the inspector behind a toggle. 60-89: single workspace with compact header and drawers. Below 60x18: explicit small-terminal view, preserving cancel/quit access. Layout calculations must never underflow.
- Use rounded Unicode borders when supported; provide an ASCII override. No emoji-dependent alignment or mandatory Nerd Fonts. Measure Unicode display width and handle CJK and combining characters.
- Support dark/light terminals, 256-color fallback, monochrome, and NO_COLOR. Status always includes text or a symbol plus text; never color alone.
- Static images by default. No GIF playback, continuous animation, flashing, or idle redraw loop in the first artwork increment.
- Form picker lists all nine names, roles, current form, and shortcut. Role instructions remain visible without knowing Pokémon lore.
- Keys: Tab cycles panes; arrows move selection; Enter opens/accepts the focused item; Esc backs out; Ctrl+K opens the command palette; ? opens help; q quits outside text entry. Ctrl+C requests cancellation of active work, otherwise exits. Approval is never a side effect of navigating or switching forms.
- Mouse is optional. Selection, scrolling, cancellation, approvals, and form switching must all work by keyboard.

### Visual refinement contract (V01, V02A, V02B, V03)

- **Hierarchy:** task title first, current action second, details on demand. Each fact has one primary home: state in the header, identity in the rail, selected artifact in the workspace, provenance/permissions in the inspector. Do not repeat “backend not connected” or “no permissions” across all panes.
- **Theme tokens:** `canvas`, `surface`, `text`, `muted`, `border`, `accent`, `selection`, and semantic `success`/`warning`/`error`. Centralize true-color, indexed, and monochrome mappings. Active form supplies one accent; its color never overrides an error or implies a successful result.
- **Quiet surfaces:** neutral canvas, modest surface separation, rounded outer panels, one-cell interior horizontal padding where space permits. Avoid nested boxes, heavy full-width separators, faux shadows, gradients, or rainbow decoration. Text/panels remain terminal-native; the companion image may use a supported terminal graphics protocol.
- **Companion portrait:** contain the actual image inside an approximately 18x9-cell area within the 22-column rail, using detected cell pixel dimensions rather than stretching a square into a rectangle. Preserve ears/tails/ribbons, alpha, aspect ratio, and original colors. Compact heights use a smaller thumbnail or the companion drawer; keep name/type/role/action text visible. Character-cell rendering must be derived from the same source image, not an unrelated silhouette.
- **Idle home:** one welcome/status sentence and one truthful CTA. Before persistence ships this remains “Edit session goal”; after E01/E02 it becomes “Create task” / “Resume task.” Never show made-up task counts, runs, timestamps, or preview data in the live app.
- **Working task card:** goal, completion checklist, and one next action. Once real evidence exists, a short handoff trail shows accepted transitions only; it is not a fixed nine-stage completion bar.
- **Evolution picker:** at >=90 columns and >=30 rows show a 3x3 grid of image thumbnails with a selected detail area. Otherwise use a compact list plus the selected image/role where space permits. Selection previews actual species artwork, responsibility, prerequisite status, and expected artifact; Enter changes form only. Digits 1-9 remain shortcuts. A recommendation is labelled as a recommendation and never activates itself.
- **Contextual footer:** at most five primary hints for the focused surface; `?` reveals the full map. Do not advertise irrelevant keys. Approval buttons have explicit labels and neutral initial focus; opening a dialog or pressing Enter to inspect must not execute anything.
- **Operational states:** ready, running, awaiting approval, blocked, complete, cancelled each has text and a distinct marker. Draft/saving/saved/save-failed are separate persistence labels, not task completion states. No progress percentage without a known denominator.
- **Empty/error states:** a short explanation, an actionable next step if available, and optional details. A missing provider is not a runtime failure; a failed command is not an empty state. Never replace useful evidence with a giant mascot or generic apology.
- **Visual acceptance:** capture the actual TUI at 120x36, 100x30, 70x24, and 60x18, plus a below-minimum screen. Check all nine forms, long paths/goals, CJK/combining text, focus, selection, and error/approval dialogs as those features ship. Dark/light, indexed, monochrome, and ASCII must retain readable selected text and the same actions. No overlapping/clipped primary action, phantom counts, duplicate status blocks, or idle redraws.
- `docs/TUI_MOCKUP.svg` is a planned composition reference, not a runtime screenshot or permission to invent live data. Implement using cell-aligned Ratatui widgets and verify the actual terminal; copying SVG pixel coordinates is not acceptance.

### Actual species artwork: sourcing and rendering

- Canonical image mapping: Eevee 133, Vaporeon 134, Jolteon 135, Flareon 136, Espeon 196, Umbreon 197, Leafeon 470, Glaceon 471, Sylveon 700. Names and types remain visible if an image cannot be shown.
- Source candidate: [PokéAPI sprites](https://github.com/PokeAPI/sprites), using the species API's `sprites.other.official-artwork.front_default`. Use the same collection for all nine; do not mix unrelated generations/styles or substitute fan-made pixel sprites without identifying their source.
- [The repository's rights notice](https://raw.githubusercontent.com/PokeAPI/sprites/master/LICENCE.txt) says image contents are copyright The Pokémon Company despite the repository's CC0 notice. Attribution, local caching, or an opt-in download does not itself grant usage rights. Do not bundle franchise image bytes in the binary/repository by default; accept local artwork or an explicit user-initiated source download. Review rights before public/commercial distribution.
- A small asset manifest records species ID, source URL/revision, SHA-256, and attribution. Pin the downloaded set; no whole-repository clone, silent update, runtime hotlink, or network access on each form switch. Cached assets must work offline. No network call is needed just to choose a form.
- Bound file bytes, decoded dimensions/allocation, and cache size before accepting images. First support static PNG only. Decode/downscale away from rendering; cache by asset identity, protocol, cell size, target rectangle, and any required background composition. Re-encode only when those inputs change.
- `ratatui-image` is the planned native renderer; inspect/pin a version compatible with the existing Ratatui release. Its upstream [protocol guidance](https://github.com/ratatui/ratatui-image) covers Kitty, iTerm2, Sixel, and half-block fallback, including protocol-specific cleanup/compatibility caveats. Enable native backends only after actual-terminal acceptance; do not infer compatibility from a terminal brand alone.
- Default `--images auto` uses a known/verified native protocol when possible, otherwise a colored half-block rendering of the same source image. `--images cells` bypasses graphics probing; `--images off` is a supported accessibility/compatibility choice. NO_COLOR/monochrome and `--ascii` use a luminance-derived text rendition with no color image escapes. Missing/corrupt assets show the species name plus an actionable asset state, never pretend an unrelated drawing is the real image.
- Protocol/font-size discovery must be bounded and share terminal-input ownership; never repeat the earlier blocking cursor-query startup failure or let a probe consume user commands. Native rendering must clear the old image on form switch, overlay occlusion, resize, theme/layout change, and exit. Unsupported SSH/tmux paths must retain a working cells/text mode.
- Keep image work separate from LLM context: displaying an avatar never sends image bytes to a model. Use PNG-only decoding features; do not introduce a runtime Chafa dependency or animation framework by default. Neither `TestBackend` nor a text-only PTY parser proves native graphics compatibility.
- The SVG remains a geometry reference with an explicitly labelled real-artwork well. An actual-source image preview may be generated as a local, untracked design artifact; it is not evidence that native terminal rendering is implemented.

## Implementation stack and efficiency

Choose Rust with Ratatui and Crossterm for the native TUI, Tokio for asynchronous I/O and subprocess coordination, Serde for schemas, TOML for recipes, and SQLite through rusqlite for durable task metadata. Store large command output in bounded/rotated local files referenced by metadata. Begin with one Cargo package and explicit modules, not a multi-crate framework.

Rust offers native startup, predictable memory ownership, and no garbage-collector pauses. This is a fit-for-purpose decision, not a claim that any language is universally fastest. Model/network latency will usually dominate agent work; reducing unnecessary context and calls matters alongside rendering efficiency.

- One terminal owner; blocking Git/SQLite/filesystem work stays off the rendering path.
- Bounded worker channels; batch stream updates. Preserve output on disk while virtualizing the visible window; do not grow an in-memory transcript forever.
- Invalidate only changed views; redraw on input/state changes, at most 30 fps during streams, never on an idle timer alone.
- Cache parsing/diff results by source identity; avoid reloading the repository or recomputing all hashes per frame.
- Use direct subprocess arguments by default; shell execution is explicit. Cancellation terminates the owned process group and records partial results.
- Select context by scope and evidence references; do not inject every form prompt or the full historical transcript. Display provider-reported token counts separately from estimates; do not invent savings percentages.
- Use one specialist request at a time initially. No background model chatter, automatic agent fan-out, or model calls for cosmetic transitions.
- Redact credentials before persistence where identifiable; do not promise perfect redaction. Let users exclude sensitive paths and purge task artifacts.

Initial performance acceptance targets, not measurements: idle CPU <1% of one core over 60s; idle RSS <=80 MiB; warm local startup to usable UI p95 <=200 ms over 30 runs; local key-to-render p95 <=50 ms under streamed output. Record hardware, release profile, terminal size, sample method, and workload. Benchmark a 10 MiB output stream and a 10,000-line diff; memory must remain bounded by configured windows/queues. Optimize from measurements, not assumptions.
