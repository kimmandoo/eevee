# Changelog

## 0.1.8

- feat(ux): sent ordinary requests directly from Enter after durable preparation, while retaining scoped approval for commands, writes and risky connections.
- fix(providers): grouped Codex native responses and CLI tools under one provider entry with explicit connection methods.
- feat(models): persisted last-used provider/model/effort in Schema 10, restored exact per-route choices, and removed the forced effort dialog after model selection.
- fix(ux): kept first-send preparation through pending context restoration and prevented cancelled model reviews from reopening an invisible input-owning picker.

## 0.1.7

- feat(providers): added native ChatGPT, OpenAI, Anthropic, Gemini and compatible-service discovery and streamed responses without installed agent runtimes.
- fix(models): separated discovery from exact configured/custom selection, made reasoning metadata optional, and retained previous catalogues on failed refreshes.
- feat(models): added local model search and exact-ID entry, and kept native service credentials and endpoints separate from external-agent controls.
- fix(agents): removed SDK/ACP model-list membership gates, accepted acknowledged custom IDs, and exposed SDK reasoning as requested values without claiming applied effort.
- fix(config): kept native user-scoped provider sources usable when launched from the verified home directory without relaxing other project-source or file-trust checks.

## 0.1.6

- fix(providers): exposed Codex executable/catalogue provenance, preserved configured defaults and showed complete newer-model choices without hardcoded names or silent fallback.
- feat(commands): added reviewed `/reset` with durable draft retention and `/init` for editable, separately approved project-guide proposals.
- fix(ux): restored the home partner, repaired fresh-home model loading and flattened local selection navigation.
- fix(responses): removed duplicate loading indicators and kept previous output distinct from current request state.

## 0.1.5

- feat(ux): replaced Goal-first setup with durable request input and streamed responses directly into the home workspace.
- feat(review): centered compact approval summaries with fixed actions, retained larger scrollable details, and guarded y/n and allow-once a shortcuts on visible scope without end-scroll gates.
- feat(store): migrated to Schema 9 for independent per-partner request drafts while preserving task facts and refusing older writers.
- feat(providers): replaced exact CLI/SDK release lockouts with reviewed installed identity and protocol/policy checks, and added explicit latest-stable setup/update paths.
- fix(requests): preserved committed input throughout active admission and permission checks, retained follow-up drafts, and kept previous response provenance distinct.

## 0.1.4

- feat(ux): shortened navigation, added fluent response streaming and activity indicators, and preserved draft/follow state across inspection.
- perf(tui): coalesced background saves, bounded event processing, synchronized frames and removed per-input Windows full-screen clears.
- feat(party): added persistent partner selection, form switching, a roster and per-partner drafts with transactional Schema 8 migration.
- fix(smoke): verified portable terminal resize, storage durability and published-version upgrade behavior without weakening safety gates.

## 0.1.3

- feat(auth): Devin website OAuth (authorization code + PKCE) with a memory-only session credential injected only into the launched `devin acp` child.
- feat(context): outgoing packets deduplicate repeated and overlapping captures (`same_bytes_as`, hoisted `source_root`), Codex edit baselines cite identical captures, and receipts disclose the local savings.
- fix(acp): oversized ACP tool observations are truncated with exact omitted-byte counts instead of aborting the provider session; session credentials are refused for non-Devin providers.
- fix(ci): clippy `-D warnings` passes on the pinned 1.90.0 toolchain.
- refactor(src): modules grouped by role (`store/`, `workspace/`, `providers/`, `platform/`), `store.rs` and `app.rs` split by responsibility, unit tests moved into sibling `tests.rs`. No behavior change.
- docs: party contract in DESIGN_RULES; backlog reorganized into `tasks/` with 105 small tasks.

## 2026-10-16

- feat(party): attributed new command runs and provider journal records to the requesting partner with an id plus name/form snapshot, showed it in approval, run history and journal views, and kept older records byte-identical as `partner unknown`.
- fix(install): reused the installation animation during self-update release lookup and preserved UTF-8 locale settings, monochrome motion and static redirected output.
- feat(models): compacted model selection into model → supported reasoning → retained editor, highlighted current choices separately from focus, retained model/effort in compact home headers, and moved complete metadata/errors behind optional details.
- feat(models): ranked exact matches first, preserved search when returning from reasoning, and added direct exact-ID and supported-effort command arguments without sending drafts.
- fix(ux): activated ready providers directly, retained explicit setup and owned-context reviews, and removed the intermediate connection drawer after local selection or cancellation.
- feat(context): added direct bounded attachment review/save/return, visible saved-file markers and path/range summaries, and direct range, search and neutral removal controls.
- fix(context): kept attachment filtering local, accepted scoped case-insensitive abbreviations and spaces, prevented path slashes from opening commands, and preserved compact error/status visibility.
- chore(release): published v0.1.8 from `53f3fd0` after Check and all five native rows passed, verified eight public assets and seven hashes, and passed 23 installation checks, 34 installed terminal scenarios, 33 workbench journeys, real model/effort restart/responses and Schema 9→10 downgrade-refusal checks.
- chore(release): published v0.1.7 from `0a43c0e` after Check and all five native build/test/install rows passed, verified eight public assets and seven checksums, and passed 23 installation checks, 34 installed terminal scenarios, 31 public workbench journeys and actual no-CLI GPT-6.1/default-model responses.
- feat(ux): made composer Enter save and send qualified ordinary requests without a second popup, kept actual effect approvals, and prevented repeat/modified Enter from submitting.
- fix(providers): presented one Codex provider with native/CLI methods while retaining distinct tool, credential and session boundaries.
- feat(models): added acknowledged durable per-route model/effort and last-used provider preferences, safe Schema 9→10 migration, explicit startup-route precedence and retryable save failures; model selection returned without a forced effort dialog.
- fix(ux): waited for matching local readiness on first direct sends and kept asynchronous metadata cancellation from stranding a hidden picker over the home editor.
- feat(providers): added native scoped HTTP connections with immutable request review, cancellable streaming, user-scoped endpoint configuration and no credential copying or implicit tool execution.
- fix(models): removed stale CLI-list membership and stable-release-only admission barriers, preserved exact configured/custom IDs and optional effort defaults, and made catalogue refresh failure nondestructive.
- feat(models): added searchable model pickers, safe custom-ID editing and scrollable native-provider selection on compact terminals.
- fix(smoke): made loopback fixture socket mode explicit on Darwin, created Windows security fixtures with atomic owner/DACL attributes, and asserted cancellation effects instead of transient notice wording.
- fix(agents): decoupled SDK/ACP catalogue membership from acknowledged model selection, preserved auth/tool-policy checks, exposed requested SDK effort, and required the updated revision-3 bridge.
- fix(config): corrected home-directory startup so private user provider configuration could load while other project-source boundaries and file-trust checks stayed enforced.

## 2026-10-15

- chore(release): published v0.1.6 from `ef9cef4` after exact-commit Check and all five native build/test/install rows passed, verified all eight public assets and seven checksums, and passed 23 installation checks, 34 installed terminal scenarios and 23 workbench journeys against the public macOS package.
- fix(smoke): applied the existing POSIX shell-session ownership guard to self-update as well as bootstrap, preserving macOS terminal-restoration evidence after installer exec.
- fix(codex): read effective nonsecret model configuration, preserved configured defaults through catalogue revalidation, exposed installed CLI provenance and update guidance, and kept full native model/effort choices inspectable on compact screens.
- feat(commands): added neutral `/reset` with a durable-save barrier and recoverable old tasks, plus `/init` proposal drafts with explicit replacement review and no implicit execution or file writes.
- fix(ux): restored the persistent home partner card, returned model/provider selections directly to their invoking surface, and removed intermediate role/form navigation steps while preserving drafts.
- fix(providers): loaded model catalogues before task creation, retained visible loading/error pickers and retry paths, isolated metadata from task content, and accepted omitted optional Codex input modalities using the protocol default.
- fix(responses): consolidated live progress in the header, gave compact responses more usable rows, and kept previous partial output distinct from current streaming/model state.
- chore(release): published v0.1.5 from `6e43e6b` after exact-commit Check and all five native build/test/install rows passed, verified all eight public assets and seven manifest checksums, and passed 23 installation lifecycle checks plus 34 installed terminal scenarios on both Linux and native Windows.
- fix(runtime): completed deferred cancel-and-exit immediately after owned-process and journal acknowledgment instead of blocking for another key on an idle screen.
- fix(smoke): checked compact outgoing context against committed capture facts without pinning JSON key order, removed an obsolete navigation-cycle expectation, and kept native helper builds on the configured local target.
- feat(providers): accepted newer stable provider releases through observed identity and capability validation, switched Gemini to current `--acp`, and refreshed the optional Claude SDK through explicit latest setup while retaining its policy controls.
- fix(requests): deferred follow-up persistence until active-request acknowledgment, recovered pre-output failures without discarding typed follow-ups, and preserved per-response request ownership across retained turns.
- fix(review): rejected hidden source-purge shortcuts after resize and required the current target/effect summary to render before approval.
- fix(ux): retained saved task context before the first response, restored invoking focus when closing popups, and preserved specialist overlays and their underlying editor across restart.
- feat(ux): made the first screen accept requests directly, prepared reviews only after durable saves, preserved task facts across follow-ups, and returned approved responses to the full-width home workspace.
- feat(review): centered command and attachment choices, exposed concise effect summaries and fixed approval controls, removed mandatory end-scroll gates, and restricted y/n/a shortcuts to visible, current, non-editing confirmation scopes.
- feat(store): added durable per-partner Request drafts and Schema 9 migration with byte-preserving old-binary refusal.
- chore(release): published v0.1.4 from `6489264` after exact-commit Check and all five native build/test/install rows passed; verified all eight public assets and checksum-covered files.
- fix(smoke): preserved existing installer checksum entries when exercising published manifests, instead of introducing duplicates in the local release fixture.
- fix(store): coalesced implicit draft, caret and navigation saves after 500ms of quiet with a 2s deferral bound, preserved immediate explicit saves and transition/exit flushes, and separated pending edits from in-flight storage and stale acknowledgments.
- fix(terminal): synchronized screen clears, text and native-image updates, released frames on errors and terminal lifecycle transitions, removed per-input Windows full-screen clears in favor of cell differences, and avoided unchanged image placement on spinner-only frames.
- perf(runtime): bounded worker-event batches to preserve input responsiveness, slept until deferred-save deadlines instead of polling idle storage, and retained cached response layout across activity ticks.
- fix(smoke): measured settled idle after deferred saves, refreshed startup readiness without losing raw-mode evidence, retained short-lived helper resource attribution after exit, and made native resize/draft checks independent of optional ConPTY hints and OSC filtering.
- feat(ux): flattened specialist and editor entry points, added multiword action search and visible blockers, kept drafts visible beneath the command chooser, and unwound repeated overlay visits instead of accumulating Back steps.
- feat(streaming): displayed credential-safe partial words before newlines across Codex, ACP and Claude, reduced SDK snapshot batching, and prioritized response text with cached wrapping, pauseable follow, new-output notices and inspectable provenance.
- feat(tui): added observed-stage activity spinners with a static `--no-animation` option, stopped animation for local decisions and idle screens, removed hidden workspace rendering behind provider responses, and corrected the live Escape cancellation hint.
- feat(providers): loaded the model catalogue automatically — a successful task-provider connection, or opening `/model`/`/effort` with nothing cached, runs the metadata-only catalogue inspection without a review page (it waits for a saved task, and preview blockers stop it with a one-line reason); the picker reopens once models arrive. `/models-refresh` stays the reviewed manual refresh.
- fix(ux): trimmed popups and repeated text — a committed, clean task exits without the Confirm exit popup; composer hints appear once (footer, bottom row) instead of in both the composer and the footer; a notice clears on the next key; `/help` became a short key/command table; task cards show relative update times and only true archive/masked flags; run, criteria, forms and inspector text was shortened, and the inspector's removed letter shortcuts (e/j/z) were replaced by slash commands.
- feat(party): retained per-(task, partner) drafts and exact carets, named the composer owner, and restored each partner's own previous-form memory. Existing drafts transferred to the task lead; discard reviews affected only the current draft. Schema 8 enlarged the reserved bounded draft column without reducing existing text limits.

- fix(ux): goal Enter stays on the composer — the automatic post-save action chooser is removed; `/` and Ctrl+K still open the same searchable choices deliberately, and the save surfaces a short "Goal saved" notice. The `Popup.after_goal` machinery and the save-failure popup dismissal it needed are gone; DESIGN_RULES first-use contract updated, and the PTY smoke now asserts no chooser after a goal save.

- feat(party): choose the task lead partner — `/lead <name>` resolves a nickname or `Eevee #n`/`#n`/bare id and bare `/lead` opens the roster, where Up/Down now select a row and Enter/`l` (or the contextual `/lead`) fronts that partner directly, no extra confirmation. `request_lead` swaps `App.lead` to the partner's stored row so the header and the form picker follow immediately, and the choice persists through the normal task save (`tasks.lead_partner` + the partner's own `form`). The toggle memory resets per lead (`previous_form` is the old lead's), and a guard refuses a switch while a party write is in flight or the current lead still owns live work — the hook PT06's per-partner slots will keep.

- feat(party): add read-only roster view — `/party` (and the companion drawer hint) opens a centered list of every partner: `Eevee #n` or nickname, current form, a derived `idle / working / waiting for approval` status and the task it is engaged with. `App::party` mirrors `Project.partners` on load/manage events, the lead row overlays the live form before a save lands, and `partner_status()` derives state from the single planning/execution slot so PT06/PT07 fill it in. No request fires and nothing is stored.
- feat(party): add new partners to the roster — the roster's `a` key and `/add` action queue `store::PartyRequest::Add` on the store worker; `State::add_partner` inserts an Eevee-form partner with a real `met_at` and the current project inside an Immediate transaction that re-checks the six-seat cap, so a seventh is refused with "at most 6 members" before anything commits. AUTOINCREMENT ids are never reused after a release, so `validate_partner`/`Snapshot.lead_partner` bounds relax to positive ids. `Event::Party` reports the committed roster; `party_request`/`party_inflight` prevent a second add while one is in flight, and the `/add` choice carries its disabled reason at capacity.
- feat(forms): previous-form toggle — `set_form` records the lead partner's prior form (persisted as `navigation.previous_form` in the task's existing bounded JSON, no schema bump), and Backspace on any non-text surface plus `/back-form` flip back through the persisted `SelectForm` path; inside text fields Backspace stays a plain delete, and the stored previous form survives a restart.
- feat(forms): named form commands — `/eevee`…`/sylveon` plus element aliases (`/normal`, `/water`, `/electric`, `/fire`, `/psychic`, `/dark`, `/grass`, `/ice`, `/fairy`) switch the lead partner's form through the same persisted `SelectForm` path as the picker; typed commands retain the composer draft and caret, pasted slash text stays literal, and a unit test pins the command+alias namespace free of duplicates.
- feat(store): Schema 7 adds a `partners` table plus `tasks.lead_partner` and a reserved `tasks.partner_drafts` (PK01 nickname and PT05B per-partner drafts share this one bump). Migrating a Schema 6 store derives partner #1 in the oldest task's form with a derived met date; `scripts/schema_upgrade_smoke.py` proves the Schema-6 binary refuses the new store byte-for-byte.
- refactor(app): the active form is now the task's lead partner (`store::Partner`) read through `App::form()`/`set_form()`; the header renders `Eevee #1 · <form>` via `lead_label()` (nickname-ready for PK01), and the stored form is the partner's form after restart.
- feat(install): redrew the install/update Poké Ball as 16×16 pixel art in half blocks (red shell with highlight, thin black band, white shell, lit button) with a caught-ball shake. POSIX uses 256 colors, Windows uses console colors; NO_COLOR draws monochrome blocks; non-UTF-8 locales and console code pages without block glyphs fall back to an ASCII ball. The POSIX installer now remembers the user's locale before forcing `LC_ALL=C`.
- refactor(app): split the single `impl App` in `app.rs` (2,328 lines) into `app/events.rs`, `app/tasks.rs`, `app/keys.rs`, `app/editing.rs` and `app/overlays.rs`; `app.rs` keeps the state types, constructor and accessors. No behavior change.
- refactor(src): grouped modules by role into `src/store/`, `src/workspace/`, `src/providers/` and `src/platform/` (artwork moved under `src/ui/`); split the 7,139-line `store.rs` into `tasks`, `memories`, `journal`, `runs`, `spool`, `schema`, `admission` and `recovery` submodules; moved the inline test modules of large files into sibling `tests.rs` files. No behavior change: 375 tests pass, 2 ignored, as before. README gained a source-layout table.
- docs(design): confirmed the party contract (PT01): up to 6 partners, up to 3 working on one task, one editing partner per task, per-partner requests/approvals/drafts, no autonomous partner work, flavor never sent to providers. Marked as pending implementation.
- docs(tasks): split the backlog into `tasks/A–H, R` section files with `task.md` as the index, and broke open work into 105 one-commit tasks (e.g. PT02 → PT02A/B, MV01 → MV01A/B/C), each with Needs / Builds on / Watch out. Added section R (RF01–RF09) to keep splitting large source files right before the features that touch them.
- docs(tasks): added party section PT01–PT11 for individually named partners that switch forms freely and cooperate on one task (contract update, Schema 7 partner model, instant switching, roster, lead switch with per-partner drafts, partner-routed requests, concurrent partners, single-writer edit lease, party board, partner-to-partner handoff, formations); revised EV/PK/MV tasks so moves, nicknames, routes, pairings and ribbons belong to individual partners and the carry-over card became a non-blocking strip.
- docs(tasks): reorganized `task.md` into open work only: removed the 25 completed task specs (kept as a one-line index and in git history) and the dropped T05; every open task now lists open `Needs`, completed `Builds on`, and `Watch out` cautions; added cross-cutting watch-outs (Schema 7 batching, flavor never in provider packets, no fake progression) and a wave-based recommended order.
- docs(tasks): added Pokémon-style partner tasks: moves taught and learned from repeated work with four slots per form, battle-style move menu, forgetting and Move Reminder, type-themed move names, observed move records, TM export/import (MV01–MV08), plus nickname, summary screen, Bug Dex, evolution transition, held items and ribbons (PK01–PK06).
- docs(tasks): extend section J with eight unimplemented form-relay tasks (EV10–EV17): pin failure locations from run output, form-ranked `@` candidates, Umbreon hypothesis → Flareon fix, Glaceon finding → Flareon fix → re-review, rerun stale checks at the current revision, Espeon decision → acceptance criteria, Leafeon baselines from existing runs and remembered per-form provider pairing. Updated the backlog to 25/66.
- docs(tasks): specify section J's nine unimplemented Eevee-identity tasks (EV01–EV09): evidence-based form suggestion, evolution carry-over card, form capability chip with owning-form redirect, form-specific empty states, party status from real exit evidence, non-evidence form route history, return to Eevee, deterministic partner lines for real events and bond proposals from repeated instructions. Updated the backlog to 25/58 without changing release gates.
- feat(context): deduplicate repeated/overlapping source captures inside an outgoing packet. Basket assembly emits `{source_root, captures:[…]}` — the shared repository root is hoisted once, `source_sha256` is hex, and a capture byte-covered by a wider capture of the same file revision carries a `same_bytes_as` citation to a text-bearing entry instead of resending identical bytes; every basket id still resolves, and identical text from a different path or revision is never conflated.
- feat(codex): `edit_baseline` entries cite a byte-identical whole-file `selected_sources` capture (`working_content_same_as_capture`) instead of embedding the file twice in one request.
- feat(receipt): the request receipt and basket drawer disclose local deduplication — citation count in the receipt lines, saved bytes in the payload detail — always labelled local-only, never as provider cache or billing.
- fix(acp): an oversized ACP tool observation no longer aborts the provider session; `rawOutput`/`content` fields are bounded to a labelled head/tail slice with `truncated_by_eevee`, `total_bytes` and exact `omitted_bytes`, preserving a bounded journal entry and completion state.
- fix(acp): a session credential attached to a non-Devin provider is now a preview blocker instead of being silently injected; injection itself is additionally gated on the Devin profile.
- fix(agents): a live Devin website credential now reports `Connected (session credential)` only while unexpired — the status checks the stored JWT expiry at render time instead of trusting the flag set at login.
- fix(auth): the Devin token exchange reads at most 64 KiB of response JSON, so a malformed or endless token endpoint cannot grow process memory while the login waits.
- test: cover capture dedup ordering/chains/revision separation, baseline citation eligibility, truncated tool snapshots and receipt disclosure.

- feat(auth): add an Eevee-owned website OAuth route for Devin (authorization code + PKCE against server.codeium.com; loopback callback on localhost, or pasted redirect/code for remote sessions). The returned Windsurf session credential lives only in process memory: it is never serialized, journaled or shown, its Debug output is redacted and its bytes are scrubbed on drop. Eevee injects it as `WINDSURF_API_KEY` into the launched `devin acp` child environment only, where it takes precedence over the CLI's stored credentials; the JWT `exp` minus a five-minute skew bounds injection, and expiry surfaces as a launch blocker on the next review. A successful native Devin CLI login clears the in-memory credential.
- feat(runner): add `spawn_piped_env` so reviewed launches can pass child-only environment overrides that are not recorded in the command spec; on Windows the overrides merge into a fresh sorted UTF-16 environment block passed with `CREATE_UNICODE_ENVIRONMENT`, so a stale or absent parent PATH/PATHEXT never reaches the child.
- fix(agents): keep in-session connection outcomes across `Refresh installed providers` and surface `Connected (session credential)` while a live website-OAuth credential can satisfy Devin ACP launches; `/connect` now syncs the hub's active provider like `/provider` so Model/Effort choices reflect the task provider.
- fix(planning): close an open model/effort picker and drop pending selection changes before clearing authentication state so a stale catalogue cannot survive a credential change; when a refreshed catalogue no longer advertises the selected model, the notice now names the stale selection instead of silently keeping it.
- fix(auth): report a misparsed authorization-code paste (`code` present without `state`, or `state` without `code`) as a Failed login instead of exchanging a malformed request, and reject non-JWT tokens or JWTs without a numeric `exp` as implausible before they reach a child environment.
- docs: document the website-login contract and credential handling in `docs/DESIGN_RULES.md` and the Devin row/README sign-in note.

## 0.1.2

- feat(providers): `/provider` and `/connect` open one hub that marks the task provider (●) with its model and effort, adds Choose model / Choose reasoning effort, and preselects the likely next step (Connect → Use for task → Choose model). Every connection, switch and request review is unchanged.
- feat(models): pickers open on the current value (or Refresh when nothing is inspected), mark it `(current)`, and choosing a model continues straight to its advertised efforts.
- refactor(providers): share provider identity through `Agent::index/from_index/command/from_command` and one `connectable` check instead of hard-coded lists and index comparisons.
- feat(tui): explanations are brief by default — the first sentence plus any `Next:` step, marked `…`; `/verbose` toggles full text. The header shows `provider · model · effort`. Consent reviews stay complete.
- fix(sprites): crop all nine forms to one shared box so they render at a single scale; keep the portrait visible while the `/` palette or `@` menu is open (the palette now stays left of the companion rail, so no Sixel full-screen erase).
- feat(sprites): `--images auto` falls back to cell art once the terminal is known to lack Sixel/Kitty; color cell art uses quadrant blocks with each cell's own two original colors, doubling horizontal detail.
- fix(install): document a Windows install that works under the default Restricted execution policy (script-block one-liner, or process-only `-ExecutionPolicy Bypass`); accept relative `-Archive`/`-Checksums`/`-InstallDir`; print the exact user-PATH command. README gives per-platform macOS, Linux/WSL, Windows and offline steps.

## 0.1.1

- feat(commands): replace action-letter, numeric and F-key shortcuts with one searchable slash shell and contextual provider/role/action choices. Goal Enter saves locally and opens neutral next steps; drafts/caret/paste, storage acknowledgement, busy/resize and once-only consent boundaries remain intact. Ctrl+K opens the same shell.
- feat(models): add provider-scoped model and reasoning pickers backed by reviewed official catalogue inspection. Snapshot selection into request consent, require live advertised model and setting acknowledgments, and offer Keep current/New session/Back for incompatible owned sessions. Inspection sends no task prompt, grants no resume ownership and consumes no model-turn budget.
- fix(claude): require bridge revision 2 before spawning runs; qualify explicit models through public SDK controls before prompt release. Refuse explicit effort when the SDK cannot acknowledge the effective pre-prompt setting after organization caps, rather than silently accepting a downgrade.
- test(commands): migrate acceptance journeys to semantic commands and add regressions for literal drafts, neutral popups, stale scope/save/busy gates, provider-scoped selections, catalogue bounds, unsupported choices and model/effort acknowledgments.
- feat(devin): add pinned Devin CLI 3000.11.3 as the fifth ACP provider with reviewed task switching, negotiated session inspection and native browser OAuth, explicit manual-token login and status. Credentials remain provider-owned; login never replays tasks. Validate the CLI release separately from its observed internal ACP agent version.
- feat(token-economy): show a local, offline request receipt in every provider preview. It splits the exact Eevee-sent payload into instructions, task facts, attachments, handoff and memories with exact bytes and a labelled `eevee-local-v1` CJK-aware estimate; provider-added context, usage and billing stay unknown, never zero. Source-basket envelopes use the same estimator.
- revert(token-economy): drop per-task budgets (`--max-provider-turns`, `--max-commands`, `--max-input-tokens`) and the repeated-command pause; the local request receipt stays.
- feat(claude): add the Anthropic Console OAuth route through the official `ant` CLI (browser or paste code, plus status). Claude API billing only; claude.ai subscription OAuth stays refused. The sidecar admits that profile exclusively and verifies the effective backend before releasing a prompt.
- test(auth): add an `--ant` Claude case to the live auth smoke.
- ci: run fmt/Clippy/script tests/SDK policy once on Linux and, per native row, tests, release build, packaging and an offline install check. PTY journeys and the lifecycle upgrade fixture move to local scripts; release qualification now requires `Check` plus five `Build` rows.
- docs(readme): shorten to quick start, commands, providers, install and development; details live in `docs/`.

## 2026-10-14

- docs(tasks): require local-only token optimization with no optimizer-initiated network/model calls or downloads; replace provider-cache-dependent T03 with local packet deduplication, remove model summarization from T04, keep TC01 discovery offline and require zero-attempt outbound verification. Reviewed user work and explicit live benchmarks remain separate.
- docs(tasks): specify twelve unimplemented token-economy and tool-calling improvements: usage receipts, budgeted symbol context, session deltas, evidence-linked compaction, budget/loop stops, lazy tool discovery, reviewed dependency plans, paged results, validated observation caching, failed-step recovery, impact-directed checks and paired quality/cost benchmarks. Record adapter limits, priorities, acceptance and the updated 25/49 backlog without changing release gates.
- fix(tui): shorten routine home/composer/status/help copy, reclaim a workspace row and keep action hints on deliberate lines; use compact connection pages and scrollable setup details without reducing consent or error information.
- feat(installer): replace the one-line spinner with a four-line Poké Ball wobble/button pulse and separate phase output in POSIX and PowerShell install/update; preserve monochrome, resize, plain-output and cursor-restoration behavior.
- docs(tasks): add six prioritized future improvements for searchable actions, attachment previews/budgets, connection diagnostics, readability/reduced motion, recipe authoring and reviewed update/rollback eligibility; keep them explicitly unimplemented and retain the 0.1.1 authentication hold.
- fix(ci): let the source-payload capture helper recognize double dialogue frames; preserve exact envelope/byte-count checks and all source/stored-reference revalidation scenarios.
- docs(auth): record fresh Claude SDK permitted-route and native Linux/Windows Gemini ACP authentication blockers, passing local handoff/section-F regressions, and the user's explicit 0.1.1 authentication-preparation hold.
- docs(release): describe online sprite-cache preparation accurately without changing exact-commit qualification or bundling image bytes.
- fix(sprites): replace automatic character-cell mosaics with negotiated original-pixel Sixel/Kitty rendering; preserve integer scale, omit too-small images, clear overlays correctly and show the renderer in F4. Verify the nine-form surface in actual Windows Terminal 1.24.12741.0 on WSL2.
- fix(input): keep graphics discovery and user input under one bounded reader, preserving typing/paste across split or late replies and resize; retain cell previews only through explicit `--images cells`.
- feat(tui): add a handheld-game-inspired sage/cream palette, square/double dialogue borders, field log, larger partner status card and evolution party styling without changing command or approval semantics.
- test(auth): verify actual Codex device authorization initiation/cancellation on WSL with callback port 1455 deliberately occupied; document the observed Windows reserved-port collision and preserve the user's system networking configuration.
- fix(sprites): use all nine pinned original pixel sprites with nearest-neighbor rendering and a separate cache; online installers verify/download them before publishing the executable and first launch loads them locally.
- feat(tui): replace the dashboard-first home with a restrained task/activity workspace, compact companion and full-width bottom composer while preserving narrow layouts and approval controls.
- feat(composer): add filtered `@` file attachment and `/` action pickers, including first-draft attachment, full-file capture review, persistent attachment counts and approval-gated `/run`.
- fix(auth): recommend official Codex device authorization on WSL/SSH, retain explicit browser login, and explain localhost callback recovery without silent authentication retries or credential capture.
- fix(ci): isolated ConPTY child standard handles from redirected runner pipes and moved native transport regressions before compilation.
- fix(ci): waited for actual bounded process exit when ConPTY cleanup raced an already-terminating Windows Job, retaining real termination errors.
- fix(ci): waited for committed task-card navigation before approving native task switches, preserving pending-save refusal and all ownership checks.
- feat(workflows): added evidence-linked diagnosis, bounded baseline/refactor comparison, independent read-only review and resumable unresolved findings.
- feat(handoff): included selected hypotheses and unresolved findings in reviewed portable handoffs without transferring grants or declaring completion.
- feat(memory): added explicitly approved task/project/personal notes with exact-content previews, separate outgoing-context selection and shared-memory-safe task deletion.
- feat(recipes): added strict versioned TOML previews and explicit plan-only application without running commands or expanding permissions.
- fix(storage): released the writer lock explicitly despite inherited descriptors and prevented late first-insert acknowledgements from resurrecting a cleared draft.
- test(acceptance): added native specialist/memory/recipe journeys, real independent-review checks, schema-upgrade refusal and measured release-performance workloads.
- fix(ci): based native quota-recovery fixtures on actual schema cost while retaining real exhaustion, draft-preservation and explicit-reclamation checks.
- chore(release): published 0.1.0 from five-target qualified immutable packages with both installers and combined checksums, and verified anonymous downloads plus the public installed schema-6 workbench.

## 0.1.0

- feat(ux): show the Goal composer and caret immediately; retain drafts across F1 help, F2 connections, F3 evolution, F4 images and the action palette.
- feat(connections): replace hidden shortcut-driven setup with provider/action/method lists, compact neutral consent, optional diagnostics and a visible reviewed “Use for task” action. Login remains separate from model submission.
- fix(artwork): trim transparent padding, preserve proportions and native alpha, composite cells against their actual panel surface without background boxes, and retain backing colors in ASCII mode. Invalidate native caches after reload; add explicit background download, progress, cancellation, missing-file resume and local reload for all nine forms.
- feat(distribution): add pinned HTTPS release installation, `ev update --check`, embedded-installer updates and owned uninstall with user-data preservation. Verify executable and legal-file ownership; retain rollback and locked-file refusal.
- feat(installer): show an original rolling Poké Ball text animation during interactive installation/update, with monochrome and plain-output modes and cursor cleanup.
- fix(platform): use Git's portable `/dev/null` isolation on Windows, accept only verified macOS system temporary-directory aliases, and decode SS3/CSI F1–F4 keys without turning them into typed text.
- ci(release): separated tag publication from native build and interactive smoke, and promoted exact-commit qualified archives with both installers and combined checksums.
- ci: move pinned workflow actions to their Node 24 majors, optimize SHA-256 in test builds so runner admission stays within bounded deadlines on loaded macOS hosts, and make process-tree fixtures beat before spawning descendants.
- docs: add release installation/update/uninstall instructions, replace the README sprite with a verified alpha-transparent illustration, and separate original code/text animation from third-party images, names and rights.

## 2026-10-13

- fix(ci): preserved split UTF-8 bytes in native Windows ConPTY smoke capture, replacing the lossy pywinpty transport without relaxing help-return surface equality.
- fix(ci): stopped hidden provider discovery from repainting the idle home, waited for complete saved terminal frames before idle checks, and retained failing-output diagnostics.
- ci(release): separated five native build jobs from installed-package smoke jobs and tag publication; required successful same-commit main-push qualification and immutable artifact provenance without rebuilding or repeating release smoke.
- ci: retained five native credential-free installation/TUI rows, added installed JSON/Markdown handoff and isolated Node 24 SDK policy coverage, pinned Rust dependency/tool caching with main-only saves, disabled persisted checkout credentials, and avoided duplicate branch-push/PR runs. OAuth and real provider/model smoke remain opt-in.
- feat(auth): added reviewed official Codex browser/device OAuth and status, OpenCode auth chooser/inventory, and explicitly external Gemini native Google/API-key/Vertex login without automatic task replay; removed the mixed-protocol ACP auth route after reproducing real OAuth output contamination.
- fix(terminal): restored native auth foreground/input ownership, neutral consent and stale-generation boundaries; removed cursor-response dependence when repainting after terminal handoff.
- feat(claude): supported explicit Bedrock bearer/web identity and opt-in AWS SSO/profile/workload or Vertex ADC chains, with credential-helper/network effects disclosed, isolated SDK configuration and effective-backend validation; claude.ai subscription OAuth remains prohibited.
- fix(providers): preserved authoritative terminal outcomes after cancellation, bounded concurrent approvals, rejected session/tool identity rewrites, serialized SDK writes with post-consent identity validation, and retained complete/partial journals without duplication or frame-loss.
- fix(workbench): separated session metadata from selectable decisions, made metadata Escape cancel unfinished inspection, kept unknown outcomes sticky, reset stale consent to Deny, guarded export/quit against pending work, and preserved exact imported decisions without duplicate labels or truncation.
- fix(storage): masked sanitized/legacy provider observations, rejected unsafe provenance, preserved execution recovery metadata and migration ID high-water marks, excluded provider credential filenames, and refused FIFO/reparse executable substitutions without hanging.
- docs(readme): redesigned the project entry point with an attributed pinned Eevee pixel sprite, custom geometric wordmark with light/dark exports, concise navigation, quick start, workflow/auth tables, collapsible references and truthful support limits; no decorative emoji or unsupported status badges.
- docs(licensing): separated Apache-2.0 original code/prose/wordmark from third-party Pokémon artwork and marks, recorded the pinned sprite's source/hash/upstream copyright, and incorporated project NOTICE into generated native-package dependency notices without packaging artwork.
- feat(implementation): added Codex selected-file `eevee_edit_file` replacement with exact dirty-baseline patch consent, anchored POSIX protected/link/stale rejection, provider-native schema-5 journals, diff review and separately chosen local checks.
- feat(agents): added optional pinned Claude Agent SDK 0.3.296 / CLI 2.1.296 TypeScript bridge with mandatory tool/config policy and permitted API/cloud auth; shared ACP1 OpenCode 1.18.0 / Gemini 0.28.0 transport with negotiated permissions/load/list and explicit external-tool boundaries. SDK/Gemini authenticated conformance remains credential-blocked.
- feat(handoff): added strict version-1 create-only JSON/Markdown review/import, idle provider/role switching with original-session retention and grant reset, unverified imported claims and dispatch-time task/source binding.
- fix(agents): made blocked-preview cancellation terminate cleanly, withheld generated provider-session provenance from all outbound task metadata, reconciled ACP native tool journals only at final/partial boundaries, and allowed Gemini's measured Node startup a disclosed bounded ten-second version probe.
- feat(planning): added pinned Codex 0.152.1 app-server Plan turns with exact context/trust previews, last-moment task/source/config revalidation, provider-owned account/model checks, correlated questions and one-time approvals, explicit resume/cancellation, and reviewed provenance-labelled Next action decisions.
- fix(planning): rendered consent controls after last-page review state updates, so reaching the end immediately shows the neutral available action rather than a stale locked footer.
- feat(agents): added versioned Codex/Claude SDK/ACP preflight profiles, passive discovery, neutral executable/configuration trust previews, bounded owned version probes, and explicit version/authentication/adapter/control blockers without model requests or credential-store access.
- fix(platform): removed a Linux-only redundant device-ID cast while retaining macOS unsigned artifact identity semantics.
- feat(context): added persisted source ranges, conservative exclusions, exact payload previews, and source/retained-reference revalidation without claiming provider isolation.
- feat(execution): added explicit argv/shell approval, quota-backed owned-process execution, bounded sanitized output, and cancellation-aware task/form/quit transitions.
- feat(evidence): persisted execution journals and exposed stale, ambiguous, interrupted, truncated, and unavailable results without replaying commands.
- fix(ux): retained search and nested-overlay focus, pinned compact capture warnings, preserved below-minimum cancellation, and reported post-commit cleanup failures without fictional rollback.
- feat(repository): added bounded read-only repository inspection, immutable dirty baselines, raw-byte identities and diffs, unattributed external changes, and non-Git task preservation without invoking repository helpers.
- feat(storage): added reversible task archival, neutral scoped deletion and retention preview, before-storage credential masking, and configurable SQLite quotas while preserving failed-save drafts.
- feat(tasks): added durable project-scoped task cards, criteria and next actions, per-task drafts/navigation, committed timestamps, guarded switching, and explicit completion/reopen transitions with transactional schema migration.
- docs(distribution): recorded five successful native installation rows, each with ownership/upgrade/failure-preservation checks and 34 installed-binary terminal scenarios; separated the subsequent account-billing CI blocker from code failures.
- feat(storage): persisted project goals and selected forms through a bounded SQLite worker, correlated committed and failed revisions, preserved unsaved edits on failure, and added single-writer exclusion, transactional migration/crash recovery, and read-only export.
- fix(installer): restored the prior Windows executable from its backup even when ReplaceFile reported failure after moving it, and retained recovery files when rollback failed.
- feat(distribution): added Apache-2.0 native verification packages, checksum-first offline POSIX/PowerShell installation, ownership-protected replacement, and installed-binary lifecycle coverage; withheld public Release and remote bootstrap.
- feat(cli): changed the executable and command-line contract to `ev` while preserving project/cache identities.
- docs(platform): recorded completed section-A acceptance and five verified native core baselines, including 34 Windows ConPTY scenarios and qualified graphics/provider/distribution limits.
- fix(windows): repainted state-changed console frames to prevent indexed-color blank cells from retaining prior editor/overlay text, while keeping idle rendering event-driven.
- test(platform): added native ConPTY alternate-buffer restoration checks using preserved primary-screen contents instead of requiring an unrelayed external VT marker.
- fix(windows): composed displayed console graphemes without changing editable text and invalidated reflowed console buffers on resize before repainting.

## 2026-10-12

- test(platform): synchronized bounded ConPTY captures with queued output and ordered resize notifications, retained strict overlay restoration checks, and preserved exact transcript newlines.
- fix(windows): preserved combining/astral text and literal bracketed paste through a bounded single-owner console-input decoder, and restored the exact original input mode on every terminal exit path.
- build(rust): raised the minimum toolchain to Rust 1.90 to match the pinned image-rendering dependency graph.
- perf(ui): reused borrowed goal lines and untruncated header text during redraws while preserving pinned actions and bounded scrolling.
- ci(platform): added pinned native macOS/Linux/Windows build, test, Clippy, and real PTY/ConPTY lifecycle jobs with host-qualified evidence artifacts and owned-process cleanup.
- feat(companion): delivered the sanctuary, pinned nine-species artwork cache, image-derived cells and qualified native Kitty rendering, deliberate grid/list handoffs, linear keyboard navigation, bounded scrolling, and safe Unicode draft/exit interactions; fixed escaped-image row width and nonregular-asset startup hangs.
- docs(platform): planned native macOS, Linux, and Windows build/runtime/distribution gates; expanded the backlog to thirty-one tasks; decoupled local UX and storage from unavailable graphics/platform runners; tightened persistence, task lifecycle, retention, cancellation, and request-preview acceptance.
- docs(ux): specified focus, editing, accessibility, approval, cancellation, and recovery contracts; separated evolution roles from execution agents; planned capability-qualified Codex, Claude Agent SDK, and ACP integration with reviewed cross-agent handoffs and twenty-seven incremental tasks.
- docs(design): refined the companion journey, sanctuary composition, and twenty-one dependency-ordered tasks, including actual species artwork, native terminal images, and source-derived cell fallbacks.
- feat(tui): added the Rust evolution workbench shell with nine forms, responsive elemental themes, keyboard navigation, and Unicode-safe session-local goal editing.
- fix(terminal): avoided cursor-position queries during fullscreen initialization and restored terminal modes on normal, error, and panic exits.
