# L — Game loop: learned moves, bag items and item sprites

Requested 2026-10-28: make everyday use feel more like the game. Repeated similar requests can become a learned move; a bag holds Potions, Berries and TMs; item sprites appear with the existing popup surfaces. Not assigned to a wave yet. GL03 (Poké Ball route capture) was dropped on 2026-10-28 at the user's request; its ID is not reused.

These tasks extend shipped foundations (moves, teaching, portable TMs, held references, ribbons, evolution popup). They do not rebuild them. Rules for every L task:
- **Game skin, real action:** every item maps to an existing reviewed action. Using an item never sends, runs, approves, grants, changes criteria or skips a review on its own. Flavor names, sprites and inventory never enter provider packets, receipts' provider bytes or portable handoffs.
- **Learning is still explicit:** repetition may *propose* a move; only the existing teach review plus a committed store acknowledgment changes inventory (DESIGN_RULES "Teach requires actual explicit committed learning").
- **No progression mechanics:** no XP, levels, scores, streaks, random drops, timers or gated features. Repetition thresholds and item quantities are evidence/inventory facts, not progress bars.
- **Rights first:** item names and sprites are franchise content. Source sprites from the same pinned PokéAPI revision as species art (`sprites/items/…`), download them only into the user cache, never bundle them, keep text/ASCII fallbacks, and extend the existing naming/art rights review (Q01) before publication.
- **Contract change required:** DESIGN_RULES forbids "celebration popups" and decorative motion. L04 must update that rule to allow a static, dismissible, non-blocking item notice on a real event (like the existing `Evolution!` notice). It must never cover consent, live file code or the composer.
- **Storage:** new persistence starts from current Schema20 under the existing admission/migration/older-writer-refusal contract.

### GL01 — Propose a move from repeated similar requests

- [ ] Notice when the user keeps sending essentially the same instruction and offer to teach it as a move.
- **Priority:** 1. **Needs:** —. **Builds on:** MV01A, MV04, MV09, EV18 (classifier), GP09 (todo).
- **Reuse:** dispatched outgoing receipts already retain the exact reviewed envelope and owner per Send, so read candidates from them instead of storing a second request history. `/teach-move` already teaches from "the last actually submitted user-authored instruction" (`App::move_teaching_proposal`) and names it with `moves::suggested_name`; the proposal opens that same path. Store only dismissed pattern digests, in task workflows like `dismissed_suggestion` (nonempty values are a strict-reader change; record it).
- **Change:** After an explicit Send, compare the submitted user-authored instruction with this partner/form's recent submitted instructions in the same project. Use deterministic local normalization (case/whitespace/punctuation, quoted literals and paths kept as slots, classifier form/operation equal). When ≥3 distinct Sends within the retained window match and no active move already covers it, show one dismissible proposal: "Eevee wants to learn *<name>*". Accept opens the existing `/teach-move` review prefilled with the instruction template, suggested name, slots and scope. Dismiss suppresses that exact pattern for the task (like `/suggestion-dismiss`).
- **Accept:** English and Korean fixtures at 120×36 and 60×18: three similar Sends propose once; different form/operation, negated/read-only variants and provider output never count; accepting still needs name/scope review and store acknowledgment; dismissal survives restart; a full 4-move inventory routes through the existing forget/replace review. No proposal mid-run, during approvals or with a pending question.
- **Watch out:** Only actual dispatched Sends count; never provider output, memories, drafts or imported TMs. Store only normalized pattern digests plus the candidate template, not raw history copies. No auto-learning, no counts shown as progress.

### GL02 — Bag: one inventory for items and TMs

- [ ] Add a `/bag` surface that lists the partner's usable items by pocket.
- **Priority:** 1. **Needs:** RF04. **Builds on:** MV08B (TM import/export), PK04 (held references), PK06 (ribbons), UX17 (game-terms help).
- **Change:** Pockets: **Items** (Potion, Berry), **TMs** (existing portable TM files and learned-move export), **Key items** (held reference, read-only view). Each row shows name, sprite or text icon, quantity where meaningful, and the exact action it opens. Enter opens that action's existing review; `d` shows details. Keyboard-only, 60×18 list layout and ≥90-column grid.
- **Accept:** Bag opens/closes preserving exact draft, caret and focus; every row's action matches the underlying command; empty pockets explain how to obtain items and link `/help` game terms; add `Bag` to that help map and to the everyday palette set only if it stays one 60×18 page; no row executes or grants anything; actual 120×36/60×18 journeys.
- **Watch out:** Inventory is local to project/partner and never portable except TMs through the existing export. Do not duplicate TM storage; the TM pocket reads the existing records.

### GL04 — Potion and Berry: reviewed recovery and one-shot context

- [ ] Map Potion to recovery of a failed/unknown step and Berry to a one-shot held context.
- **Priority:** 2. **Needs:** GL02. **Builds on:** TC05 (failed-step recovery), journal retry, PK04/PK08 held references.
- **Change:** **Potion** appears when the lead has a failed command run, storage-retry, or recoverable connection failure; using it opens exactly the existing review for that case: `/rerun` (new approval), `/plan-resume`, `/storage-retry`, `/move-retry`/`/records-retry`, or the connection diagnostics page (new approval for any rerun). **Berry** is a user-made one-shot held context: pick a memory/file/note, and the next explicit Send for that partner includes it once, shown as a removable chip and in the outgoing receipt; it is consumed only after dispatch acknowledgment.
- **Accept:** Potion never reruns or resends by itself; unknown outcomes keep their existing guards; Berry inclusion is visible before Send, included exactly once, restored if the Send is cancelled before dispatch; restart preserves unconsumed Berries; 120×36/60×18 journeys.
- **Watch out:** No HP or health metaphor beyond the label; failure stays shown as failure. Berry content is ordinary user-selected context under existing privacy/exclusion checks.

### GL05 — Item sprites with event notices

- [ ] Show item sprites in the bag and in a static notice when a real item/move event happens.
- **Priority:** 2. **Needs:** GL02, RF04, rights review. **Builds on:** species artwork pipeline (`/images`, cache install, nearest-neighbor rendering, mono/ASCII fallback), evolution popup placement rules.
- **Change:** Extend the artwork installer/cache to an item set (Potion, TM by type, a few Berries) from the pinned PokéAPI revision `sprites/items/`. Bag rows and the notice render the sprite at small integer scale, with a text icon fallback (`[TM]`, `+`). Notices: move proposed/learned (GL01, existing teach), Berry consumed (GL04), TM imported (MV08B). A notice is static, dismissed by the next real key without consuming it, never covers consent/live code/composer, and is suppressed under `--no-animation`/reduced motion only for motion, not content.
- **Accept:** Sprites render in true-color/indexed/mono and graphics-protocol terminals with matching surface background; offline/no-cache shows text icons; notices appear once per real committed event and never for previews, cancellations or replays; update DESIGN_RULES (popup and artwork sections) and `ASSET_LICENSES.md`; actual 120×36/60×18 screenshots.
- **Watch out:** Do not bundle images in the binary, source tree or packages. Rights review before public release; source availability is not clearance.
