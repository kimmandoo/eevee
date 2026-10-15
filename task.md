# Tasks

Open work only. Completed task specs and their evidence were removed on 2026-10-15; they live in git history (`git show fadc6f4:task.md`), `docs/WORK_CHECKPOINT.md` and `CHANGELOG.md`. Contract: [design rules](docs/DESIGN_RULES.md). Architecture: [implementation plan](docs/IMPLEMENTATION_PLAN.md). Adapters: [agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Status

- **Released:** 0.1.2 (tags v0.1.0–v0.1.2). Local core, Codex, OpenCode ACP and Devin routes ship; Schema 6 store.
- **Done:** 25 original tasks (index below). **Open:** 65 tasks in sections A–G. **Dropped:** T05 (per-task budgets/repeat stop, removed at the user's request on 2026-10-14).
- **Blocked:** A04/A06 need permitted Claude API/cloud credentials and completed Gemini authentication; A07 → Q01 → Q02 → P03 inherit that block. Never substitute subscription tokens or fake provider evidence.
- **Partially implemented:** T01 (local receipt), T03 (in-packet dedup), A04/A06/A07 adapters. Their unchecked acceptance is recorded under each task.

### Completed index (referenced by "Builds on")

V01 sanctuary layout · P01 native terminal lifecycle · V02A/V02B species art and native graphics · V03 evolution picker · UX01 focus/editing · E01 durable goal · E02 task cards/resume · E03 task lifecycle/privacy/quota · C01 repository baseline/diff · C02 Vaporeon source map/basket · R01 Jolteon approve/run/cancel · R02 execution journal/freshness · UX02 interruption/recovery · P02 filesystem/process parity · A01 agent preflight · A02 Espeon planning request · A03 Flareon implementation · A05 OpenCode ACP · U01 Umbreon hypotheses · L01 Leafeon refactor comparison · G01 Glaceon review desk · S01 Sylveon handoff · S02 curated memory (Bond) · S03 Evolution Stone recipes.

## How to pick work

- Take the first ready task in **Recommended order**. A task is ready when every ID in **Needs** is done; **Builds on** lists completed tasks to reuse.
- One task = one actual-TUI demo at 120×36 and 60×18, its tests, changelog/checkpoint update and one commit. A mock, build-only change or inactive control is not completion.
- Reuse existing modules (`workflows.rs`, `memory.rs`, `runner.rs`, `context.rs`, `receipt.rs`, `store.rs`); no parallel dispatcher, executor, recipe parser or agent runtime.
- Keep keyboard-only operation, draft/focus preservation, cancel/quit, terminal restoration and honest evidence. Navigation, paste, suggestions and form switches never execute, approve or send anything.
- When strategy-like defaults change (suggestion rules, ranking, learning thresholds), report the observed behavior on fixtures.

### Cross-cutting watch-outs

- **Schema:** The store is Schema 6; PT02 introduces Schema 7 with partner records. Every persisted addition (partners, route events, pairings, dismissals, nickname, moves, dex, receipt identity) needs a migration plus the existing byte-preserving old-binary refusal test. Batch additions that ship together into one schema bump; never rewrite data to let an older binary start.
- **Packets:** Partner flavor (nickname, partner names, move names, dialogue, ribbons, route) never enters provider packets or handoffs. Check the receipt: flavor bytes must be zero.
- **Facts first:** Pokémon-style copy wraps a factual line (command, exit code, permission scope), never replaces it. Monochrome, ASCII and linear layouts keep the same information.
- **No fake progression:** DESIGN_RULES forbids XP, levels, affection scores, streaks, hunger and random unlocks. Raising the partner means learned moves, a nickname and real records only. Update DESIGN_RULES' concept list when a new concept (Moves, nickname, Dex) ships.
- **Rights:** Canonical move names are used as labels. Q01's naming/art-rights review must cover them before the next distribution.
- **Party concurrency:** Partners never start work on their own or chain another partner's work. Every event, approval and journal entry carries its partner; each provider request keeps its own review and receipt, so cooperation never hides cost. One writer per task.
- **Local only:** Detection, ranking and learning use local journal/store data. No optimizer or flavor feature makes a network or model call.

## Recommended order

| Wave | Tasks (in order) | Why this order |
|---|---|---|
| 1. Party foundation | PT01 → PT02 → PT03 → PT04 → PT05 → PT06 | Contract first, then the partner model (Schema 7), free switching, roster, per-partner drafts and request routing. Everything below assumes partners exist. |
| 2. Partner basics | EV04 → EV03 → EV07 → PK01 → EV01 → EV02 | Small UI slices reused later (owning-form redirect, suggestion, carry-over strip, nickname). |
| 3. Form relays | EV10 → EV14 → EV12 → EV13 → EV15 → EV16 → PT10 | Highest daily value; PT10 lets the same relays target a party member. |
| 4. Cooperation | PT08 → PT07 → PT09 | Writer lease before concurrency; the board needs concurrent work to show. |
| 5. Moves | MV01 → MV02 → MV03 → MV04 → MV05 → MV06 → MV07 | MV01 defines per-partner move storage; learning needs the menu; forgetting needs learning. |
| 6. Partner record | EV05 → EV06 → PK02 → EV08 → PK05 → EV09 (after T01) → EV17 → EV11 | Uses evidence identities, partners and moves from earlier waves. |
| 7. Optional flavor | PK03 (after UX04) → PK04 (after C03) → PK06 → PT11 | Polish; drop any item that starts to feel like gamification. |
| Context/token track | C03 → T01 → T02 → T03 → T04 · TC01 → TC03 → TC04 · TC02 → TC05 · D01 · Q03 last | Independent of the waves; can interleave. |
| UX/platform track | UX03, A08 (priority 1) → UX04 → S04 → MV08 → P04 | UX04 unblocks PK03; S04 and MV08 share one export path. |
| Blocked | A04, A06 → A07 → Q01 → Q02 → P03 | Resume only after the user prepares permitted authentication. |

## A — Party: free form switching and cooperating partners

Each Eevee is an individual **partner** (nickname, current form, moves, met date). An idle partner changes form freely at any time, and several partners can work on the same task in different forms, e.g. Mizu (Vaporeon) gathers context while Bolt (Jolteon) runs tests and Aurora (Glaceon) reviews. Today the app has one `form` and one planning/request state per session, so this section first changes the contract, then the model, then routing, and only then concurrency.

### PT01 — Party contract

- [ ] Record the party model in the binding design contract before any code changes.
- **Priority:** 1. **Needs:** —. **Builds on:** —.
- **Change:** Update DESIGN_RULES (lines on "one partner", "one active form per task", running-request ownership) and the README tagline. Proposed decisions to confirm with the user: party of up to **6** partners; up to **3** partners working concurrently per task; exactly **one writer** (Flareon/Leafeon edit lease) per task; each partner owns its own requests, approvals and draft; partners never start work on their own or trigger another partner's work; idle partners cost nothing (still "not nine permanently running agents"); Glaceon review by a different partner than the patch author is preferred and labelled when not.
- **Accept:** DESIGN_RULES and README describe partners, free switching and cooperation limits without contradicting the remaining contract (permissions, evidence, no fake progression). Documentation-only; the user confirms the limits.
- **Watch out:** Until PT06/PT07 ship, the implementation still allows one running request; mark the concurrency rules as planned in DESIGN_RULES' status line.

### PT02 — Partner identity model

- [ ] Replace the single session form with stored partner records.
- **Priority:** 1. **Needs:** PT01. **Builds on:** E02, V03.
- **Change:** `partners` table (id, nickname, current form, met date, created project). Migrate the existing state as partner #1 with its current form; a task records its lead partner. Header shows the lead partner's form; behavior is otherwise unchanged.
- **Accept:** Upgrade a Schema 6 store: one partner exists in the previous form, every task resumes unchanged, an old binary refuses the new store without changing bytes.
- **Watch out:** This is the Schema 7 bump. Design the table so PK01, MV01, EV06 and EV17 add columns/tables in the same migration where they ship together. Replace `App.form: usize` through one accessor so later tasks touch one place.

### PT03 — Free form switching

- [ ] Change an idle partner's form instantly.
- **Priority:** 1. **Needs:** PT02. **Builds on:** V03, UX01.
- **Change:** Picker Enter switches immediately (no confirmation dialog). Add named commands (`/jolteon`, `/vaporeon`, …) and a `previous form` toggle. Switching preserves task, draft, focus and context. A working partner cannot change form (Stay or cancel-and-wait), but other partners stay switchable.
- **Accept:** Cycle Vaporeon → Flareon → Jolteon → previous → previous in under a second of keystrokes; stored task/draft/context are byte-identical; no request, approval or grant appears.
- **Watch out:** Numeric shortcuts stay removed (DESIGN_RULES). Free switching must not redraw native images repeatedly; clear and redraw once per switch.

### PT04 — Party roster

- [ ] See, add and release partners.
- **Priority:** 1. **Needs:** PT02. **Builds on:** UX01.
- **Change:** `/party` lists partners with nickname, current form, status (idle / working / waiting for approval) and current task. Add a new Eevee (up to the PT01 limit) starting in Eevee form; release a partner with a preview of what is deleted (its moves, pairings) and what stays (evidence, tasks).
- **Accept:** Add two partners, restart, see three; the limit refuses a seventh with a reason; releasing an idle partner removes it and its moves; a working partner cannot be released.
- **Watch out:** Evidence a released partner produced keeps its partner attribution as a name snapshot, never a dangling id.

### PT05 — Lead switch and per-partner drafts

- [ ] Bring a different partner to the front of a task.
- **Priority:** 1. **Needs:** PT04. **Builds on:** UX01, E01.
- **Change:** Choose the lead partner for the current task from the roster or `/lead <nickname>`. Drafts become per (task, partner); the composer shows whose draft it is. Existing drafts migrate to the lead partner.
- **Accept:** Write different drafts for two partners on one task, switch leads and restart; each draft returns to its partner exactly, including CJK and caret position.
- **Watch out:** Dirty-draft warnings and paste safety (never execute) apply per draft.

### PT06 — Route requests by partner

- [ ] Make every request, event and approval belong to a partner.
- **Priority:** 1. **Needs:** PT05. **Builds on:** A02, A03, R01, R02.
- **Change:** Replace the single planning/request state with a per-partner slot, still limited to one working partner per task. Journal entries, provider events, questions and approvals carry (task, partner, request id); approval dialogs name the partner: `Bolt (Jolteon) wants to run: cargo test`.
- **Accept:** Start a request with one partner, switch the lead and back; events and the pending approval land on the original partner only. Late or stale events for a released/other partner are refused. All existing provider/run tests pass unchanged.
- **Watch out:** Pure refactor plus labels; no concurrency yet. This is the riskiest step for regressions, so keep the existing approval tests as the gate.

### PT07 — Cooperating partners

- [ ] Let up to the PT01 limit of partners work on one task at the same time.
- **Priority:** 2. **Needs:** PT06, PT08. **Builds on:** R02, UX02.
- **Change:** Allow several per-partner slots to run concurrently. The header shows working partners; each approval is queued with its partner name and is answered one at a time. `Cancel` targets the lead's work; `Cancel all` lists every working partner first. Quit with working partners lists them all.
- **Accept:** Mizu (Vaporeon) runs a provider planning request while Bolt (Jolteon) runs an approved command; both finish with correct attribution; cancelling one leaves the other running; a fourth partner is refused with a reason.
- **Watch out:** Each provider request still has its own review and receipt; concurrent cost is shown per partner, never hidden. Bound output/journal memory per slot. Measure in Q02.

### PT08 — Single-writer edit lease

- [ ] Prevent cooperating partners from editing the same task concurrently.
- **Priority:** 2. **Needs:** PT06. **Builds on:** A03, L01.
- **Change:** Edit-capable work (Flareon, Leafeon) takes a per-task write lease shown in the header. Another partner's edit request shows `Bolt (Flareon) is editing` and waits or cancels. A run started while the lease is held records `revision moving` instead of a stable revision.
- **Accept:** Two Flareon partners request edits: the second waits and starts nothing; a Jolteon run during the edit is marked `revision moving` and is not offered as fresh verification.
- **Watch out:** The lease is released on completion, cancellation and crash recovery; never leave a stale lease that blocks the task forever.

### PT09 — Party board

- [ ] See what every partner on the task is doing at a glance.
- **Priority:** 2. **Needs:** PT07. **Builds on:** UX02.
- **Change:** A compact party HUD (one line per working/waiting partner: sprite, nickname, form, status, last event) above the composer, collapsing into a count at 60×18. Selecting a line focuses that partner.
- **Accept:** With three working partners, each line updates from its own events only; at 60×18 the count opens the full board.
- **Watch out:** No idle redraw timer; no HP bars or fake progress.

### PT10 — Hand work to another partner

- [ ] Pass a result to a party member instead of switching your own form.
- **Priority:** 2. **Needs:** PT05. **Builds on:** S01, G01.
- **Change:** Relays (EV12, EV13, EV14, EV15) offer both `Switch to <form>` and `Send to <partner> (<form>)` when an idle partner is in the target form. Sending pre-fills that partner's draft with references; nothing is sent to a provider.
- **Accept:** Send a supported hypothesis to an idle Flareon partner; its draft cites the evidence; Umbreon's partner and state are unchanged. A Glaceon review requested from the patch author's own partner is labelled `same partner`.
- **Watch out:** A handoff transfers references, not approvals or provider sessions.

### PT11 — Formations

- [ ] Save and reuse a party arrangement for a kind of task.
- **Priority:** 3. **Needs:** PT07. **Builds on:** S03.
- **Change:** Extend the S03 recipe schema with optional role slots (scout: Vaporeon, runner: Jolteon, reviewer: Glaceon); applying a recipe maps slots to party members and sets their forms. Plan-only: nothing runs.
- **Accept:** Apply a formation to a task with three partners; forms are set and drafts pre-filled; no request starts. Missing partners show which slot is empty.
- **Watch out:** Reuse the recipe parser and schema version rules; a formation never grants permissions.

## B — Partner basics and form suggestion

These tasks make each switch informed and each form immediately useful, for the lead partner and for party members. All are local UI/state work.

### EV04 — Form-specific empty states

- [ ] Give each form one obvious first action when its surface is empty.
- **Priority:** 1. **Needs:** —. **Builds on:** V01, UX01.
- **Change:** Show the form's `object → action` line from the DESIGN_RULES interaction table plus one primary action that opens the existing flow (Vaporeon `@` attach, Jolteon `/run` review, Umbreon add hypothesis, Glaceon review desk, Sylveon handoff preview, Eevee edit criteria).
- **Accept:** Open all nine forms on an empty task at both sizes; each primary action opens its review/flow and executes nothing. Non-empty surfaces are unchanged.
- **Watch out:** Concise-copy rule: one line, one action. No placeholder counts or sample results.

### EV03 — Form capability chip and owning-form redirect

- [ ] Show what the active form may do and route out-of-role actions to their owner.
- **Priority:** 1. **Needs:** —. **Builds on:** UX01, R01, A03.
- **Change:** Header chip with the active form's default capabilities from the role table (`read · run` for Jolteon, `read` for Glaceon). An out-of-role action shows a disabled reason naming the owning form and offers `Switch to <form>`, plus `Send to <partner>` when an idle party member is already in that form (after PT10).
- **Accept:** From Glaceon request an edit and from Espeon request a command: each names the owner and offers a working switch path without starting the action.
- **Watch out:** The chip is descriptive. Enforcement stays in the existing approval flow; never widen or bypass a grant because a form is active. Header must still fit project/form names at 60×18.

### EV07 — Return to Eevee

- [ ] Step a partner back to intake from any form in one action.
- **Priority:** 1. **Needs:** PT03. **Builds on:** V03.
- **Change:** `/eevee` and a top picker entry return the lead partner (if idle) to Eevee with task, draft, focus and context unchanged. Show EV01's suggestion once EV01 exists.
- **Accept:** Return from Flareon, Umbreon and Sylveon; stored state is byte-identical before/after.
- **Watch out:** During a running request, offer Stay or explicit cancel-and-wait (running-ownership rule). Never cancel implicitly or transfer the request.

### PK01 — Partner nickname

- [ ] Let the user name each Eevee in the party.
- **Priority:** 1. **Needs:** PT02. **Builds on:** UX01, E02.
- **Change:** Personal-scope nickname (≤12 graphemes, Unicode) set from the partner card or `/nickname`; header and dialogue show `Nickname (Species)`, e.g. `Bolt (Jolteon)`. Clearing restores `Eevee #n`. Nicknames are unique within the party so approvals and the board stay unambiguous.
- **Accept:** Set a CJK nickname, restart, see it in header/dialogue in every form at both sizes; species name and type stay visible. The receipt shows zero nickname bytes in an assembled provider packet.
- **Watch out:** Column on PT02's partners table, shipped in the same Schema 7 migration if possible. Width must use grapheme/cell width, not byte length.

### EV01 — Evidence-based form suggestion

- [ ] Suggest the form that fits the task's current evidence, with the fact that triggered it.
- **Priority:** 1. **Needs:** —. **Builds on:** V03, E02, R02, U01, G01.
- **Change:** Deterministic rules, first match wins: goal without criteria → Espeon; criteria without pinned context → Vaporeon; latest owned run failed → Umbreon; working diff without a run at its revision → Jolteon; passing current run without review → Glaceon; accepted review → Sylveon. Show in the task card and picker: `Suggested: Umbreon — cargo test exited 101`. When an idle party member is already in that form, name it: `Mizu (Umbreon) is free`.
- **Accept:** Seed each state in a fixture task and observe the expected suggestion and reason; no match shows nothing. Editing a source after a passing run changes the suggestion. Dismiss hides it until the triggering evidence identity changes.
- **Watch out:** Report the fixture results when rules change. A suggestion never self-activates (Enter in the picker previews only). Keep rules in one table so EV08/EV07 reuse them.

### EV02 — Evolution carry-over strip

- [ ] Show what stayed and what changed right after a free switch, without slowing it down.
- **Priority:** 1. **Needs:** PT03. **Builds on:** C02, UX01.
- **Change:** After any switch, a one-line non-blocking strip shows carried state (criteria met/total, pinned captures, open hypotheses/findings) and the capability diff (`+edit −run`). It fades on the next keystroke and is reopenable from the partner card. No confirmation step.
- **Accept:** Vaporeon→Flareon, Flareon→Jolteon, Glaceon→Flareon show counts equal to stored state and the correct diff; typing immediately after a switch goes to the draft unchanged. Added/removed capabilities are distinguishable in monochrome/ASCII; fits 60×18.
- **Watch out:** It must never steal focus or swallow keys; free switching (PT03) is the point. Relays (EV12/EV13), held items (PK04) and the transition (PK03) reuse its data, so keep it data-driven.

## C — Form relays

One form's stored output pre-fills the next form's reviewed input. A relay only pre-fills: approval, execution and sending still go through the normal review, and every pre-filled item shows its source.

### EV10 — Pin a failure location from Jolteon output

- [ ] Turn a `file:line` in a failed run into pinned context in one action.
- **Priority:** 1. **Needs:** —. **Builds on:** R02, C02.
- **Change:** Locally parse `path:line[:col]` from retained run output (rustc/cargo, Python traceback, Node/TypeScript, generic). Selecting one offers `Pin range` with a bounded window, reusing capture identity and exclusion checks; the capture records the run as provenance.
- **Accept:** From a real failing `cargo test` and a Python traceback, pin the reported locations; the basket shows the run reference. Out-of-root, excluded and missing paths are refused with a reason; unparseable output offers nothing.
- **Watch out:** Put the parser in its own small function with fixture tests; PK05 reuses it. Retained output may be truncated (only 1 MiB kept); parse only what is retained and say so.

### EV14 — Rerun a stale check at the current revision

- [ ] Refresh outdated verification with one reviewed action.
- **Priority:** 1. **Needs:** —. **Builds on:** R01, R02.
- **Change:** Wherever a run is marked stale (task card, Glaceon criterion, later EV05), offer `Rerun with Jolteon`, opening the normal command review pre-filled with the same argv/cwd at the current code-state identity. The result is a new run linked to the old one.
- **Accept:** Edit a source after a passing run, rerun from the marker and approve; both runs remain with distinct revisions. Denying starts nothing; an argv edited in review is shown before approval.
- **Watch out:** Never update or relabel the old run. A previous approval is not reused.

### EV12 — Carry a supported hypothesis into a Flareon fix

- [ ] Start a fix from Umbreon's conclusion instead of restating it.
- **Priority:** 1. **Needs:** EV02. **Builds on:** U01, A03.
- **Change:** On a supported hypothesis, `Fix with Flareon` switches the lead partner to Flareon (or, after PT10, sends to an idle Flareon partner), shows the EV02 strip, and pre-fills the requirement with the hypothesis and evidence references and proposes editable paths from linked ranges.
- **Accept:** The Flareon draft cites the hypothesis and evidence; editable paths still need normal approval; cancel leaves Umbreon state unchanged; unresolved/refuted hypotheses show a disabled reason.
- **Watch out:** Proposed paths go through A03's limits (1–8 files ≤12 KiB, POSIX only for controlled writes). Show the limit instead of trimming silently.

### EV13 — Fix Glaceon findings and re-review

- [ ] Close the review loop finding by finding.
- **Priority:** 1. **Needs:** EV02. **Builds on:** G01, A03.
- **Change:** From a request-changes finding, `Fix with Flareon` switches or sends to a Flareon partner and pre-fills requirement/scope. A later diff touching the finding's ranges marks it `addressed, needs re-review`, makes the verdict stale and offers returning to Glaceon with a fresh review context.
- **Accept:** Relay two findings and change one; only that one becomes `addressed, needs re-review`. No finding closes without a new Glaceon verdict.
- **Watch out:** The re-review context must stay independent of the implementation conversation (G01 contract); with a party, prefer a Glaceon partner that did not author the patch.

### EV15 — Turn an Espeon decision into acceptance criteria

- [ ] Make a chosen design immediately checkable.
- **Priority:** 2. **Needs:** —. **Builds on:** A02, E01.
- **Change:** `Add as criteria` on a decision card lists its acceptance items as editable criteria linked to the decision; duplicates are flagged, not added twice; nothing changes until confirmed.
- **Accept:** Add three, edit one, restart; criteria persist with the decision link and the partner card's met/total updates. Cancel adds nothing.
- **Watch out:** Provider-proposed text stays labelled unverified until the user confirms it as a criterion.

### EV16 — Leafeon baseline from an existing run

- [ ] Reuse a run as the "before" side of a refactor comparison.
- **Priority:** 2. **Needs:** —. **Builds on:** L01, R02.
- **Change:** When starting a comparison, offer recent passing runs at the pre-refactor revision with matching argv, showing age and revision. Choosing none falls back to the fresh-baseline flow.
- **Accept:** Pick an existing run, refactor, compare with a new run of the same argv; the comparison names both runs. Mismatched revision/argv is excluded with a reason.
- **Watch out:** Timing from different machine load is noisy; show both durations and never a single speedup claim from one sample.

## D — Moves: teaching and learning skills

Raising your own partners means each one learns your repeated work as named **moves**. Moves belong to the individual partner, four per form: Bolt's Jolteon moves return whenever Bolt becomes Jolteon again, and another partner does not know them until taught (MV08). A move is a single-form reusable action; an Evolution Stone (S03 recipe) remains a whole-task workflow. Moves reuse the S03 recipe step types and validation, carry no grants, and every use goes through the normal review.

| Form | Move kind | Example default name |
|---|---|---|
| Eevee | task template (goal + criteria) | Helping Hand |
| Vaporeon | pin set (paths/ranges/globs resolved at use) | Surf |
| Jolteon | run (exact argv + project-relative cwd) | Thunderbolt |
| Umbreon, Leafeon | run through Jolteon's review (diagnostic/measurement) | Foul Play, Leaf Blade |
| Espeon, Flareon, Glaceon, Sylveon | request template (instruction text + context selectors) | Future Sight, Flamethrower, Ice Beam, Moonblast |

### MV01 — Move model and teaching a move

- [ ] Let the user teach a partner's current form a named move from a reviewed action.
- **Priority:** 1. **Needs:** PT02. **Builds on:** S03, R02, C02, S02.
- **Change:** Store moves (partner, name, form, kind, steps, scope project/personal, provenance, created date) with at most **four per partner per form** in the visible scope. After a reviewed action (approved run, pin set, sent request), offer `Teach as move`, preview the exact steps and save. Picker detail lists the form's moves. Add Moves to DESIGN_RULES' concept list.
- **Accept:** Teach a Jolteon run and a Vaporeon pin set to one partner, restart, see both in picker detail; switch that partner away and back to Jolteon and the move is still there; another partner does not see it. A fifth move is refused with a reason until MV04 exists. Project moves are invisible in another project.
- **Watch out:** Schema 7 migration. Reuse the `Recipe`/`RecipeStep` parser and validation; no second format. Steps must be exact argv (never shell text) and project-relative paths; refuse secrets (reuse `privacy.rs`) and out-of-root paths.

### MV02 — Move menu

- [ ] Use a learned move from a battle-style menu.
- **Priority:** 1. **Needs:** MV01. **Builds on:** UX01, R01.
- **Change:** `/move` and a form-surface key open a 2×2 menu (list below 90 columns) showing name, type accent, kind and last outcome. Choosing opens the normal review pre-filled (command review, pin preview, request preview).
- **Accept:** Use a Jolteon and a Vaporeon move; the review shows exact argv/paths and nothing runs until approved. Escape restores the draft and focus. Pin moves whose files vanished show them as unavailable in the preview.
- **Watch out:** A move is never pre-approved, even when used repeatedly. Request moves still show the full receipt.

### MV03 — Learn a move from repetition

- [ ] Notice repeated work and offer to learn it.
- **Priority:** 1. **Needs:** MV01, MV02. **Builds on:** R02.
- **Change:** Locally detect the same normalized argv+cwd approved ≥3 times by the same partner in a project, or the same pin set used ≥3 times. Show `Jolteon wants to learn a move!` with the exact steps and the three sources: Learn (opens naming), Not now, Never for this.
- **Accept:** Run `cargo test --workspace` three times in a fixture project; the prompt appears once with sources. Different arguments do not merge. `Never` survives restart; `Not now` returns after three more uses.
- **Watch out:** Report the threshold behavior on fixtures. Never auto-learn. Exclude argv with secrets, env assignments or absolute paths outside the project. Prompt only at idle boundaries, never during a review or running request.

### MV04 — Forget a move when slots are full

- [ ] Choose which move to forget when learning a fifth.
- **Priority:** 2. **Needs:** MV03. **Builds on:** —.
- **Change:** Learning into a full form shows the four moves and the new one: forget one, or `Give up learning`. Forgotten moves go to the reminder list (MV05).
- **Accept:** Fill four slots, learn a fifth, forget one; the forgotten move leaves the menu and appears in the reminder list after restart. Giving up changes nothing.
- **Watch out:** Personal and project moves share the visible four; say which scope a forgotten move came from.

### MV05 — Move Reminder

- [ ] Relearn a forgotten or postponed move.
- **Priority:** 2. **Needs:** MV04. **Builds on:** E03.
- **Change:** A reminder list per form with forgotten and `Not now` moves; relearning needs a free slot (or MV04's forget flow). Purge removes reminder entries with their project.
- **Accept:** Relearn a forgotten move with identical steps; purge a project and its reminders disappear while personal ones stay.
- **Watch out:** Revalidate steps on relearn (paths may no longer exist).

### MV06 — Type-themed move names

- [ ] Suggest a fitting Pokémon move name and allow renaming.
- **Priority:** 2. **Needs:** MV01. **Builds on:** —.
- **Change:** A fixed local list of canonical moves per type (Electric: Thunderbolt, Thunder Shock, Discharge, Spark; Water: Surf, Hydro Pump, Aqua Ring; Psychic: Future Sight, Calm Mind, Psychic; and so on). Suggest by meaning where a mapping exists (test run → Thunderbolt, planning → Future Sight, review → Ice Beam), otherwise the first unused. Users rename freely; names are unique per form.
- **Accept:** Teach three moves on Jolteon and get three distinct suggestions; rename one to a CJK name; a duplicate name is refused.
- **Watch out:** Names are labels only; they never change behavior and never enter provider packets. Include the list in Q01's naming-rights review.

### MV07 — Move record

- [ ] Show how a move has actually performed.
- **Priority:** 2. **Needs:** MV02. **Builds on:** R02.
- **Change:** Move detail shows uses, last result and revision, and observed outcomes as a count (`7/9 exited 0`). No power, accuracy percentage or level.
- **Accept:** Use a run move with passing and failing outcomes; counts match the journal and survive restart. A request move shows uses and last sent date only.
- **Watch out:** Exit 0 is not correctness. Label counts as observed; denied/cancelled uses are listed separately, not as failures.

### MV08 — TM export and import

- [ ] Share moves between party members, projects and people.
- **Priority:** 2. **Needs:** MV01, S04. **Builds on:** S03.
- **Change:** Teach a move directly to another party member (choose partner and slot), or export it as a create-only TOML "TM" through S04's export path; import previews steps, target partner, form and slot, then teaches it.
- **Accept:** Teach Bolt's move to Mizu; it appears in Mizu's moves with provenance. Export from one project, import into another; using it still requires review. Newer schema, out-of-root paths and secrets are refused; an existing file is never overwritten.
- **Watch out:** Imported moves carry no approvals and never execute on import.

## E — Partner record and flavor

### EV05 — Party status from real exit evidence

- [ ] Show in the picker what each form has actually produced for this task.
- **Priority:** 2. **Needs:** —. **Builds on:** V03, R02, G01, S01.
- **Change:** Mark each form's exit artifact as `produced`, `stale` or `none` with one fact and the partner that produced it (Vaporeon `4 captures`, Jolteon `exit 0 at a1b2c3, stale`, Glaceon `changes requested`). Derived from stored evidence identities only.
- **Accept:** Produce then invalidate artifacts for Vaporeon, Jolteon and Glaceon; markers follow after restart. Unused forms show `none`, not failure. Readable in monochrome/ASCII and the compact list.
- **Watch out:** No HP, levels or completion percentages.

### EV06 — Form route in the field journal

- [ ] Record how each partner moved between forms without treating switches as evidence.
- **Priority:** 2. **Needs:** EV05, PT02. **Builds on:** E02, R02.
- **Change:** Store switches as route events (partner, from, to, time, accepted suggestion, optional reason). Task card shows one route per partner, e.g. `Bolt: Eevee → Flareon → Jolteon`, dimming visits without an exit artifact. Resume restores each partner's last form.
- **Accept:** Switch four times, restart, resume in the last form with the same route. Journal labels route events as non-evidence; provider packets and Sylveon handoffs contain none. Purge removes them.
- **Watch out:** Schema change; batch with other wave-4 persistence.

### PK02 — Partner summary screen

- [ ] A Pokémon-style summary page built from real records.
- **Priority:** 2. **Needs:** PK01, MV01, EV05. **Builds on:** S02.
- **Change:** From the partner card or roster, per partner: nickname, current form/type, `Met on <date> in <project>` (PT02's met date), moves per form, approved Bond memories count, and per-form task counts with produced exit artifacts.
- **Accept:** Numbers equal store queries; a fresh install shows the met date and `No moves yet`. Fits 60×18 via paging.
- **Watch out:** No level, XP or invented stats. The migrated first partner's met date is the earliest task date, labelled as derived.

### EV08 — Partner lines for real events

- [ ] Eevee reacts in the dialogue box to actual events.
- **Priority:** 2. **Needs:** EV01. **Builds on:** V01, R02.
- **Change:** For a fixed set (owned run passed/failed, review verdict, handoff ready, source changed after verification, move used) show one deterministic line in game style with the fact and suggestion: `Bolt used Thunderbolt! cargo test failed (exit 101). Umbreon can investigate.` A persistent preference turns lines off.
- **Accept:** Each event shows exactly one line; journal replay or restart never repeats it. Lines off changes nothing else. Permission/error facts are never hidden; lines stay in the composer area at 60×18.
- **Watch out:** No randomness, praise or emotion. Uses the acting partner's nickname (not the lead's) and the move name when a move was used.

### PK05 — Bug Dex

- [ ] Remember failures you have met before and how they were resolved.
- **Priority:** 2. **Needs:** EV10. **Builds on:** R02, U01, A03.
- **Change:** Locally derive signatures from owned failing runs (rustc error code + location, test name, panic location) and keep per-project entries shared by the whole party: first seen, seen count, linked hypothesis/fix/run that resolved it. When a known signature reappears, show `Seen before: resolved in task #12 (hypothesis …)`.
- **Accept:** Fail, fix and fail the same test again in a fixture; the second failure links the first resolution. Another project never sees the entry. Purge removes it.
- **Watch out:** A past resolution is a lead, not a current fix or verification. Signatures go through redaction; never store full output twice. Schema change.

### EV09 — Learn a habit from repeated instructions

- [ ] Offer to save an instruction repeated across requests as an approved Bond memory.
- **Priority:** 2. **Needs:** T01. **Builds on:** S02.
- **Change:** Detect identical normalized instruction text in ≥3 requests in a project; Sylveon proposes it as project or personal memory with its sources, through the existing S02 approval. Dismissal is remembered per text hash.
- **Accept:** Three recorded planning requests with the same instruction produce one proposal; approving it makes the next receipt show the memory instead of repeated text; a dismissed proposal does not return.
- **Watch out:** Never infer or save preferences automatically. Secret-filtered or excluded text never becomes a proposal. Moves (MV03) cover actions; this covers instructions.

### EV17 — Per-partner provider pairing

- [ ] Give each partner its own remembered agent/model per form.
- **Priority:** 2. **Needs:** PT02. **Builds on:** A01, A02, V03.
- **Change:** After choosing an agent/model, offer `Use for <partner> as <form> in this project`; later switches preselect it labelled remembered. Cooperating partners can therefore use different agents (Bolt on Codex, Mizu on OpenCode). Unavailable pairings show the blocker and fall back to the task selection.
- **Accept:** Pair two partners' Glaceon and Flareon forms with different agents; switching preselects each and the review shows it. Removing the pairing restores the default; a disconnected agent is labelled unavailable, never replaced.
- **Watch out:** Selection never connects, sends, or changes billing/authentication.

### EV11 — Rank `@` candidates by the active form

- [ ] Show the most relevant files first when attaching.
- **Priority:** 2. **Needs:** EV10. **Builds on:** C02.
- **Change:** Same picker; order by form (Umbreon: latest failure locations; Flareon/Leafeon: working diff; Glaceon: changed files and their tests) with a short reason. Typed queries return the same match set in every form.
- **Accept:** With a failing run and a diff, top entries differ between Umbreon and Flareon and show reasons. Excluded files never appear.
- **Watch out:** Report ranking on the fixture when the rule changes. Ranking is local; no index rebuild on every keystroke.

### PK03 — Evolution transition

- [ ] A short evolution moment when a form switch is confirmed.
- **Priority:** 3. **Needs:** EV02, UX04. **Builds on:** V02B.
- **Change:** On a picker switch only (never on `/form` commands or the previous-form toggle), a ≤300 ms silhouette flash from old to new sprite with `What? Bolt is evolving!` → `Bolt became Jolteon!`; text-only routes show the final line only.
- **Accept:** Rapid switching never queues animations; any key skips it; typed input during it lands in the draft unchanged. Reduced motion shows no frames. No idle redraw after it ends.
- **Watch out:** Forms are reversible (DESIGN_RULES); returning to Eevee uses neutral copy, not "de-evolution". Clear native images behind overlays.

### PK04 — Held items

- [ ] Let a form always bring a chosen file or memory.
- **Priority:** 3. **Needs:** C03, EV02. **Builds on:** S02.
- **Change:** Assign one held item per partner per form per project (e.g. Glaceon holds `CONTRIBUTING.md`). Entering the form proposes it as an attachment chip, shown in the carry-over card and receipt.
- **Accept:** Glaceon's held file appears as a removable chip after switching and is revalidated before send; an excluded file cannot be held.
- **Watch out:** Proposed, never silently included; its bytes count in the receipt.

### PK06 — Ribbons

- [ ] Commemorate real first milestones.
- **Priority:** 3. **Needs:** PK02. **Builds on:** G01, S01.
- **Change:** One-time, evidence-linked ribbons per project, attributed to the partner that produced the evidence and shown on its summary (first accepted review, first verified handoff, first learned move). Hideable.
- **Accept:** Each ribbon links its evidence; deleting the evidence by purge removes it.
- **Watch out:** Drop this task if it reads as gamification: no counts, no streaks, nothing gated behind a ribbon.

## F — Context, token economy and tool calling

Local deterministic optimizer only: no optimizer-initiated network, hosted summarization, embeddings, telemetry or tokenizer downloads. Distinguish exact Eevee-sent bytes, labelled estimates, provider-reported usage and billing; unknown stays unknown. Tool discovery/paging/caching applies only to documented adapter capabilities and Eevee-owned execution.

### C03 — Inspectable attachment chips and context budget

- [ ] Show exactly which files/ranges accompany a request.
- **Priority:** 1. **Needs:** —. **Builds on:** C02, A02.
- **Change:** Removable file/range chips on `@` attachments, selected-content preview, stale/unavailable markers, exact bytes plus labelled token estimate. Reuse excerpt identities and dispatch revalidation.
- **Accept:** Attach a file and a range, inspect/remove by keyboard, restart and recover. Source changes go stale before send; missing captures block assembly; displayed bytes equal the serialized request. Long Unicode names keep remove/inspect visible.
- **Watch out:** Never imply control over provider-added reads/history. Excluded secrets and out-of-root links never become chips.

### T01 — Per-turn context and usage receipts

- [ ] Explain what each turn sends and where its tokens/cost went.
- **Priority:** 1. **Needs:** C03. **Builds on:** A02, R02.
- **Change:** Receipt split by instructions, task facts, attachments, history, tool schemas, tool results; record provider usage (input/output/cache/reasoning) without double-counting deltas/cumulatives; name model, tokenizer and price-table version.
- **Accept:** Reconcile two real turns and a resume against provider usage events; repeated events count once; before/after selection comparison; receipt identity survives restart; purge respects E03.
- **Status:** Done in 0.1.1: local offline receipt per preview with exact bytes per section and `eevee-local-v1` estimate. Missing: real usage reconciliation, before/after comparison, durable identity.
- **Watch out:** Unavailable prices produce no invented cost; opaque provider input is not attributed to selected files.

### T02 — Budgeted, symbol-aware context compiler

- [ ] Assemble the smallest useful reviewable source packet.
- **Priority:** 1. **Needs:** T01. **Builds on:** C02.
- **Change:** From goal, selected symbols, working diff and pins, prefer definitions, signatures, direct references and relevant tests over whole files; text-only fallback is labelled. Rank, dedup and show reasons/omissions under a user budget; overflow needs a decision.
- **Accept:** In a multi-file bug fixture, include defect, caller and check with fewer tokens than the full-file baseline (one named tokenizer); a seeded cross-file requirement stays discoverable; editing a source after preview stops dispatch for review.
- **Watch out:** Never drop mandatory instructions or pins silently; never fabricate symbol knowledge for unsupported syntax. Quality is measured in Q03, not assumed.

### T03 — Local request deduplication and revision cache

- [ ] Avoid sending the same evidence twice.
- **Priority:** 2. **Needs:** T01, T02. **Builds on:** C02.
- **Change:** Identity/revision index across attachments, memory, handoff facts and tool-result references; emit once with citations; cache assembly/tokenization by content+config identity.
- **Accept:** One capture via three entry points: fewer serialized tokens, every citation resolvable; distinct sources with identical text not conflated; changed source/policy/tokenizer invalidates; zero network access.
- **Status:** Done 2026-10-15: in-packet `same_bytes_as` dedup, hoisted `source_root`, Codex baseline citation, receipt disclosure. Missing: cross-entry-point cache, persisted revision index, three-entry-point harness.
- **Watch out:** A local cache hit never implies provider retention or a billing discount.

### T04 — Evidence-linked compaction and rehydration

- [ ] Continue long work with a compact ledger.
- **Priority:** 2. **Needs:** T01. **Builds on:** C02, E02, R02.
- **Change:** Deterministic local packet of requirements, approved decisions, references, unresolved hypotheses and next action; inferences separate from observations; preview before any approved request.
- **Accept:** Offline resume with conflicting decisions, a failed check and a changed source keeps the latest decision and the failure; references resolve or say unavailable; cancel changes nothing; zero network; measure processing cost and packet size.
- **Watch out:** No model summarization. An old approval is never a new grant. Local compaction cannot erase provider history.

### TC01 — On-demand tool catalog and scoped MCP discovery

- [ ] Expose only relevant tool definitions.
- **Priority:** 1. **Needs:** —. **Builds on:** A01, A02, C02.
- **Change:** Search locally stored metadata of explicitly trusted tools; load exact versioned schemas for supported adapters; catalog refresh is a separate reviewed action.
- **Accept:** 100-tool controlled catalog: intended tool found with fewer schema tokens; collisions, changed schemas, malicious descriptions and a disconnected server handled without execution; unsupported providers say lazy exposure is unavailable; one real compatible round trip.
- **Watch out:** Relevance is not authorization; a read-only hint is not a sandbox; no hidden fetch for missing schemas.

### TC03 — Searchable, paged tool-result artifacts

- [ ] Let the model request the relevant slice of output.
- **Priority:** 1. **Needs:** TC01. **Builds on:** E03, R02.
- **Change:** Immutable artifact identities, exit summary, total/captured/omitted sizes, preview, exact range/search retrieval; inherits isolation/redaction/quota/purge.
- **Accept:** 10 MiB real log with errors outside the preview retrieved by search with much less input; quota-limited run reports discarded bytes; wrong-task/purged/stale handles refused; instruction-like output cannot approve calls.
- **Watch out:** No unbounded in-memory transcript. Shares the location parser with EV10.

### TC04 — Dependency-validated observation cache

- [ ] Reuse unchanged read observations honestly.
- **Priority:** 2. **Needs:** TC03. **Builds on:** C02.
- **Change:** Cache allowlisted reads/searches keyed by operation version, args, project and content/config fingerprints; label hits with original time.
- **Accept:** Repeat a query without a worker; changing content, ignore rules, tool version or a link boundary invalidates; other projects never share; compare executions avoided vs hashing cost.
- **Watch out:** Shell, tests, builds and network reads are non-cacheable. A cache hit is never a fresh test result.

### TC02 — Reviewed tool plans and dependency-aware execution

- [ ] Execute a visible typed plan with fewer round trips.
- **Priority:** 2. **Needs:** TC01. **Builds on:** R02, A03.
- **Change:** Bounded dependency graph with validated args, bindings and effects; parallelize only enforceably read-only independent calls; serialize writes and unknown effects; per-call review for effects.
- **Accept:** Independent reads overlap, dependents wait, writes never overlap; failed prerequisite blocks descendants; malformed bindings cause no effect; cancellation starts no queued node; report counts/time vs serial.
- **Watch out:** A batch is not blanket consent; unresolved placeholders cannot be approved.

### TC05 — Failed-step recovery and explicit tool replay

- [ ] Recover a failed plan without repeating successful mutations.
- **Priority:** 2. **Needs:** TC02, TC03. **Builds on:** R02.
- **Change:** Persist node args, fingerprints, results and certainty; preview resuming only failed/unstarted nodes; recorded responses inspectable without re-execution.
- **Accept:** Interrupt after a confirmed write: recovery does not repeat it; upstream change requires a new plan; a lost mutation response blocks until reconciled; stale approvals and late responses start nothing.
- **Watch out:** Never promise exactly-once across crashes.

### D01 — Impact-directed verification plan

- [ ] Recommend the checks most likely to catch a change's defects.
- **Priority:** 2. **Needs:** T02, TC02. **Builds on:** C01, R01.
- **Change:** Use changed symbols, local dependencies, declared test mappings and prior failures (PK05 when available) to propose a minimal check set with reasons via Jolteon's review.
- **Accept:** Seeded project: the caller's test catches a changed callee; shared config or unmapped dependency triggers a broader plan; compare commands/time with the full suite; targeted passes are labelled targeted.
- **Watch out:** A recommendation is not proof of completeness or permission to run.

### Q03 — Token-saving and tool-efficiency quality benchmark

- [ ] Prove optimizations reduce measured work without weaker answers.
- **Priority:** 2. **Needs:** T01, T02, T03, T04, TC01, TC02, TC03, TC04, TC05, D01.
- **Change:** Paired baseline/optimized journeys (bug fix, explanation, noisy log, resume) with fixed model/settings/repo; cold/warm cache separated; all optimizer costs included.
- **Accept:** ≥3 paired trials per journey on a qualified provider with raw samples, correctness, missed defects, time, turns, tools, available usage/cost; same functional/permission checks; publish regressions; no universal percentage.
- **Watch out:** Live runs are paid and need explicit user approval; missing credentials block live results only.

## G — UX, platform and release

### UX03 — Contextual, searchable actions

- [ ] Make the next useful action discoverable.
- **Priority:** 1. **Needs:** —. **Builds on:** UX01, UX02.
- **Change:** Reuse the `/` and Ctrl+K registry with aliases, current-surface actions and visible disabled reasons (EV03's owning-form reasons plug in here).
- **Accept:** At both sizes find attachment, provider, cancellation and evidence actions by partial query; empty/disabled results explain the next step; Escape restores draft and focus; mutating actions open review only.
- **Watch out:** No second dispatcher; no execution on search.

### A08 — Actionable connection diagnostics

- [ ] Turn provider setup failures into one next step.
- **Priority:** 1. **Needs:** —. **Builds on:** A01.
- **Change:** Separate executable/version, permitted authentication, eligibility and transport failures; native setup action; redacted diagnostics; explain callback-free WSL login.
- **Accept:** Missing executable, incompatible version, real auth-required response and cancelled login each show the blocker and next action; details reachable; no tokens/codes in diagnostics; install ≠ authenticated.
- **Watch out:** Never read/copy credential stores or silently switch method/billing.

### UX04 — Readability and reduced-motion preferences

- [ ] Tune text density and motion without losing safety information.
- **Priority:** 2. **Needs:** —. **Builds on:** UX01, V01.
- **Change:** Compact/comfortable density, persistent reduced motion, installer animation opt-out.
- **Accept:** Preferences survive restart; focus, CJK editing and actions work at both sizes; reduced motion yields no animated frames in install/update and the TUI (PK03) with truthful phase output; redirected output stays escape-free.
- **Watch out:** NO_COLOR is not a reduced-motion request. Test a real assistive setup before claiming screen-reader support.

### S04 — Create reusable recipes from reviewed work

- [ ] Save a verified workflow without its authority or secrets.
- **Priority:** 2. **Needs:** —. **Builds on:** S03, R02, C02.
- **Change:** Preview a recipe from selected user-authored steps and real validation commands; edit name/scope/criteria; create-only TOML export via S03's schema. MV08 reuses this export path.
- **Accept:** Export, reopen and apply in another project; argv stays distinct from shell text; fresh review/approval required; cancel leaves no file; no overwrite; nothing auto-executes.
- **Watch out:** Exclude grants, sessions, credentials and absolute paths unless explicitly reviewed.

### P04 — Reviewed updates and safe rollback eligibility

- [ ] Explain an update before replacement and refuse unsafe rollback.
- **Priority:** 2. **Needs:** —. **Builds on:** P02, E01.
- **Change:** TUI entry to the checksum-verified updater with versions, source, sprite-cache effects and retained-data policy; rollback only when the package and schema compatibility are established.
- **Accept:** In isolated layouts: cancel changes nothing; approved update resumes the task; compatible rollback works; newer-schema rollback refused with byte-identical data; checksum failure keeps the last usable install.
- **Watch out:** Schema 7 from waves 1–4 makes rollback to ≤0.1.2 ineligible; test that refusal explicitly. No elevation or profile edits.

### A04 — Claude Agent SDK interoperability (blocked)

- [ ] Run the same task UX through the Claude SDK adapter.
- **Needs:** permitted Claude API/cloud or Console credentials. **Builds on:** A03.
- **Accept:** In a disposable workspace: preview/run, answer a question, deny/allow once, interrupt, resume by explicit session ID; no queued follow-up after cancel; no silently saved rule; cumulative usage not double-counted; hook/MCP boundaries verified; the Rust local path works without Node.
- **Status:** Sidecar implemented (SDK 0.3.296 / CLI 2.1.296 / Node ≥24, strict policy, questions/permissions, explicit resume, Bedrock/Vertex/Console `ant` routes). `--inspect` reports `credential_route: unavailable`; Console login start/cancel smoke passed without sign-in. Evidence: `target/claude-acceptance-inspect.json`, `target/auth-smoke-claude-console`.
- **Watch out:** Never reuse claude.ai subscription OAuth or read credential stores.

### A06 — Gemini ACP conformance (blocked)

- [ ] Prove the ACP client is not OpenCode-specific.
- **Needs:** completed Gemini authentication and current eligibility. **Builds on:** A05.
- **Accept:** Same prompt/cancel/permission/context scenarios on real Gemini ACP; a capability present in one peer and absent in the other gets a correct reason and is never invoked; record version, protocol, auth route and restrictions.
- **Status:** Gemini 0.28.0 `--experimental-acp` negotiates ACP1/loadSession, omits `session/list` (not invoked), returns real `session/new` 401 on Linux and Windows. Login uses the official native CLI. Evidence: `target/gemini-acceptance-inspect.json`.
- **Watch out:** A 401 does not mean API keys are the only route.

### A07 — Cross-agent handoff (blocked by A04, A06)

- [ ] Continue one task across agents without portable sessions.
- **Needs:** A04, A06. **Builds on:** A05, S01.
- **Accept:** Codex → Claude → ACP peer via accepted handoff; old session stays available; grants reset; only reviewed context sent; stale approvals and newer schema rejected; manual export/import creates no verified evidence.
- **Status:** Schema-1 JSON/Markdown handoff, idle switching, grant reset and Codex→OpenCode continuation passed; four provider-free manual cases pass on the current binary.

### Q01 — Complete journey and visual state matrix (blocked by A07)

- [ ] Prove all nine workflows and the composed UX.
- **Needs:** A04, A06, A07.
- **Accept:** Empty/missing, ready/running, permission/question, cancellation/unknown, save failure, stale evidence, handoff and party (concurrent partners, approval queue, writer lease) states across sizes/themes/CJK; keyboard focus, no accidental execution, bounded streams, purge, image cleanup, restoration; every controlled action names its enforcement boundary; naming/art-rights review (including MV06 move names) before distribution.
- **Status:** Five native rows passed at `d198fa5`; specialist journeys pass in all themes/layouts; Codex and Codex→OpenCode live rows pass. Claude/Gemini rows missing.

### Q02 — Measured native and token efficiency (blocked by Q01)

- [ ] Measure the performance contract.
- **Needs:** Q01.
- **Accept:** Observations against targets with method/sample size, core vs sidecar vs agent resources separate and aggregate, including three concurrent partners; bounded queues; no model call on form selection; unknown provider usage shown as unknown.
- **Status:** Local slice measured on M1 Pro (warm start p95 89.55 ms, idle 0.826% CPU / 7.44 MiB, streaming input p95 12.61 ms, 10 MiB stream bounded to 1 MiB, 10k-line diff 137.66 ms). Codex review resources measured; SDK and provider token totals unavailable.

### P03 — Native distribution, upgrade and support evidence (blocked by Q02)

- [ ] Reproducible install/upgrade on the supported matrix.
- **Needs:** Q01, Q02. **Builds on:** P02.
- **Accept:** Clean-target install without Rust; checksums, help/version, local flow, no account/art/network prerequisites; schema upgrade with resume; incompatible downgrade refused without data loss; uninstall keeps data; per-target evidence and unsigned-artifact disclosure.
- **Status:** 0.1.0–0.1.2 ship five native packages with notices, bootstrap/update/uninstall and release gating. Remaining: integrated/schema/provider acceptance and signing/notarization.
