# E — Partner record and flavor

Pokémon flavor wraps facts and never replaces them. No XP, levels, affection, streaks or random unlocks (DESIGN_RULES § Bond).

Summary, factual dialogue, Dex, pairings, attachment ranking, held references and ribbons are complete; see [COMPLETED.md](COMPLETED.md). The existing one-time evolution popup is not PK03's optional motion implementation.

### PK03 — Evolution transition

- [ ] Add an optional picker-only transition to the released evolution receipt.
- **Priority:** Pokémon milestone 4. **Needs:** UX04B. **Builds on:** EV02, EV18, V02B.
- **Existing foundation:** v0.1.19 already shipped a localized static one-time popup for real manual/automatic changes, with safe deferral and input forwarding. Do not add a second popup or reimplement automatic evolution.
- **Change:** Enhance an explicitly identified picker switch with a non-flashing transition lasting at most 300 ms. Reuse the actual species/reason receipt; automatic changes and text-only/reduced-motion routes keep complete static results. Update the current no-evolution-timer contract only when this bounded optional behavior is implemented.
- **Accept:** Actual English/Korean compact/wide journeys cover rapid switches, resize, deferred/unseen transitions, same-form selection, restart and neutral return to Eevee. Never queue or replay transitions. A key or paste skips motion and reaches its original editor exactly once with draft/caret/focus intact; it cannot answer consent. Persistent reduced motion and `--no-animation` render no animated frames; no idle redraw remains afterward.
- **Watch out:** Approval/waiting-owner facts and live generated code retain priority. Clear intersecting native images and restore them once; do not animate the idle portrait or replace factual state with effects. Screen-reader support requires separate actual assistive evidence.

