# C — Form relays

One form's stored output pre-fills the next form's reviewed input. A relay only pre-fills: approval, execution and sending still go through the normal review, and every pre-filled item shows its source. After PT10, every relay can also target an idle party member.

### EV10A — Failure location parser

- [x] Extract `file:line[:col]` locations from retained run output.
- **Priority:** 1. **Needs:** —. **Builds on:** R02.
- **Change:** A small pure function with fixture tests for rustc/cargo, Python tracebacks, Node/TypeScript and generic `path:line` output. The run view lists the parsed locations.
- **Accept:** Real failing `cargo test` and pytest outputs list the right locations; unparseable output lists none.
- **Watch out:** Only the retained bytes are parsed (output is bounded), and the view says so. PK05 and TC03 reuse this parser.
- **Verified:** Actual failing Cargo and pytest runs listed `src/lib.rs:3` and `test_failure.py:3` at 120×36 and 60×18; unparseable output listed none. Pure fixtures covered Rust thread IDs, Python, Node/TypeScript, spaces and bounded deduplication. `target/wave3-final-execution-relays/results.json`.

### EV10B — Pin a location

- [x] Turn a listed location into pinned context in one action.
- **Priority:** 1. **Needs:** EV10A. **Builds on:** C02.
- **Change:** `Pin range` on a location captures a bounded window through existing capture identity and exclusion checks, recording the run as provenance.
- **Accept:** The pinned range shows its run reference in the basket. Out-of-root, excluded and missing paths are refused with a reason.
- **Verified:** Both sizes' reviewed capture and persisted basket retained the originating run; out-of-root, `.env` and missing references refused without adding pins. The same actual execution-relay artifact recorded all cases.

### EV14 — Rerun a stale check

- [x] Refresh outdated verification with one reviewed action.
- **Priority:** 1. **Needs:** —. **Builds on:** R01, R02.
- **Change:** Wherever a run is stale, `Rerun with Jolteon` opens the normal command review pre-filled with the same argv/cwd. The result is a new run linked to the old one.
- **Accept:** Edit a source after a passing run, rerun and approve; both runs remain with distinct revisions. Denying starts nothing.
- **Watch out:** Never update or relabel the old run, and never reuse an old approval.
- **Verified:** Both sizes used ordinary home `/refresh` after editing a source, then cancellation, denial and a new explicit approval. The new linked run had a distinct revision; the original record stayed unchanged and denial started nothing.

### EV12 — Hypothesis → Flareon fix

- [x] Start a fix from Umbreon's conclusion.
- **Priority:** 1. **Needs:** EV02. **Builds on:** U01, A03.
- **Change:** On a supported hypothesis, `Fix with Flareon` switches the lead to Flareon (or sends to a Flareon partner after PT10), shows the EV02 strip, and pre-fills the requirement with the hypothesis and evidence plus proposed editable paths.
- **Accept:** The draft cites the hypothesis; paths still need approval; cancel leaves Umbreon state unchanged; unresolved or refuted hypotheses show a disabled reason.
- **Watch out:** Proposed paths respect A03's limits (1–8 files ≤12 KiB, POSIX-only controlled writes); show a limit hit instead of trimming silently.
- **Verified:** Both actual sizes refused an unsupported hypothesis, preserved the sender's Unicode draft/caret and hypothesis state on cancellation, and staged cited evidence plus proposed scope for an idle Flareon. A separately reviewed real Eevee callback then wrote only the selected file through the controlled route; no relay itself sent or granted anything.

### EV13A — Finding → Flareon fix

- [x] Start a fix from a Glaceon finding.
- **Priority:** 1. **Needs:** EV02. **Builds on:** G01, A03.
- **Change:** `Fix with Flareon` on a request-changes finding pre-fills the requirement and scope from the finding and its evidence.
- **Accept:** The draft cites the finding, and cancelling leaves review state unchanged.
- **Verified:** Both sizes staged the individually selected request-changes finding and exact source-range references; cancelling preserved the review state. Recipient drafts were explicitly cleared through normal UI between independent relay cases.

### EV13B — Addressed findings need re-review

- [x] Track which findings a new diff touched.
- **Priority:** 1. **Needs:** EV13A. **Builds on:** G01.
- **Change:** A diff touching a finding's ranges marks it `addressed, needs re-review`, makes the verdict stale and offers returning to Glaceon with a fresh context.
- **Accept:** Of two relayed findings, change one; only that one is marked. No finding closes without a new verdict.
- **Watch out:** The re-review context stays independent of the implementation conversation; prefer a partner that did not author the patch.
- **Verified:** Both sizes relayed two distinct ranges, changed only the first, retained `[addressed, unaddressed]`, staled the review and left both resolutions open. A fresh managed review stayed independent; external edits did not inherit the earlier controlled author's label.

### EV15 — Decision → acceptance criteria

- [x] Make a chosen design checkable.
- **Priority:** 2. **Needs:** —. **Builds on:** A02, E01.
- **Change:** `Add as criteria` on an Espeon decision lists its items as editable criteria linked to the decision. Duplicates are flagged, and nothing changes until confirmed.
- **Accept:** Add three and edit one, then restart; the criteria persist with the link and the met/total count updates. Cancel adds nothing.
- **Watch out:** Provider-proposed text stays unverified until the user confirms it.
- **Verified:** Both sizes explicitly chose an actual loopback proposal, edited one of three new criteria, flagged a duplicate, cancelled without adding anything, confirmed three unchecked linked items and restored the same origins after restart. Met/total became 1/4 only after an explicit checkbox action; native prose claiming a write did not create patch authorship.

### EV16 — Existing run as a Leafeon baseline

- [x] Reuse a run as the "before" side of a refactor comparison.
- **Priority:** 2. **Needs:** —. **Builds on:** L01, R02.
- **Change:** Offer recent passing runs at the pre-refactor revision with the same argv as the baseline, showing age and revision. Choosing none falls back to a fresh baseline.
- **Accept:** The comparison names both runs; a mismatched revision or argv is excluded with a reason.
- **Watch out:** Show both durations; one sample is never a speedup claim.
- **Verified:** Both sizes selected an eligible retained pass with age/revision, rejected old revisions and changed argv, compared distinct actual before/after runs and opened the fresh command fallback without execution. Both run IDs, both actual durations and the no-speedup warning were keyboard-accessible, including compact paging (`target/wave3-final-workflow-relays/results.json`).
