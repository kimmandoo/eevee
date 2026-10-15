# E — Partner record and flavor

Pokémon flavor wraps facts and never replaces them. No XP, levels, affection, streaks or random unlocks (DESIGN_RULES § Bond).

### EV05 — Party status in the picker

- [ ] Show what each form has actually produced for this task.
- **Priority:** 2. **Needs:** PT06A. **Builds on:** V03, R02, G01, S01.
- **Change:** Mark each form `produced`, `stale` or `none` with one fact and the producing partner (Vaporeon `4 captures · Mizu`, Jolteon `exit 0 at a1b2c3, stale · Bolt`).
- **Accept:** Produce and then invalidate artifacts for three forms; the markers follow after a restart. Unused forms show `none`. Readable in monochrome/ASCII.

### EV06 — Form route per partner

- [ ] Record each partner's form switches without treating them as evidence.
- **Priority:** 2. **Needs:** EV05. **Builds on:** E02.
- **Change:** Route events (partner, from, to, time, accepted suggestion). The task card shows one route per partner, e.g. `Bolt: Eevee → Flareon → Jolteon`, dimming visits without an exit artifact.
- **Accept:** After a restart each partner resumes in its last form with the same route. The journal labels these as non-evidence; packets and handoffs contain none; purge removes them.
- **Watch out:** This is a schema change; batch it with other wave-6 persistence.

### PK02 — Summary screen

- [ ] A Pokémon-style summary page built from real records.
- **Priority:** 2. **Needs:** PK01, MV01A, EV05. **Builds on:** S02.
- **Change:** Per partner: nickname, form/type, `Met on <date> in <project>`, moves per form, Bond memory count, and per-form task counts with produced artifacts.
- **Accept:** Numbers equal store queries; a new partner shows its met date and `No moves yet`. Pages at 60×18.
- **Watch out:** No level or invented stats; a derived met date is labelled derived.

### EV08A — Partner lines for run results

- [ ] Eevee reacts to a finished owned run in one factual line.
- **Priority:** 2. **Needs:** EV01A. **Builds on:** V01, R02.
- **Change:** On run pass/fail, show one deterministic line with the acting partner, the fact and the suggestion: `Bolt used Thunderbolt! cargo test failed (exit 101). Umbreon can investigate.` (the move name only when a move was used).
- **Accept:** Exactly one line per run; replaying the journal or restarting never repeats it; it stays in the composer area at 60×18.
- **Watch out:** No randomness or praise, and never hide permission/error facts.

### EV08B — More events and an off switch

- [ ] Extend lines to review, handoff and staleness events, and allow turning them off.
- **Priority:** 2. **Needs:** EV08A. **Builds on:** G01, S01.
- **Change:** Add lines for review verdict, handoff ready and source-changed-after-verification, plus a persistent preference to turn lines off.
- **Accept:** Each event shows exactly one line; with lines off nothing else changes.

### PK05A — Failure signatures

- [ ] Remember failures the party has met.
- **Priority:** 2. **Needs:** EV10A. **Builds on:** R02.
- **Change:** Derive signatures from owned failing runs (rustc error code + location, test name, panic location) into per-project Dex entries: first seen, seen count, runs.
- **Accept:** The same failure twice gives one entry with count 2; another project never sees it; purge removes it.
- **Watch out:** Redact signatures and never store output twice. Schema change.

### PK05B — "Seen before" with its resolution

- [ ] Point at how a known failure was resolved last time.
- **Priority:** 2. **Needs:** PK05A. **Builds on:** U01, A03.
- **Change:** Link a Dex entry to the supported hypothesis/fix/passing run that followed it. When the signature reappears, show `Seen before: resolved in task #12 (hypothesis …)`.
- **Accept:** Fail, fix and fail again in a fixture; the second failure links the first resolution.
- **Watch out:** A past resolution is a lead, not a current fix or verification.

### EV09 — Learn a habit from repeated instructions

- [ ] Offer to save an instruction repeated across requests as a Bond memory.
- **Priority:** 2. **Needs:** T01A. **Builds on:** S02.
- **Change:** Identical normalized instruction text in ≥3 requests in a project → Sylveon proposes it through S02's approval, showing its sources. Dismissal is remembered per text hash.
- **Accept:** One proposal for three repeats; after approval the next receipt shows the memory instead of repeated text; a dismissed proposal doesn't return.
- **Watch out:** Never infer or save preferences automatically; secret-filtered text is never proposed. Moves cover actions; this covers instructions.

### EV17 — Per-partner provider pairing

- [ ] Give each partner a remembered agent/model per form.
- **Priority:** 2. **Needs:** PT02A. **Builds on:** A01, A02, V03.
- **Change:** After choosing an agent/model, offer `Use for <partner> as <form> in this project`; later switches preselect it, labelled remembered. Cooperating partners can use different agents (Bolt on Codex, Mizu on OpenCode).
- **Accept:** Two partners' pairings each preselect correctly; removing one restores the default; a disconnected agent is labelled unavailable, never replaced.
- **Watch out:** Selection never connects, sends, or changes billing/authentication.

### EV11 — Rank `@` candidates by form

- [ ] Show the most relevant files first when attaching.
- **Priority:** 2. **Needs:** EV10A. **Builds on:** C02.
- **Change:** Same picker, ordered by form (Umbreon: failure locations; Flareon/Leafeon: working diff; Glaceon: changed files and their tests), each with a short reason. Typed queries return the same match set in every form.
- **Accept:** With a fixture failure and diff, the top entries differ between Umbreon and Flareon and show reasons; excluded files never appear.
- **Watch out:** Report ranking on the fixture when rules change. No index rebuild per keystroke.

### PK03 — Evolution transition

- [ ] A short evolution moment on picker switches.
- **Priority:** 3. **Needs:** EV02, UX04B. **Builds on:** V02B.
- **Change:** On picker switches only, a ≤300 ms silhouette flash with `What? Bolt is evolving!` → `Bolt became Jolteon!`; text-only routes show the final line only.
- **Accept:** Rapid switching never queues animations; any key skips; typed input lands in the draft; reduced motion shows no frames; no idle redraw afterwards.
- **Watch out:** Forms are reversible, so returning to Eevee uses neutral copy. Clear native images behind overlays.

### PK04 — Held items

- [ ] Let a partner's form always bring a chosen file or memory.
- **Priority:** 3. **Needs:** C03A, EV02. **Builds on:** S02.
- **Change:** One held item per partner per form per project, proposed as a removable chip on entering the form and shown in the strip and receipt.
- **Accept:** It appears after switching and is revalidated before send; an excluded file cannot be held.
- **Watch out:** Proposed, never silently included.

### PK06 — Ribbons

- [ ] Commemorate real first milestones.
- **Priority:** 3. **Needs:** PK02. **Builds on:** G01, S01.
- **Change:** One-time, evidence-linked ribbons per project (first accepted review, first verified handoff, first learned move), attributed to the partner and shown on its summary. Hideable.
- **Accept:** Each ribbon links its evidence; purge removes it.
- **Watch out:** Drop this task if it reads as gamification: no counts or streaks, and nothing gated.
