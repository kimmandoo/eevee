# Active tasks

Only unfinished work belongs here. Completed specifications and stable dependency IDs are in [tasks/COMPLETED.md](tasks/COMPLETED.md); dated execution/release evidence remains in [WORK_CHECKPOINT.md](docs/WORK_CHECKPOINT.md) and [CHANGELOG.md](CHANGELOG.md). Contract: [design rules](docs/DESIGN_RULES.md). Provider boundaries: [agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Current state

- **Released:** [v0.1.21](https://github.com/kimmandoo/eevee/releases/tag/v0.1.21), removing everyday navigation detours while preserving reviews, live-task ownership and drafts. Exact-source native qualification and public asset verification passed; detailed runtime/installation evidence is recorded in the checkpoint.
- **Compatibility:** Schema 18 and complete SDK bridge 9, unchanged from v0.1.20. Back up the full data directory before upgrading from an older schema; an in-place downgrade is not backup restoration.
- **Remaining:** **38 tasks**. Pokémon completion has **three product features plus one enabling accessibility task**; the other 34 follow that milestone. UX03 is complete, not a new implementation task.
- **Next implementation when requested:** **PT11 — Formations**. This reorganization does not itself authorize implementing the remaining backlog or running paid/live model trials.

## Pokémon completion first

| Order | Task | Deliverable and genuine prerequisite |
|---|---|---|
| 1 | [PT11](tasks/A-party.md) | Save, preview and reapply a party formation using existing recipes and owned partner state; preserve drafts and never start work. Ready now. |
| 2 | [MV08B](tasks/D-moves.md) | Portable TM export/import for every supported move kind, explicit local teaching and fresh use-time review. Reuse existing create-only publication; the missing reviewed portable-export slice belongs here, not behind all of S04. Ready now. |
| 3 | [UX04B](tasks/G-ux-platform.md) | Persistent reduced motion plus explicit installer/update opt-out, extending the existing `--no-animation` behavior. Finish the whole task rather than leaving an unnamed installer remainder. |
| 4 | [PK03](tasks/E-partner-record.md) | Optional bounded picker transition on the existing one-time evolution receipt. Needs UX04B; static text, approval priority and input preservation remain mandatory. |

**Milestone exit:** All four acceptance sets pass, including actual English/Korean 120×36 and 60×18 journeys, restart, neutral cancellation, busy ownership and no implicit send/run/grant. Party, moves, summary, Dex, held references, ribbons, automatic form selection and the static evolution popup are already implemented; do not rebuild them. No XP, levels, streaks, random unlocks or provider access gates are added.

## Remaining tracks

| File | Unfinished scope | Open |
|---|---|---:|
| [A — Party](tasks/A-party.md) | Formations | 1 |
| [D — Moves](tasks/D-moves.md) | Portable TM files | 1 |
| [E — Partner record](tasks/E-partner-record.md) | Optional evolution transition | 1 |
| [F — Context and tools](tasks/F-context-tools.md) | Context comparison/receipts, deterministic selection, tool artifacts/plans and measured optimization | 16 |
| [G — UX and platform](tasks/G-ux-platform.md) | Reduced motion, remaining diagnostics, density, workflow export, update review and rollback eligibility | 6 |
| [H — Acceptance](tasks/H-provider-acceptance.md) | Live-provider rows, composed quality/resource measurements and native distribution evidence | 6 |
| [R — Structure](tasks/R-code-structure.md) | Responsibility-based, just-in-time extractions | 7 |

B (partner basics) and C (relays) are entirely completed and have moved into the archive, not the active sequence.

### After the Pokémon milestone

1. Finish everyday platform extensions: A08 → UX04A → S04 → P04A → P04B. Reuse existing setup/updater/export owners; S04 adds workflow authoring, not another portable-file writer.
2. Context track: T01B → T01C → T02A → T02B → T03A → T03B → T04. Completed T01A/C03A/C03B remain foundations, not pending gates.
3. Tool track: TC01A → TC01B; independently TC03A → TC03B → TC04; then TC02 → TC05 → D01 → Q03. Respect each task's exact `Needs`; Q03's live trials require separate authorization.
4. Reconcile local Q01 visual/state rows, Q02 current measured resource workloads and P03 supported-target install/upgrade evidence. They are not collectively blocked on provider credentials. Their outstanding authenticated rows remain explicitly incomplete.
5. Run A04/A06 when permitted Claude credentials and eligible Gemini authentication are available; then finish full A07 and the dependent composed-provider Q01 rows. Implemented transports and deterministic fixtures are not live acceptance.
6. Perform RF01/RF02/RF04/RF06/RF07/RF08/RF09 immediately before a feature genuinely needs the same extraction, not as a prerequisite cleanup wave. Local priorities in track files never outrank Pokémon completion.

## Execution rules

- Take the first ready task in the ordered milestone. `Needs` names unfinished prerequisites; `Builds on` names completed foundations in the archive. External prerequisites are explicitly identified in H.
- Complete a coherent task with behavior-focused tests and an actual runtime demonstration, update changelog/checkpoint, and commit. A large responsibility extraction may use sequential commits; an arbitrary file count is not acceptance.
- Reuse existing dispatch, recipes, shared file policy, store writer and provider transports. No parallel executor, parser, receipt authority or approval queue.
- Preserve exact draft/caret/focus, cancellation, terminal restoration, source identity and truthful evidence. Navigation, imports, suggestions, formations and relays never execute or grant permission by themselves.
- Keep partner flavor out of provider packets and portable handoffs; local imported definitions must not manufacture historical evidence. Missing usage remains unknown, not zero.
- Report fixture observations when suggestion, ranking or learning rules change. Keep local deterministic work separate from paid/live provider acceptance.
- New persistence must start from current Schema 18 and use the existing admission/migration/older-writer-refusal contract; do not revive historical Schema 7/14 planning.
- Preserve the existing naming/art distribution review and its non-clearance limitation. New asset/name changes require a scoped rights review; Q01 still owns complete-product acceptance. T05 remains dropped, not completed.
