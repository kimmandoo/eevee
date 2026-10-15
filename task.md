# Tasks

Index of open work. Each section lives in its own file under [`tasks/`](tasks/); every ID is one small, separately committed unit. Completed specs were removed on 2026-10-15 and remain in git history (`git show fadc6f4:task.md`), `docs/WORK_CHECKPOINT.md` and `CHANGELOG.md`. Contract: [design rules](docs/DESIGN_RULES.md). Architecture: [implementation plan](docs/IMPLEMENTATION_PLAN.md). Adapters: [agent compatibility](docs/AGENT_COMPATIBILITY.md).

| File | Section | Open |
|---|---|---|
| [A-party.md](tasks/A-party.md) | Party: free form switching and cooperating partners | 15 |
| [B-partner-basics.md](tasks/B-partner-basics.md) | Partner basics and form suggestion | 9 |
| [C-relays.md](tasks/C-relays.md) | Form relays | 8 |
| [D-moves.md](tasks/D-moves.md) | Moves: teaching and learning skills | 14 |
| [E-partner-record.md](tasks/E-partner-record.md) | Partner record and flavor | 13 |
| [F-context-tools.md](tasks/F-context-tools.md) | Context, token economy and tool calling | 19 |
| [G-ux-platform.md](tasks/G-ux-platform.md) | UX, platform and release | 7 |
| [H-provider-acceptance.md](tasks/H-provider-acceptance.md) | Provider acceptance (blocked) | 6 |
| [R-code-structure.md](tasks/R-code-structure.md) | Code structure | 8 |

## Status

- **Released:** 0.1.3 (tags v0.1.0–v0.1.3). Schema 7 store.
- **Done:** the 25 original tasks plus PT01 (party contract), PT02A (partner table / Schema 7), PT02B (lead-partner accessor), PT03A (named form commands), PT03B (previous-form toggle), PT04A (party roster) and the first code-structure pass (index below). **Open:** 99 small tasks. **Dropped:** T05.
- **Blocked:** section H needs permitted Claude credentials and completed Gemini authentication.
- **Partially implemented:** T01 (local receipt), T03 (in-packet dedup), A04/A06/A07 adapters.

### Completed index (referenced by "Builds on")

V01 sanctuary layout · P01 native terminal lifecycle · V02A/V02B species art and native graphics · V03 evolution picker · UX01 focus/editing · E01 durable goal · E02 task cards/resume · E03 task lifecycle/privacy/quota · C01 repository baseline/diff · C02 Vaporeon source map/basket · R01 Jolteon approve/run/cancel · R02 execution journal/freshness · UX02 interruption/recovery · P02 filesystem/process parity · A01 agent preflight · A02 Espeon planning request · A03 Flareon implementation · A05 OpenCode ACP · U01 Umbreon hypotheses · L01 Leafeon refactor comparison · G01 Glaceon review desk · S01 Sylveon handoff · S02 curated memory (Bond) · S03 Evolution Stone recipes · PT01 party contract (DESIGN_RULES § Party: ≤6 partners, ≤3 working per task, one writer) · PT02A/PT02B partner table and lead accessor · PT03A/PT03B named form commands and previous-form toggle · PT04A party roster view · RF03 `app.rs` split.

## How to pick work

- Take the first ready task in **Recommended order**. A task is ready when every ID in **Needs** is done; **Builds on** lists completed work to reuse.
- One task = one actual-TUI demo at 120×36 and 60×18 (or the stated check for non-UI work), its tests, a changelog/checkpoint update and one commit. A mock, a build-only change or an inactive control is not completion.
- Reuse the existing modules (see README's source-layout table); no parallel dispatcher, executor, recipe parser or agent runtime. Put new store tables in their own `src/store/<name>.rs`.
- Keep keyboard-only operation, draft/focus preservation, cancel/quit, terminal restoration and honest evidence. Navigation, paste, suggestions, relays and form switches never execute, approve or send anything.
- When suggestion, ranking or learning rules change, report the observed behavior on fixtures.

### Cross-cutting watch-outs

- **Schema:** The store is Schema 7 (partners). PT02A reserved `partners.nickname` and `tasks.partner_drafts` so waves 1–2 share that one bump. Every later persisted addition needs a migration plus the byte-preserving old-binary refusal test; never rewrite data so an older binary can start.
- **Packets:** Partner flavor (nicknames, partner names, move names, dialogue, ribbons, routes) never enters provider packets or handoffs. The receipt must show zero flavor bytes.
- **Facts first:** Pokémon-style copy wraps a factual line (command, exit code, permission scope), never replaces it. Monochrome, ASCII and linear layouts keep the same information.
- **No fake progression:** No XP, levels, affection, streaks, hunger or random unlocks. Raising partners means learned moves, nicknames and real records. Add each new concept (Moves, Dex) to DESIGN_RULES when it ships.
- **Party concurrency:** Partners never start work on their own or chain another partner's work. Every event, approval and journal entry carries its partner; each provider request keeps its own review and receipt; one writer per task.
- **Rights:** Canonical move names are labels; Q01's naming/art-rights review must cover them before the next distribution.
- **Local only:** Detection, ranking and learning use local data. No flavor or optimizer feature makes a network or model call.

## Recommended order

| Wave | Tasks (in order) | Why this order |
|---|---|---|
| 1. Party foundation | PT02A → PT02B → PT03A → PT03B → PT04A → PT04B → PT05A → PT05B → RF05 → PT06A → PT06B → PT06C | Model and Schema 7, then switching, roster, drafts, and finally request routing (still one working partner). |
| 2. Partner basics | EV04 → EV03A → EV03B → EV07 → PK01 → EV01A → EV01B → EV02 | Small UI slices reused later. |
| 3. Form relays | EV10A → EV10B → EV14 → EV12 → EV13A → EV13B → EV15 → EV16 → PT10 → EV01C | Highest daily value; PT10 lets relays target a party member. |
| 4. Cooperation | PT08A → PT08B → PT07A → PT07B → PT07C → PT09 → PT04C | Write lease before concurrency; queue and cancel-all before the board. |
| 5. Moves | MV01A → MV01B → MV02A → MV02B → MV03A → MV03B → MV06 → MV04 → MV05 → MV07 → MV01C → MV03C → MV08A | Teach → use → learn → name → forget/relearn → record. |
| 6. Partner record | EV05 → EV06 → PK02 → EV08A → EV08B → PK05A → PK05B → EV09 (after T01A) → EV17 → EV11 | Uses attribution, moves and relays from earlier waves. |
| 7. Optional flavor | PK03 (after UX04B) → PK04 (after C03A) → PK06 → PT11 | Polish; drop anything that starts to feel like gamification. |
| Context/token track | C03A → C03B → T01A → T01B → T01C → T02A → T02B → T03A → T03B → T04 · TC01A → TC01B → TC03A → TC03B → TC04 · TC02 → TC05 · D01 · Q03 last | Independent of the waves; can interleave. |
| UX/platform track | UX03, A08 → UX04A, UX04B → S04 → MV08B → P04A → P04B | UX04B unblocks PK03; S04 and MV08B share one export path. |
| Code structure | RF01, RF02, RF04, RF06, RF07, RF08, RF09 | Each right before the feature that next touches that file. |
| Blocked | A04, A06 → A07 → Q01 → Q02 → P03 | Resume only after the user prepares permitted authentication. |
