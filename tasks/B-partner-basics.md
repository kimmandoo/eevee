# B — Partner basics and form suggestion

These tasks make each switch informed and each form immediately useful, for the lead partner and for party members. All are local UI/state work.

### EV04 — Form-specific empty states

- [x] Give each form one obvious first action when its surface is empty.
- **Priority:** 1. **Needs:** —. **Builds on:** V01, UX01.
- **Change:** Show the form's `object → action` line from the DESIGN_RULES interaction table plus one primary action that opens the existing flow (Vaporeon `@` attach, Jolteon `/run` review, Umbreon add hypothesis, Glaceon review desk, Sylveon handoff preview, Eevee edit criteria).
- **Accept:** Open all nine forms on an empty task at both sizes; each primary action opens its flow and executes nothing. Non-empty surfaces are unchanged.
- **Watch out:** One line and one action, per the concise-copy rule. No placeholder counts.

### EV03A — Form capability chip

- [x] Show what the active form may do.
- **Priority:** 1. **Needs:** —. **Builds on:** UX01.
- **Change:** Add a header chip with the form's default capabilities from the role table (`read · run`, `read · edit`, `read`).
- **Accept:** All nine forms show the chip matching the role table, and it fits at 60×18 without truncating project/form names.
- **Watch out:** The chip is descriptive only; enforcement stays in the approval flow.

### EV03B — Owning-form redirect

- [x] Route an out-of-role action to the form that owns it.
- **Priority:** 1. **Needs:** EV03A. **Builds on:** R01, A03.
- **Change:** An out-of-role action shows a disabled reason naming the owning form and offers `Switch to <form>`. After PT10, it also offers `Send to <partner>` when an idle party member is in that form.
- **Accept:** Requesting an edit from Glaceon and a command from Espeon each name the owner and offer a working switch, without starting the action.
- **Watch out:** Never widen or bypass a grant.
- **Verified:** Glaceon `/paths` and `/role-implement` offered Flareon; Espeon `/run` → `/command-edit` offered Jolteon. Actual TUI passed all three routes at 120×36 and 60×18, including Escape, deliberate switching and explicit retry, with zero execution/provider records and unchanged source. The switch did not replay the action. Leafeon retained explicit scope editing; existing neutral reviews kept priority. `Send to <partner>` remains dependent on PT10.

### EV07 — Return to Eevee

- [x] Step the lead partner back to intake in one action.
- **Priority:** 1. **Needs:** PT03A. **Builds on:** V03.
- **Change:** `/eevee` (from PT03A) plus a top picker entry; also show EV01's suggestion once EV01A exists.
- **Accept:** Return from Flareon, Umbreon and Sylveon; stored task payload, draft text/carets and evidence are byte-identical before and after. Form/previous-form navigation, revision and save time change as required by a persisted switch.
- **Watch out:** A working partner offers Stay or cancel-and-wait; never cancel implicitly.
- **Verified:** 16 actual-TUI cases at 120×36 and 60×18 passed named/picker returns from all three forms with a real capture/run and retained Unicode draft, owned-command Stay then cancel-and-wait, and native cancellation holding the form on unknown remote outcome. Provider cancellation retained the visible draft/caret. No implicit execution/send; EV01A's future recommendation remains separate.

### PK01 — Partner nickname

- [x] Let the user name each Eevee.
- **Priority:** 1. **Needs:** PT02A. **Builds on:** UX01.
- **Change:** Set a personal-scope nickname (≤12 graphemes, Unicode, unique in the party) from the partner card or `/nickname`. The header and dialogue show `Nickname (Species)`; clearing it restores `Eevee #n`.
- **Accept:** Set a CJK nickname, restart, and see it in the header and dialogue in every form at both sizes; species and type stay visible. The receipt shows zero nickname bytes in a provider packet.
- **Watch out:** Measure width by grapheme/cell width, not bytes. Use the column reserved in PT02A.
- **Verified:** `/nickname`, Companion/roster `n`, committed rename/clear and refusal/retry use the reserved column with no schema change. Actual TUI passed both sizes' personal-only and task-backed restart, all nine forms' header/dialogue/species/type, duplicate and 13-grapheme refusal, 12-CJK display and clear. Exact loopback request bodies omitted the nickname and the visible receipt showed 0 B of automatically added partner flavor.

### EV01A — Suggestion rules

- [x] Suggest the form that fits the current evidence, with the triggering fact.
- **Priority:** 1. **Needs:** —. **Builds on:** V03, E02, R02, U01, G01.
- **Change:** One rule table, first match wins: goal without criteria → Espeon; criteria without pins → Vaporeon; latest owned run failed → Umbreon; diff without a run at its revision → Jolteon; passing current run without review → Glaceon; accepted review → Sylveon. Show it in the task card: `Suggested: Umbreon — cargo test exited 101`.
- **Accept:** Fixture tasks for each state show the expected suggestion; no match shows nothing; editing a source after a passing run changes the suggestion.
- **Watch out:** Report fixture results whenever a rule changes. A suggestion never self-activates.
- **Verified:** Both sizes' actual fixtures showed goal→Espeon, criteria without pins→Vaporeon, actual exit 3→Umbreon, current pass→Glaceon, source edit + `/refresh`→Jolteon, guarded Accept→Sylveon, and no match on empty/clean pinned state. Source/read guards stayed off render; newer unknown/partial results never reused an older pass. Provider output and current-task cards kept the recommendation, with no auto form/send/run or recommendation bytes in the actual fixture request.

### EV01B — Suggestion in the picker and dismissal

- [x] Show the suggestion where forms are chosen, and let the user dismiss it.
- **Priority:** 1. **Needs:** EV01A. **Builds on:** V03.
- **Change:** The picker marks the suggested form. Dismissing hides the suggestion until the triggering evidence identity changes.
- **Accept:** Dismiss survives a restart and reappears after a new failing run. Enter in the picker still only previews or switches.
- **Verified:** Both 120×36 and 60×18 actual TUI fixtures marked Umbreon after exit 7 while keeping Jolteon current/selected; immediate Enter started nothing. Picker `d` from the top intake row persisted the exact trigger, hid it across restart/form changes, preserved Unicode request/caret and evidence bytes, and a second actual failed run resurfaced it. Schema 10→11 preserved existing rows; the actual Schema 10 binary refused the migrated database byte-for-byte.

### EV01C — Suggest a free party member

- [ ] Name an idle partner already in the suggested form.
- **Priority:** 2. **Needs:** EV01A, PT04A. **Builds on:** —.
- **Change:** If an idle partner is in the suggested form, append `Mizu (Umbreon) is free`.
- **Accept:** With an idle Umbreon partner the hint appears, and it disappears while that partner works.

### EV02 — Carry-over strip

- [x] Show what stayed and changed right after a switch, without slowing it down.
- **Priority:** 1. **Needs:** PT03A. **Builds on:** C02, UX01.
- **Change:** After any switch, a one-line non-blocking strip shows carried state (criteria met/total, pins, open hypotheses/findings) and the capability diff (`+edit −run`). It fades on the next keystroke and can be reopened from the partner card.
- **Accept:** Three switch pairs show counts that equal stored state and the correct diff. Typing right after a switch lands in the draft unchanged. Fits 60×18 in monochrome/ASCII.
- **Watch out:** Never steal focus or swallow keys. Keep it data-driven: EV12, EV13, PK03 and PK04 reuse it.
- **Verified:** Actual 120×36/60×18 × monochrome Unicode/ASCII fixtures passed all 16 journey groups. Jolteon→Flareon, Flareon→Espeon and Espeon→Sylveon showed stored C1/2 P1 H1 F1 with `+edit -run`, `+edit-plans -edit` and `+edit-docs -edit-plans`; the next Unicode key/paste preserved exact draft bytes/caret. Companion `c` reopened the same record. Captures and provider journals stayed byte-identical; each fixture made only its one explicit loopback request and no local run. Pending context/history and deliberate evidence refresh had behavioral regressions; the eight actual execution journeys also passed.
