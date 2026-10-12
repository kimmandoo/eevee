# Eevee Design Rules

Status: approved direction from the product request; implementation not started.

## Product

Eevee is a local-first terminal workbench for LLM-assisted development. One partner retains the task, evidence, and decisions while specialized forms expose the right tools. It is not another full-screen chat log or nine permanently running agents.

Use Eevee and its eight existing evolutions; do not invent additional species. Species and types follow the original concept. Software responsibilities below are product interpretations, not claims about canonical abilities. Forms are reversible workbench states, not a simulation of the games' evolution restrictions.

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

The visual language is an evolution sanctuary: quiet rounded panels, a compact companion silhouette, restrained elemental accents, and a luminous-looking active focus without animation-dependent information. Developer evidence remains the visual priority.

| Form | Accent family | Motif |
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

Use original small terminal motifs, not downloaded franchise sprites. Before public distribution, review naming/art rights and avoid any claim of affiliation; rights clearance is not implied by this plan.

Layout at 120x36: compact header; 22-column companion/task rail; flexible evidence workspace; 30-column context/approval inspector; short command bar and key hints. Header shows task, full form name, state, and pending approvals. The central workspace changes with the form; chat is an optional drawer.

- >=120 columns: three panes. 90-119: hide the inspector behind a toggle. 60-89: single workspace with compact header and drawers. Below 60x18: explicit small-terminal view, preserving cancel/quit access. Layout calculations must never underflow.
- Use rounded Unicode borders when supported; provide an ASCII override. No emoji-dependent alignment or mandatory Nerd Fonts. Measure Unicode display width and handle CJK and combining characters.
- Support dark/light terminals, 256-color fallback, monochrome, and NO_COLOR. Status always includes text or a symbol plus text; never color alone.
- Optional tiny idle motif, disabled by default. No continuous animation, flashing, or redraw loop when idle.
- Form picker lists all nine names, roles, current form, and shortcut. Role instructions remain visible without knowing Pokémon lore.
- Keys: Tab cycles panes; arrows move selection; Enter opens/accepts the focused item; Esc backs out; Ctrl+K opens the command palette; ? opens help; q quits outside text entry. Ctrl+C requests cancellation of active work, otherwise exits. Approval is never a side effect of navigating or switching forms.
- Mouse is optional. Selection, scrolling, cancellation, approvals, and form switching must all work by keyboard.

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
