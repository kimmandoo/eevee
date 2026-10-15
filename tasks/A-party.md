# A — Party: free form switching and cooperating partners

Contract: [DESIGN_RULES § Party](../docs/DESIGN_RULES.md) (confirmed 2026-10-15: party ≤6, ≤3 working partners per task, one writer per task). Today the app has one `App.form` and one planning/request state, so the order is model → switching → roster → routing → concurrency. Each ID below is one commit.

### PT02A — Partner table and Schema 7 migration

- [x] Store partners and migrate the existing state as partner #1.
- **Priority:** 1. **Needs:** —. **Builds on:** PT01, E02.
- **Change:** Add a `partners` table (id, nickname nullable, current form, met date, created project) and `tasks.lead_partner`. Migrating a Schema 6 store creates partner #1 in the stored form, with the met date set to the earliest task date and labelled derived. No UI change.
- **Accept:** Upgrade a Schema 6 fixture: one partner exists in the previous form and every task resumes unchanged. An old binary refuses the new store with byte-identical files (reuse `scripts/schema_upgrade_smoke.py`).
- **Watch out:** This is the only Schema 7 bump planned for waves 1–2. Reserve the columns PK01 (nickname) and PT05B (per-partner drafts) need, so they don't trigger another bump. Add a `src/store/partners.rs` submodule; don't grow `tasks.rs`.

### PT02B — Lead-partner accessor

- [x] Route every read and write of the active form through the lead partner.
- **Priority:** 1. **Needs:** PT02A. **Builds on:** V03.
- **Change:** Replace direct `App.form` uses with one accessor/mutator pair backed by the lead partner. The header shows the partner (`Eevee #1 · Jolteon`).
- **Accept:** All existing form-switch, picker and persistence tests pass unchanged. After a restart, the stored form equals the partner's form.
- **Watch out:** Pure refactor plus one label. Grep that no `self.form =` writes remain outside the accessor.

### PT03A — Named form commands

- [x] Switch form by name without opening the picker.
- **Priority:** 1. **Needs:** PT02B. **Builds on:** UX01.
- **Change:** Register `/eevee`, `/vaporeon`, … `/sylveon` in the existing `/` registry with type aliases (`/water`, `/electric`, …). Each switches the lead partner instantly.
- **Accept:** Each command switches the form and keeps task, draft, caret and context byte-identical. Pasted `/jolteon` text never switches.
- **Watch out:** Numeric shortcuts stay removed. Avoid aliases that collide with existing commands (`/run` already exists), and test the registry for duplicates.

### PT03B — Previous-form toggle

- [x] Flip back to the last form with one key.
- **Change:** Add a `previous form` action (`/back-form` plus a key that is free on every non-text surface), stored per partner.
- **Accept:** Vaporeon → Flareon → toggle → toggle ends in Flareon. The toggle is suppressed inside text fields, and the stored previous form survives a restart.
- **Watch out:** Native images clear and redraw once per switch, not once per frame.

### PT04A — Party roster view

- [ ] See every partner and what it is doing.
- **Priority:** 1. **Needs:** PT02A. **Builds on:** UX01.
- **Change:** `/party` opens a read-only list: sprite (or text), nickname or `Eevee #n`, current form, status (idle / working / waiting for approval) and current task.
- **Accept:** With one migrated partner, the roster shows it as idle in its form at both sizes. Escape restores focus.
- **Watch out:** The roster performs no request, and its status is derived, never stored.

### PT04B — Add a partner

- [ ] Welcome a new Eevee into the party.
- **Priority:** 1. **Needs:** PT04A. **Builds on:** —.
- **Change:** A roster action adds a partner that starts in Eevee form with today's met date. A seventh is refused with the limit as the reason.
- **Accept:** Add two partners, restart, and see three. The seventh attempt changes nothing.

### PT04C — Release a partner

- [ ] Remove an idle partner with a clear preview.
- **Priority:** 2. **Needs:** PT04B. **Builds on:** E03.
- **Change:** Show a preview of what is deleted (moves, pairings, previous-form state) and what stays (tasks, evidence with a name snapshot); approval is neutral. Partner #1 can be released only if another partner remains.
- **Accept:** Release an idle partner and its rows disappear while evidence still shows the snapshot name. A working partner, or the last partner, cannot be released.
- **Watch out:** Store the attribution as a name snapshot in evidence at write time, so releasing never leaves a dangling id.

### PT05A — Choose the lead partner

- [ ] Bring a different partner to the front of a task.
- **Priority:** 1. **Needs:** PT04B. **Builds on:** E02.
- **Change:** `/lead <name>` and a roster action set the task's lead partner. The header and picker follow the lead.
- **Accept:** Switch the lead between two partners in different forms; the header and picker follow, and the choice survives a restart.

### PT05B — Per-partner drafts

- [ ] Give each partner its own draft on the same task.
- **Priority:** 1. **Needs:** PT05A. **Builds on:** E01, UX01.
- **Change:** Drafts are keyed by (task, partner), and existing drafts migrate to the lead partner. The composer title shows whose draft it is.
- **Accept:** Write different CJK drafts for two partners, switch leads and restart; each draft and caret returns exactly. A dirty-draft warning applies per draft.
- **Watch out:** Use the column reserved in PT02A. Paste safety (never execute) is unchanged.

### PT06A — Partner attribution on journal entries

- [ ] Record which partner produced each run, request and review.
- **Priority:** 1. **Needs:** PT05A. **Builds on:** R02.
- **Change:** New journal/run/provider records carry a partner id plus a name snapshot; old records show `partner unknown`. Evidence views display it.
- **Accept:** Run a command with each of two partners; each record names its partner, and old fixture records show `partner unknown`.
- **Watch out:** Attribution is display metadata. It never enters provider packets or handoffs.

### PT06B — Per-partner request slot

- [ ] Replace the single planning/request state with a slot keyed by partner.
- **Priority:** 1. **Needs:** PT06A. **Builds on:** A02, A03, R01.
- **Change:** Move request state into a map keyed by partner, still limited to one working partner per task. Events, questions and approvals are routed by (task, partner, request id).
- **Accept:** Start a request, switch the lead and back: the events and the pending approval stay on the original partner. Late events for another or released partner are refused. All existing provider/run tests pass unchanged.
- **Watch out:** This is the riskiest refactor in the backlog. Gate it on the existing approval/cancellation tests and land it with no behavior change except routing.

### PT06C — Partner-named approvals

- [ ] Make every approval say who is asking.
- **Priority:** 1. **Needs:** PT06B. **Builds on:** R01.
- **Change:** Approval, question and cancel dialogs show `Bolt (Jolteon) wants to run: cargo test`.
- **Accept:** The run, provider-question and permission dialogs each name the partner at 60×18 without hiding Approve/Deny.

### PT08A — Write lease

- [ ] Allow only one editing partner per task.
- **Priority:** 2. **Needs:** PT06B. **Builds on:** A03, L01.
- **Change:** Edit-capable work (Flareon, Leafeon) takes a per-task lease shown in the header. A second edit request shows `Bolt (Flareon) is editing` and starts nothing. The lease is released on completion, cancellation and startup recovery.
- **Accept:** Two Flareon partners request edits: the second waits. Killing the process mid-edit and restarting leaves no stale lease.
- **Watch out:** Persist the lease with the owning request id so recovery can prove it is stale.

### PT08B — `revision moving` runs

- [ ] Stop runs during an edit from counting as fresh verification.
- **Priority:** 2. **Needs:** PT08A. **Builds on:** R02.
- **Change:** A run started while a lease is held records `revision moving` instead of a stable revision. Glaceon and stale markers never treat it as current.
- **Accept:** A Jolteon run during a Flareon edit shows `revision moving` and is not offered as verification.

### PT07A — Concurrent working partners

- [ ] Let up to three partners work on one task at once.
- **Priority:** 2. **Needs:** PT08B, PT06C. **Builds on:** R02, UX02.
- **Change:** Allow several request slots to run concurrently, up to 3 per task; a fourth is refused with the reason. The header shows the working partners.
- **Accept:** Mizu (Vaporeon) runs a provider planning request while Bolt (Jolteon) runs an approved command; both finish with correct attribution and receipts.
- **Watch out:** Bound output and journal memory per slot. Show each provider request's cost separately. Measure three concurrent partners in Q02.

### PT07B — Approval queue

- [ ] Answer concurrent approvals one at a time.
- **Priority:** 2. **Needs:** PT07A. **Builds on:** R01.
- **Change:** Pending approvals and questions queue in arrival order; the dialog shows `1 of 2` with partner names. Esc defers without answering.
- **Accept:** Two partners ask at once; each answer goes to the right partner, and deferring keeps both pending.
- **Watch out:** Never auto-answer or batch-approve.

### PT07C — Cancel all and quit with working partners

- [ ] Stop cooperating work safely.
- **Priority:** 2. **Needs:** PT07A. **Builds on:** UX02.
- **Change:** `Cancel` targets the lead's work; `Cancel all` lists every working partner first. Quitting with working partners lists them all with Stay as the default.
- **Accept:** Cancel one partner while the other keeps running. Cancel all stops both, and a quit review lists both.

### PT09 — Party board

- [ ] See every working partner at a glance.
- **Priority:** 2. **Needs:** PT07A. **Builds on:** UX02.
- **Change:** A compact HUD above the composer with one line per working or waiting partner (sprite, name, form, status, last event), collapsing to a count at 60×18. Selecting a line focuses that partner.
- **Accept:** With three working partners, each line updates only from its own events, and the count opens the full board at 60×18.
- **Watch out:** No idle redraw timer, HP bar or fake progress.

### PT10 — Send work to a partner

- [ ] Pass a result to a party member instead of switching your own form.
- **Priority:** 2. **Needs:** PT05B. **Builds on:** S01, G01.
- **Change:** Relays (EV12, EV13, EV14, EV15) offer `Send to <partner> (<form>)` when an idle partner is already in the target form; sending pre-fills that partner's draft with references. A Glaceon review requested from the patch author's own partner is labelled `same partner`.
- **Accept:** Send a supported hypothesis to an idle Flareon partner; its draft cites the evidence, and the sender's state is unchanged.
- **Watch out:** Sending transfers references only: no approvals, sessions or provider calls.

### PT11 — Formations

- [ ] Save and reuse a party arrangement.
- **Priority:** 3. **Needs:** PT07A. **Builds on:** S03.
- **Change:** Add optional role slots to the S03 recipe schema (scout: Vaporeon, runner: Jolteon, reviewer: Glaceon). Applying one maps slots to partners, sets their forms and pre-fills drafts; nothing runs.
- **Accept:** Apply a formation with three partners: forms are set and no request starts. A missing partner shows an empty slot.
- **Watch out:** Reuse the recipe parser and versioning; a formation grants nothing.
