# D — Moves: teaching and learning skills

Raising your own partners means each one learns your repeated work as named **moves**. Moves belong to the individual partner, four per form: Bolt's Jolteon moves come back whenever Bolt becomes Jolteon again, and another partner doesn't know them until taught (MV08A). A move is a single-form reusable action; an Evolution Stone (S03 recipe) stays a whole-task workflow. Moves reuse S03's step types and validation, carry no grants, and every use goes through the normal review.

| Form | Move kind | Example default name |
|---|---|---|
| Eevee | task template (goal + criteria) | Helping Hand |
| Vaporeon | pin set (paths/ranges resolved at use) | Surf |
| Jolteon | run (exact argv + project-relative cwd) | Thunderbolt |
| Umbreon, Leafeon | run through Jolteon's review (diagnostic/measurement) | Foul Play, Leaf Blade |
| Espeon, Flareon, Glaceon, Sylveon | request template (instruction + context selectors) | Future Sight, Flamethrower, Ice Beam, Moonblast |

### MV01A — Move storage and teaching a run move

- [ ] Teach a partner's Jolteon form a run move from an approved run.
- **Priority:** 1. **Needs:** PT02A. **Builds on:** S03, R02.
- **Change:** Add a `src/store/moves.rs` table (partner, form, name, kind, steps, scope, provenance, created) with at most four per partner per form. `Teach as move` after an approved run previews the exact argv/cwd and saves it with a placeholder name. Picker detail lists the partner's moves for that form. Add Moves to DESIGN_RULES' concept list.
- **Accept:** Teach a run move, restart, and see it. Switch the partner away and back to Jolteon and it is still there; another partner doesn't see it. A fifth move is refused with the reason.
- **Watch out:** Ship in the Schema 7 migration if possible. Steps are exact argv (never shell text) with project-relative cwd. Refuse secrets via `workspace::privacy` and out-of-root paths.

### MV01B — Teach a pin-set move

- [ ] Teach Vaporeon a reusable set of context pins.
- **Priority:** 1. **Needs:** MV01A. **Builds on:** C02.
- **Change:** `Teach as move` from the basket stores project-relative paths/ranges, resolved again at use time.
- **Accept:** Teach a three-pin set; the preview shows the paths, and excluded files are refused.

### MV01C — Teach request and task-template moves

- [ ] Teach request forms and Eevee reusable templates.
- **Priority:** 2. **Needs:** MV01A. **Builds on:** A02, E01.
- **Change:** After a sent request, store its user-authored instruction plus context selectors; for Eevee, store goal and criteria as a task template.
- **Accept:** Teach an Espeon request and an Eevee template; their previews show only user-authored text.
- **Watch out:** Never store provider output or memories inside a move.

### MV02A — Move menu

- [ ] Show a partner's moves in a battle-style menu.
- **Priority:** 1. **Needs:** MV01A. **Builds on:** UX01.
- **Change:** `/move` and a form-surface key open a 2×2 grid (a list below 90 columns) with name, type accent, kind and last outcome.
- **Accept:** Opens and closes with the draft and focus restored at both sizes. An empty menu says how to teach a move.

### MV02B — Use a move

- [ ] Turn a chosen move into the normal pre-filled review.
- **Priority:** 1. **Needs:** MV02A. **Builds on:** R01, C02.
- **Change:** Choosing a move opens the command review, pin preview or request preview, pre-filled.
- **Accept:** Nothing runs before approval; missing pin files show as unavailable; a request move shows the full receipt.
- **Watch out:** A move is never pre-approved, however often it is used.

### MV03A — Detect repeated runs

- [ ] Notice when the same approved run repeats.
- **Priority:** 1. **Needs:** MV01A. **Builds on:** R02.
- **Change:** A local query finds the same normalized argv+cwd approved ≥3 times by the same partner in a project.
- **Accept:** Three `cargo test --workspace` runs produce one candidate; different arguments don't merge; argv with secrets, env assignments or out-of-project absolute paths is never a candidate.
- **Watch out:** Report threshold behavior on fixtures.

### MV03B — "wants to learn a move!" prompt

- [ ] Offer to learn a detected candidate.
- **Priority:** 1. **Needs:** MV03A, MV02B. **Builds on:** —.
- **Change:** At an idle boundary show `Bolt wants to learn a move!` with the steps and their three sources: Learn (opens naming), Not now, Never for this.
- **Accept:** The prompt appears once. Never survives a restart; Not now returns after three more uses.
- **Watch out:** Never prompt during a review or a running request, and never auto-learn.

### MV03C — Detect repeated pin sets

- [ ] Offer Vaporeon moves for repeated context selections.
- **Priority:** 2. **Needs:** MV03B, MV01B. **Builds on:** —.
- **Change:** The same pin set used in ≥3 requests produces a learn prompt.
- **Accept:** Three requests with identical pins produce one prompt; a reordered set counts as the same.

### MV04 — Forget a move when slots are full

- [ ] Choose which move to forget when learning a fifth.
- **Priority:** 2. **Needs:** MV03B. **Builds on:** —.
- **Change:** Learning into a full form shows the four moves and the new one: forget one, or `Give up learning`.
- **Accept:** The forgotten move leaves the menu and is still in the reminder list after a restart; giving up changes nothing.
- **Watch out:** Say which scope (project/personal) the forgotten move came from.

### MV05 — Move Reminder

- [ ] Relearn a forgotten or postponed move.
- **Priority:** 2. **Needs:** MV04. **Builds on:** E03.
- **Change:** A per-partner, per-form reminder list of forgotten and Not-now moves; relearning needs a free slot (or MV04's flow).
- **Accept:** A relearned move has identical steps. Purging a project removes its reminders and keeps personal ones.
- **Watch out:** Revalidate steps on relearn.

### MV06 — Type-themed names

- [ ] Suggest a fitting Pokémon move name and allow renaming.
- **Priority:** 2. **Needs:** MV01A. **Builds on:** —.
- **Change:** A fixed local list of canonical moves per type, matched by meaning where possible (test run → Thunderbolt, planning → Future Sight, review → Ice Beam), otherwise the first unused name. Names are freely renamable and unique per partner and form.
- **Accept:** Three Jolteon moves get distinct suggestions; a CJK rename works; a duplicate name is refused.
- **Watch out:** Names are labels: they never change behavior or enter provider packets. Include the list in Q01's naming-rights review.

### MV07 — Move record

- [ ] Show how a move has actually performed.
- **Priority:** 2. **Needs:** MV02B. **Builds on:** R02.
- **Change:** Move detail shows uses, last result and revision, and observed outcomes as a count (`7/9 exited 0`). Denied and cancelled uses are listed separately.
- **Accept:** Counts match the journal and survive a restart.
- **Watch out:** No power, accuracy % or level. Exit 0 is not correctness.

### MV08A — Teach a move to a party member

- [ ] Share a move with another partner directly.
- **Priority:** 2. **Needs:** MV01A, PT04B. **Builds on:** —.
- **Change:** `Teach to <partner>` copies a move into that partner's slot for the same form, with provenance (`taught by Bolt`).
- **Accept:** Mizu receives Bolt's move; a full target opens MV04's flow; Bolt's copy is unchanged.

### MV08B — TM export and import

- [ ] Share moves across projects and people as files.
- **Priority:** 2. **Needs:** MV01A, S04. **Builds on:** S03.
- **Change:** Export a create-only TOML "TM" through S04's export path. Import previews the steps, target partner, form and slot.
- **Accept:** Export from one project and import into another; use still requires review. Newer schema, out-of-root paths and secrets are refused; existing files are never overwritten.
- **Watch out:** Imports carry no approvals and never execute.
