# V — Audit: weak or ambiguous implementation

Recorded 2026-10-28 from a whole-project review after the 0.1.35–0.1.37 work (code, CI, smoke drivers, classifier probes and actual PTY captures). Each item is a real gap found in the current source, not a speculative cleanup. Not assigned to a wave.

Rules for every V task: keep existing safety contracts (explicit review, no implicit send/run/grant, truthful evidence); prove the change with a focused test plus an actual-binary journey where the behavior is visible; update DESIGN_RULES when a contract changes.

## Verification and release reliability

### VA01 — Run and repair the 64 opt-in smoke drivers

- [ ] Only 8 of 72 `scripts/*_smoke.py` run in CI. The rest are run by hand per release and drift silently (0.1.35 already broke `conversation_flow_smoke.py` on a moved header line and `progress_smoke.py` on auto adoption).
- **Priority:** 1.
- **Change:** Run every driver against the current release binary, repair the stale ones, and add a cheap CI job (Linux, one size) that at least imports each driver and runs its `--plan-only`/offline subset where one exists. Record which drivers need credentials or long runtimes.
- **Accept:** A table of all 72 drivers with pass/fail/needs-credentials; no driver silently depends on removed text.

### VA02 — Stop smoke drivers from reading SQLite during writes

- [ ] CI linux-x86_64 failed once in the coverage step and the local equivalent failed on `sqlite3.OperationalError: database is locked`. 0.1.35 raised read timeouts from 1 s to 10 s, which hides rather than fixes the race.
- **Priority:** 1.
- **Change:** Give drivers one shared read helper that retries on `SQLITE_BUSY` with a bounded deadline and records each retry, or read through an exported snapshot. Remove per-driver ad-hoc connections.
- **Accept:** 20 back-to-back runs of the full native profile with `--jobs 4` show no lock failures; retries are visible in results.

### VA03 — Check the release notes before tagging

- [ ] Release run 40640741692 failed after a green native run because CHANGELOG had no `## <version>` section; moving the unpublished tag then needed a force-push, so 0.1.35 was skipped and a stale remote `v0.1.35` tag remains on `ad5c634`.
- **Priority:** 1.
- **Change:** In the Check job, fail when `Cargo.toml`'s version has no matching `## <version>` CHANGELOG section with at least one entry. Decide whether to delete the orphan `v0.1.35` tag (needs explicit approval).
- **Accept:** A missing section fails Check on the main push, before any tag; documented in TROUBLESHOOTING.

### VA04 — Native qualification of 0.1.35–0.1.37 surfaces

- [ ] Bag, Berry consumption, item notices, live-file flashing, `[Pinned]`/`a` toggle and the first-run provider list were verified with unit tests and Linux PTY captures only. A real provider dispatch consuming a Berry, and Windows ConPTY/macOS rendering of item sprites and the blinking caret, were not exercised.
- **Priority:** 2. **Relates to:** Q01, P03.
- **Accept:** Controlled-peer journey: attach a file, make it a Berry, Send, observe the receipt includes it once and the basket no longer does; native captures on all three OSes.

## Intent, todo and evolution

### VA05 — Questions should not default to full write tools

- [ ] Unclassified requests ("설명해줘", "Why does the build fail?", "Explain how the planner works", "UI 개선해줘") resolve to Implement with every file/command tool. Each effect still needs approval, but a pure question should not look like a coding run, and "UI 개선해줘" (a real change request) is not recognized either.
- **Priority:** 1. **Builds on:** EV18.
- **Change:** Add explicit question/explanation intents (explain, why, how, what, 설명, 왜, 어떻게, 알려줘) → Plan with read tools; extend change verbs (개선, 바꿔줘 without object, improve) → Implement. Keep the full-tools fallback only for genuinely unknown imperative text.
- **Accept:** Bilingual classifier fixtures; evolution preview and dispatched operation agree.

### VA06 — Todo and game state must not invalidate the context plan

- [ ] `optimizer::Facts::from_task` hashes the entire `workflows` JSON, so every auto-adopted todo revision, Berry toggle or dismissed move proposal changes the basis and drops an applied `/context-apply` packet before the next Send.
- **Priority:** 1. **Builds on:** T01, GP09.
- **Change:** Hash only the workflow fields that actually enter provider context; leave progress-status churn, Berries' presentation flag and dismissals out (the Berry's attachment itself is already part of the basket identity).
- **Accept:** Apply a context plan, receive two todo updates and toggle a Berry; the plan stays applied. Changing a field that enters context still invalidates it.

### VA07 — Bound automatic todo churn

- [ ] In Auto mode every provider todo snapshot becomes a new adopted revision. A chatty provider can create dozens of revisions per run, each saved and each changing the next evolution.
- **Priority:** 2. **Builds on:** GP09.
- **Change:** Coalesce snapshots within one run into one revision at settlement (keep live display), and show the revision diff once.
- **Accept:** A fixture emitting 20 snapshots yields one durable revision per run; display still updates live.

### VA08 — Disclose the default todo prelude's cost

- [ ] Since 0.1.36 every native request carries the phased-todo prelude by default. Its bytes are in the receipt, but its token cost and effect on short chats are unmeasured.
- **Priority:** 3. **Relates to:** Q02/Q03.
- **Accept:** Measured byte/token overhead per route; decide whether short conversational requests skip it.

## Game loop

### VA09 — Moves cannot take parameters

- [x] Repetition grouping only matches case/whitespace/punctuation variants because request moves have no slots. "Fix the bug in src/a.rs" and "… src/b.rs" never become one move.
- **Priority:** 2. **Builds on:** GL01, MV09.
- **Change:** Add optional typed slots (path, quoted text) to request moves with explicit fill-in at use time; extend grouping to slot-equal instructions. Portable TM schema needs a version bump.
- **Accept:** Three path-varied Sends propose one parameterized move; using it asks for the slot value; old TMs still import.
- **Completed (2026-10-29, XC07):** typed exact-span/default parameters, guarded similarity, observed-variant teaching review, JSON-array fill and full substituted-instruction review before ordinary outgoing consent. TM2 retained parameters; literal TM1 imported unchanged. Both terminal sizes passed real three-variant learning, neutral cancellation, exact other-draft/caret retention, actual substituted wire, library32/replacement/four quick slots and explicit no-replay reconnection after restart in `target/xc-038-final-c-20261029/moves`.

### VA10 — Move proposal coverage check uses a possibly stale inventory

- [ ] `move_proposal` checks "already a move" against `self.moves.records`, which is only loaded for the owner last opened in `/move`. After restart, a learned move can be proposed again until `/move` is opened.
- **Priority:** 2. **Builds on:** GL01.
- **Change:** Load the lead/form inventory with the records snapshot (or keep a per-owner digest set) before offering.
- **Accept:** Restart after learning; no duplicate offer.

### VA11 — Item notices overwrite each other

- [ ] There is one notice slot; a move learned right after a Berry is used replaces the Berry notice before it is seen.
- **Priority:** 3. **Builds on:** GL05.
- **Change:** A small FIFO (at most three) shown one at a time.
- **Accept:** Two events in one frame are both shown in order.

### VA12 — Item sprites only render as cells

- [ ] Item sprites use cell rasters everywhere, including terminals with native graphics where species art uses the graphics protocol; the bag shows a sprite only at ≥70 columns.
- **Priority:** 3. **Builds on:** GL05.
- **Accept:** Native-protocol item rendering where species art already uses it; compact bag shows a small icon column.

## Connections and context

### VA13 — Say when a reviewed tool selection was left out

- [ ] Since 0.1.35, a `/tools` selection for another adapter/task/partner is silently excluded from the request. Users can think tools were sent when they were not.
- **Priority:** 2.
- **Change:** Show "Tools selected for <route> were not sent; /tools to reselect" in the preview/receipt and as a one-time notice after the switch.
- **Accept:** Switch provider with a selection; the notice and receipt line appear; nothing is blocked.

### VA14 — First-run default route

- [ ] UX15 opens the full provider list on a first Send, but the default task route is still `Codex/CLI` even when no Codex CLI is installed, so the header starts as `Codex/CLI (not connected)`.
- **Priority:** 2.
- **Change:** With no saved preference, show the route as `Not connected` until the user picks one; never auto-select.
- **Accept:** Fresh HOME with and without CLIs; saved preference unchanged once set.

## Rights and naming

### VA15 — Rights review for item names and sprites

- [ ] `ASSET_LICENSES.md` records the item sprite source, hashes and boundary but, like the species sprites, is not clearance. Potion, Berry, TM and Bag names were added to the UI.
- **Priority:** 2. **Relates to:** Q01.
- **Accept:** Extend the distribution review with the item names and sprites, or replace them with neutral names/icons before any commercial distribution.
