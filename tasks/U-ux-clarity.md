# U — UX clarity: fix ambiguity that comes from the design itself

Requested 2026-10-28. The 0.1.35 PTY audit (120×36 and 60×18, isolated HOME) fixed surface issues: unconnected header label, status spacing, the `/todo` panel, palette order, the auto-mode suggestion strip and help. What remains comes from design decisions, so each task below changes a DESIGN_RULES contract first and the code second. Implemented 2026-10-28 (unreleased; see the checkpoint for verification).

Rules for every U task:
- **Safety is unchanged:** reviews, approvals, neutral defaults and refusal behavior stay exactly as they are. Only wording, grouping, defaults of *presentation* and discoverability change.
- **Update the drivers together:** native smoke drivers match footer prefixes (`Enter Send /`, `Type Request /`, `Draft only /`) and captions (`Specialist desk`, `/ Commands`, `Storage: saved`). Change them in the same commit and run the CI `native_workflows_smoke.py` profile locally before pushing.
- **Prove it on screen:** every task shows before/after captures from the actual binary at 120×36 and 60×18, English and Korean input.

### UX10 — One unambiguous key-hint style

- [x] Stop using `/` both as a separator and as the Commands key.
- **Priority:** 1. **Needs:** —. **Builds on:** UX04A.
- **Problem:** Footers read `Enter Send / Ctrl+N Newline / @ Files / / Commands` and `Up/Down Choose / Enter Open`, where `/` is a separator, a key and part of key names.
- **Change:** Use one ASCII separator (` | `, already used by connection footers) for footer key-hint lines in `messages/*.ftl` and inline UI strings; keep `/` only inside key names (`Up/Down`, `PgUp/PgDn`) and the Commands key shown as `/ Commands`. Add a lint test that rejects ` / /` and a trailing ` /` in hint messages.
- **Accept:** No hint line contains the separator next to the `/` key; all smoke footer matchers updated; captures at both sizes.

### UX11 — Brief safety copy by default

- [x] Say each safety fact once, where it matters, and move repeated disclaimers behind `/verbose`.
- **Priority:** 1. **Needs:** —. **Builds on:** UX04A (text density).
- **Problem:** Ordinary screens repeat legal-style lines ("nothing sent", "Provider status claims are not evidence, completion or permission", "no task is sent", "not acknowledged unless reported") so the real choice is hard to find.
- **Change:** Inventory every always-visible disclaimer. Keep it inline only on the screen where the effect could happen (consent, send, run, delete); elsewhere show it in compact density only under `/verbose` or `d Details`. Contract stays: the text still exists and stays inspectable.
- **Accept:** Count of always-visible disclaimer lines on home, `/todo`, `/evolution-preview`, connections and palette drops; consent/delete/run reviews keep every required statement; density preference still applies.

### UX12 — Everyday vs advanced commands

- [x] Make the palette show what a normal session needs first and hide duplicates.
- **Priority:** 1. **Needs:** —. **Builds on:** GP10.
- **Problem:** `/` shows 223 entries. Pairs such as `/provider`/`/connect`, `/new`/`/session-new`, `/resume`/`/session-resume`, `/todo-edit`/`/todo` look like different actions.
- **Change:** Tag each command `everyday` or `advanced`. The empty query lists everyday commands only, with `Ctrl+K All` (existing) showing everything. Aliases collapse into one row that lists the alias. Search still matches every command.
- **Accept:** Empty palette fits one 60×18 page of everyday commands; every command remains reachable by name; alias rows open the same action; captures.

### UX13 — Make Auto/Manual form mode a visible, reversible choice

- [x] Show clearly whether Eevee chooses the form and how to switch.
- **Priority:** 1. **Needs:** —. **Builds on:** EV18, GP09.
- **Problem:** Picking a specialist in `/forms` silently switches to Manual; returning needs the "Return to Eevee / intake" row or Backspace. `[Auto]` in the header doesn't say what it controls.
- **Change:** `/forms` shows a top toggle "Auto (Eevee picks at Send) / Pinned: <form>"; choosing a form says "Pinned — Auto off" and offers `a` to turn Auto back on. The header shows `[Auto]` or `[Pinned]`.
- **Accept:** Switching modes never changes drafts, policy or the running request; the todo-driven evolution follows the mode; captures.

### UX14 — Compact layout keeps the "what's next" facts

- [x] At 60×18, show the current todo step and predicted next form without the companion pane.
- **Priority:** 2. **Needs:** —. **Builds on:** GP09 (todo), 0.1.35 companion todo line.
- **Problem:** The companion pane, which now shows `→ Flareon · Todo 2/3 …`, doesn't exist below 90 columns, so compact users can't see why Eevee evolves.
- **Change:** Add the same line to the status row when space allows and to the composer title otherwise, yielding to activity/approval text.
- **Accept:** 60×18 and 80×24 captures with and without todo; approvals and errors still take priority.

### UX15 — First-run route that can actually connect

- [x] Don't default a new user to a route they can't use.
- **Priority:** 2. **Needs:** —. **Builds on:** A08, GP10.
- **Problem:** A fresh install defaults to `Codex/CLI` even when the Codex CLI is missing; the first Enter opens OpenAI connection methods, which doesn't match the header.
- **Change:** Until a route is configured, the header shows the route as `(not connected)` and the first Enter opens the full provider list with the cursor on the first ready route, else the current task route. Never auto-select or connect. (List order is kept so existing keyboard paths stay valid.)
- **Accept:** Fresh HOME with and without CLIs installed; the saved preference wins once set; no network or discovery beyond what exists today.

### UX16 — Advisory suggestions that fit the moment

- [x] Stop suggesting a specialist on a fresh project before any work happened.
- **Priority:** 2. **Needs:** —. **Builds on:** suggestion table (DESIGN_RULES §Suggestions).
- **Problem:** In manual mode, typing the first request on a new project shows `Suggested: Jolteon — Observed source changes have no complete run at this revision.` because existing files count as "changes".
- **Change:** Require an actual Eevee-observed source change (after a mediated write or a revision change since the last run) before that rule fires.
- **Accept:** Fresh project shows no suggestion; after an applied write without a run it does; other rules unchanged.

### UX17 — One map of game nouns

- [x] Explain moves, TMs, held items, formations, recipes, ribbons and the bag in one place.
- **Priority:** 3. **Needs:** GL02. **Builds on:** MV/PK/PT tracks.
- **Problem:** Several overlapping nouns each have their own commands and screens; users can't tell which one to use.
- **Change:** A game-terms section in `/help` with one line each: what it is, what it is for, the one command to open it. Link from the `/move` and `/formation` empty states (the bag waits for GL02).
- **Accept:** Every listed command opens; text fits 60×18 with scrolling; no new behavior.

### Dev hygiene — keep the progress driver current

- [x] Update `scripts/progress_smoke.py` for auto-adopted todos and the default-on native todo prelude (0.1.35). It is not in the CI profile, so it is currently stale.
