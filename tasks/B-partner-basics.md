# B — Partner basics and form suggestion

These tasks make each switch informed and each form immediately useful, for the lead partner and for party members. All are local UI/state work.

### EV04 — Form-specific empty states

- [ ] Give each form one obvious first action when its surface is empty.
- **Priority:** 1. **Needs:** —. **Builds on:** V01, UX01.
- **Change:** Show the form's `object → action` line from the DESIGN_RULES interaction table plus one primary action that opens the existing flow (Vaporeon `@` attach, Jolteon `/run` review, Umbreon add hypothesis, Glaceon review desk, Sylveon handoff preview, Eevee edit criteria).
- **Accept:** Open all nine forms on an empty task at both sizes; each primary action opens its flow and executes nothing. Non-empty surfaces are unchanged.
- **Watch out:** One line and one action, per the concise-copy rule. No placeholder counts.

### EV03A — Form capability chip

- [ ] Show what the active form may do.
- **Priority:** 1. **Needs:** —. **Builds on:** UX01.
- **Change:** Add a header chip with the form's default capabilities from the role table (`read · run`, `read · edit`, `read`).
- **Accept:** All nine forms show the chip matching the role table, and it fits at 60×18 without truncating project/form names.
- **Watch out:** The chip is descriptive only; enforcement stays in the approval flow.

### EV03B — Owning-form redirect

- [ ] Route an out-of-role action to the form that owns it.
- **Priority:** 1. **Needs:** EV03A. **Builds on:** R01, A03.
- **Change:** An out-of-role action shows a disabled reason naming the owning form and offers `Switch to <form>`. After PT10, it also offers `Send to <partner>` when an idle party member is in that form.
- **Accept:** Requesting an edit from Glaceon and a command from Espeon each name the owner and offer a working switch, without starting the action.
- **Watch out:** Never widen or bypass a grant.

### EV07 — Return to Eevee

- [ ] Step the lead partner back to intake in one action.
- **Priority:** 1. **Needs:** PT03A. **Builds on:** V03.
- **Change:** `/eevee` (from PT03A) plus a top picker entry; also show EV01's suggestion once EV01A exists.
- **Accept:** Return from Flareon, Umbreon and Sylveon; stored state is byte-identical before and after.
- **Watch out:** A working partner offers Stay or cancel-and-wait; never cancel implicitly.

### PK01 — Partner nickname

- [ ] Let the user name each Eevee.
- **Priority:** 1. **Needs:** PT02A. **Builds on:** UX01.
- **Change:** Set a personal-scope nickname (≤12 graphemes, Unicode, unique in the party) from the partner card or `/nickname`. The header and dialogue show `Nickname (Species)`; clearing it restores `Eevee #n`.
- **Accept:** Set a CJK nickname, restart, and see it in the header and dialogue in every form at both sizes; species and type stay visible. The receipt shows zero nickname bytes in a provider packet.
- **Watch out:** Measure width by grapheme/cell width, not bytes. Use the column reserved in PT02A.

### EV01A — Suggestion rules

- [ ] Suggest the form that fits the current evidence, with the triggering fact.
- **Priority:** 1. **Needs:** —. **Builds on:** V03, E02, R02, U01, G01.
- **Change:** One rule table, first match wins: goal without criteria → Espeon; criteria without pins → Vaporeon; latest owned run failed → Umbreon; diff without a run at its revision → Jolteon; passing current run without review → Glaceon; accepted review → Sylveon. Show it in the task card: `Suggested: Umbreon — cargo test exited 101`.
- **Accept:** Fixture tasks for each state show the expected suggestion; no match shows nothing; editing a source after a passing run changes the suggestion.
- **Watch out:** Report fixture results whenever a rule changes. A suggestion never self-activates.

### EV01B — Suggestion in the picker and dismissal

- [ ] Show the suggestion where forms are chosen, and let the user dismiss it.
- **Priority:** 1. **Needs:** EV01A. **Builds on:** V03.
- **Change:** The picker marks the suggested form. Dismissing hides the suggestion until the triggering evidence identity changes.
- **Accept:** Dismiss survives a restart and reappears after a new failing run. Enter in the picker still only previews or switches.

### EV01C — Suggest a free party member

- [ ] Name an idle partner already in the suggested form.
- **Priority:** 2. **Needs:** EV01A, PT04A. **Builds on:** —.
- **Change:** If an idle partner is in the suggested form, append `Mizu (Umbreon) is free`.
- **Accept:** With an idle Umbreon partner the hint appears, and it disappears while that partner works.

### EV02 — Carry-over strip

- [ ] Show what stayed and changed right after a switch, without slowing it down.
- **Priority:** 1. **Needs:** PT03A. **Builds on:** C02, UX01.
- **Change:** After any switch, a one-line non-blocking strip shows carried state (criteria met/total, pins, open hypotheses/findings) and the capability diff (`+edit −run`). It fades on the next keystroke and can be reopened from the partner card.
- **Accept:** Three switch pairs show counts that equal stored state and the correct diff. Typing right after a switch lands in the draft unchanged. Fits 60×18 in monochrome/ASCII.
- **Watch out:** Never steal focus or swallow keys. Keep it data-driven: EV12, EV13, PK03 and PK04 reuse it.
