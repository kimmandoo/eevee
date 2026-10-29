# X — Contract relaxation: deliberate changes that buy flexibility

Requested 2026-10-29: identify functional-flexibility work that is blocked by an existing contract and specify what each relaxation would look like if the contract were intentionally changed. Planning only; nothing is implemented. Not assigned to a wave.

Each task below **breaks a binding rule on purpose**. It names the exact rule, the proposed replacement wording, the remaining safety floor and the evidence needed. An X task is not "done" until its DESIGN_RULES amendment, code, tests and runtime journey land together.

## Rules for every X task

- **Amend first, in the same change:** quote the old rule and the replacement in the commit and update `docs/DESIGN_RULES.md` (and `task.md` execution rules / README where cited). A relaxation without the matching contract text is a bug, not a feature.
- **Floors that no X task relaxes:**
  - Capabilities are enforced at the execution boundary, not in prompts (DESIGN_RULES:489). A prompt change alone never widens or narrows what runs.
  - Terminal output, repository text, model output, MCP results and delegated-run text are untrusted; they cannot create, widen or match a rule, or issue application commands.
  - Delete, rename, `SelectedCommit`, Shell-dialect commands, out-of-project paths, protected/credential/excluded paths and work while local effects are unknown always need a fresh explicit review.
  - Receipts stay truthful: every automatic decision (rule match, retry, steering, delegation, profile bytes) appears as its own receipt line with its cause. Missing evidence stays unknown.
  - Navigation, imports, suggestions and selection still never send, run or grant on their own.
- **Default posture:** each relaxation ships behind an explicit, visible state. Where a default changes behavior (XC04, XC06), the off switch is one command and the state is shown in the header chip or receipt.
- **Storage:** new persistence follows the current Schema20 admission contract unless XC08 has shipped, in which case additive data uses XC08's minor/critical rules.
- **Verification:** focused behavior tests plus an actual-binary PTY journey at 120×36 and 60×18; provider-facing behavior uses the existing loopback/controlled-peer fixtures. No paid/live provider call without separate authorization.

## Recommended order

1. **XC04** (retry) — smallest, no storage, removes the most common dead end.
   **XC09** (token/cost meter) — independent and equally small; may run alongside XC04. It reads existing usage records and also gives VA08 its measurement surface.
2. **XC08** (forward-compatible storage) — makes every later persisted feature (XC01, XC03, XC06, XC07) cheaper and non-breaking for the next upgrade.
3. **XC01** (approval rules) — largest daily effect; easier after XC08.
4. **XC03** (steering and queued Send).
5. **XC02** (read-only delegation) — turns the party into a real capability.
6. **XC05** (MCP client) — needs XC01 to stay usable.
7. **XC06** (form behavior profiles) — needs the F-track paired benchmark to claim quality.
8. **XC07** (similar-request moves, larger library) — after or merged with VA09.

---

## Approval

### XC01 — Persistent, scoped approval rules

- [ ] Let the user turn a reviewed command or write into a project rule so the same kind of call is allowed (or denied) without a prompt.
- **Priority:** 1. **Needs:** — (XC08 recommended first). **Builds on:** task-scoped `/auto-approve` (`App::auto_approve_command` in [src/app/motion.rs:35](../src/app/motion.rs#L35), `planning.auto_approve` in [src/app/planning.rs:177](../src/app/planning.rs#L177)), `execution::State::auto_approve` ([src/app/execution.rs:620](../src/app/execution.rs#L620)), `drive_auto_approval` ([src/app/cooperation.rs:734](../src/app/cooperation.rs#L734)), runner `CommandSpec`.
- **Today:** two choices only — allow once, or allow everything for the lead in this task until `/auto-approve-off`. The in-memory flag does not survive restart, cannot be narrowed and cannot deny. The native tool prompt hard-codes "Each call gets separate user approval, never a batch grant" ([src/providers/direct.rs:629](../src/providers/direct.rs#L629)).
- **Contract to change:**
  - DESIGN_RULES:439 "Broad/persistent options require explicit scope disclosure; hide them if their duration or boundary cannot be represented accurately" — **kept**, and becomes the rule editor's requirement.
  - DESIGN_RULES:441 "Never remember an allow merely because a form or model was chosen" — **kept**; add: "A remembered allow exists only as an explicit user-created rule with a displayed boundary."
  - Native prompt (direct.rs:629): "Each call gets separate user approval, never a batch grant" → "Each call is reviewed by the user or matched by a user-created rule shown in the receipt."
  - Permissions section (after DESIGN_RULES:491): add the rule model below.
- **Rule model:**
  - `Rule { id, project, effect: Allow | Deny, kind, created_from: request/run id, created_at, expires: Never | Until(date) | ThisLaunch }`.
  - `kind = Command { program: resolved executable identity (path + file identity the runner already freezes), argv_prefix: Vec<String> (token-exact), cwd: project-relative prefix }` — **Direct spec only**; Shell, `.cmd`/`.bat` wrappers and `SelectedCommit` never match.
  - `kind = Write { ops: create | edit (never delete/rename), paths: project-relative glob list }` — protected/excluded/credential paths are refused before matching.
  - Matching order: any Deny wins → most specific Allow → normal review. A rule never matches a call from another project, and never matches while `planning_any_unknown_effects()` is true.
- **UX:**
  - Command review gains `r` "Make a rule…": opens an editor prefilled with the **narrowest** rule (full argv, exact cwd). The user may shorten the argv prefix token by token, widen cwd, pick an expiry; the editor shows a plain sentence ("Allow `cargo test …` with any further arguments in `./` for this project until revoked") and the executable path.
  - After the same argv prefix has been approved twice in a task, the review shows a one-line hint to create a rule (no auto-creation).
  - `/rules`: list, inspect, revoke (immediate, before the next call), export/import as reviewed TOML (import is untrusted config; rules arrive disabled until each is enabled).
  - Receipt and transcript: "Allowed by rule #3 · cargo test (prefix) · /rules".
  - `/auto-approve` remains as the broad, task-scoped, non-persistent option.
- **Storage:** new `approval_rules` table keyed by project identity; quota-accounted; included in `ev snapshot`.
- **Accept:**
  - Prefix boundaries: `cargo test` matches `cargo test -p x` but not `cargo testx` or `cargo build`; a different resolved `cargo` binary does not match.
  - Shell spec, delete/rename, protected path and out-of-project cwd never match even with a matching rule.
  - Deny beats Allow; revoke during a run stops the next match; `ThisLaunch` disappears on restart, `Never` survives it.
  - Model output that contains rule-like text cannot create or alter rules.
  - Two-partner journey: a project rule applies to each partner's own calls without moving approval focus between owners.
  - PTY journey: create a rule from a review, rerun, see "Allowed by rule", revoke, see the review again.
- **Watch out:** `cargo test`/`npm test` run arbitrary project code (build scripts, test bodies). The rule sentence must say "runs project code" for build/test programs; this is disclosure, not a sandbox.

---

## Parallel work

### XC02 — Read-only delegated sub-runs

- [ ] Let the lead's model delegate bounded read-only investigations to bench partners that run concurrently and return a summary.
- **Priority:** 2. **Needs:** —. **Builds on:** cooperation capacity (up to three partners per task, one writer: `work_capacity_reason` and `write_owner` in [src/app/cooperation.rs:288-339](../src/app/cooperation.rs#L288-L339)), native read tools, receipts.
- **Today:** several partners can work at once, but only when the user sends to each one. The model cannot split work, so large investigations serialize inside one context window.
- **Contract to change:**
  - `task.md` execution rule "No parallel executor, parser, receipt authority or approval queue" → "No parallel **effect** executor, parser, receipt authority or approval queue. Read-only delegated runs reuse the existing provider worker and receipt store."
  - DESIGN_RULES "Role, agent, model, and control are independent" gains a delegation bullet (below).
- **Design:**
  - New native tool `eevee_delegate { role: "explore" | "review", instruction, paths: [project-relative] }`, offered to the lead only, not to delegated runs (no recursion).
  - A delegated run takes a free party member (or an unnamed helper when none is free) and counts toward the existing three-partner capacity; when capacity is full the tool returns a refusal result, never waits silently.
  - It has **read tools only** (list/read/structural search and XC05 tools explicitly marked read by the user), enforced by passing `writing = false` to `definitions` ([src/providers/writes/tools.rs:268](../src/providers/writes/tools.rs#L268)) and refusing effect calls at the executor, not by prompt.
  - Own frozen packet, own receipt linked to `parent_run_id`, same route/model as the lead unless the user pins another; bounded rounds (e.g. 64), bytes and wall time; result returned to the lead as a tool result of at most 16 KiB.
  - Up to three delegations in one model round run concurrently; results return in call order.
  - The delegation call itself is reviewed (one review per round listing every instruction), or allowed by an XC01 rule of kind `Delegate`.
- **UX:** work rows show "Vaporeon · exploring for Flareon · round 5"; `/receipts` nests child receipts; cancelling the lead cancels its children; a child's failure is a tool result, not a lead failure.
- **Out of scope:** concurrent writers. A later task may add write delegation into a separate Git worktree; it is not part of XC02.
- **Accept:** loopback fixture issuing three delegations in one round → three concurrent child runs, ordered results, combined usage shown; a child calling `eevee_write_file` is refused at the executor; capacity-full refusal; parent cancel → children cancelled with no effects; receipts linked both ways; PTY journey showing work rows.
- **Watch out:** cost multiplies; the review must show the planned child count and per-child bounds. Partner flavor stays out of child packets (XC06 profiles excepted).

---

## Run control

### XC03 — Visible mid-run steering and queued follow-up Send

- [ ] Let the user add an instruction to a running request, or queue the next request, without cancelling.
- **Priority:** 1. **Needs:** —. **Builds on:** native round loop ([src/providers/direct/coding.rs:119](../src/providers/direct/coding.rs#L119)), provider `Request` channel ([src/providers.rs:756](../src/providers.rs#L756)), journal items, WebSocket chain rules.
- **Today:** while the lead works the only way to change direction is Cancel and resend, which drops the run's context and repeats discovery.
- **Contract to change:** DESIGN_RULES:116 "No … hidden steering, queued follow-up, reconnect/retry …" → "No **hidden** steering. Explicit steering is a user message entered at the next tool-result boundary and recorded as its own receipt item. A queued follow-up is an explicit Send held until the previous request settles." (Retry wording changes in XC04.)
- **Design:**
  - `Request::Steer { generation, id, text }`. The worker drains it only at a round boundary — after tool results are acknowledged and before the next inference — and appends it as a `user` item. On WebSocket it is sent with the completed response ID like a tool result; amend the "Send only actual acknowledged tool-result items" sentence accordingly.
  - If the run ends before a boundary is reached, the steer is **not** sent; it returns to the composer with a notice offering "Send as new request".
  - Composer while the lead is busy: Enter opens a three-choice line — **Steer this run** / **Queue as next Send** / **Keep as draft** (neutral default: Keep). Exact draft/caret preserved for every choice.
  - Queued Send: one slot per partner, shown as a chip with cancel. It dispatches only when the previous outcome is Completed; Failed/Cancelled/Unknown returns it to the composer with the reason.
  - External routes: offered only where the adapter protocol supports user input during a turn; otherwise the Steer choice is hidden and only Queue/Keep remain.
- **Accept:** loopback fixture with a multi-round run: steering text reaches round N+1 exactly once and appears in the receipt as "steer #1 at round N"; steering after the final round returns to the composer; queued Send after Completed dispatches with fresh preparation and review, after Failed/Unknown does not; Esc on the choice line keeps the draft; PTY journeys at both sizes.
- **Watch out:** steering is user-authored content and still passes privacy/secret checks; it must not bypass the per-Send preparation for attachments (attachments are not allowed in a steer, only text).

### XC04 — Bounded automatic retry for provably unprocessed failures

- [ ] Retry an inference round automatically when the failure proves the provider did not process it.
- **Priority:** 1. **Needs:** —. **Builds on:** `http_client` with `retry::never()` ([src/providers/direct.rs:1035](../src/providers/direct.rs#L1035)), `http_status` ([src/providers/direct.rs:1042-1050](../src/providers/direct.rs#L1042-L1050)), connection diagnostics, WebSocket session.
- **Today:** a single 429 or a dropped connection before any response ends the whole Run, often after many approved tool rounds; the user must resend and repeat work.
- **Contract to change:**
  - DESIGN_RULES:116 "no … reconnect/retry or stale-ID replay" → "no reconnect or replay after any response byte for that round; bounded retry only for the eligible failures below."
  - DESIGN_RULES:454 keeps "never replays a mutating request automatically" (tool effects are never repeated; only the inference request for the current round is resent).
  - README "There is no automatic reconnect, retry or replay" and the "no automatic retry was attempted" messages are updated.
- **Eligible (all must hold):**
  - No response body/SSE/WS event for this round was received.
  - Failure is a connect/DNS/TLS error before the request was written, **or** HTTP 429, 503, or Anthropic 529, **or** a WebSocket handshake failure before `response.create` was sent.
  - Not cancelled, within the run deadline, and `Retry-After` (if present) ≤ 60 s.
  - Not eligible: 5xx other than 503/529, timeouts after the request was sent, mid-stream disconnects (stay outcome unknown), 4xx other than 429.
- **Policy:** at most 3 extra attempts per round, backoff 1 s / 2 s / 4 s with jitter or `Retry-After`; the same frozen body; the same transport (no HTTP↔WS fallback). Status row shows "Rate limited · retrying in 4 s (2/3) · Esc stops"; Esc during backoff ends the run as Cancelled. Launch option `--retry off|safe` and a saved preference; default `safe`.
- **Receipt:** one logical dispatch with an attempts list (time, status, wait); billing/usage notes that rejected attempts usually are not billed but this is provider-reported, not proven.
- **Accept:** loopback server returning 429 twice then 200 → one logical run, three attempts in the receipt; 500 → no retry; 503 after 10 SSE bytes → unknown, no retry; `Retry-After: 120` → no retry, message names the wait; Esc during backoff → Cancelled; `--retry off` restores today's behavior; tool effects from earlier rounds are never re-executed.
- **Watch out:** idempotency is the provider's claim; keep the eligible list narrow and per-protocol. Never retry model listing pages silently beyond the same rules.

---

## Extensibility

### XC05 — MCP client for native routes

- [ ] Let the user connect stdio MCP servers and expose selected tools to native requests under review.
- **Priority:** 2. **Needs:** XC01 (otherwise each MCP call prompts). **Builds on:** reviewed tool catalogue (`/tools`, [src/providers/catalog.rs](../src/providers/catalog.rs), [src/store/tool_catalog.rs](../src/store/tool_catalog.rs)), runner process-tree ownership, receipts.
- **Today:** native routes offer a fixed set of host tools; the prompt forbids MCP ([src/providers/direct.rs:309](../src/providers/direct.rs#L309), [:629-631](../src/providers/direct.rs#L629-L631)) and the catalogue states "Catalog routes are closed Eevee-owned executors, not arbitrary MCP/SDK forwarding" ([src/providers/catalog.rs:2](../src/providers/catalog.rs#L2)).
- **Contract to change:**
  - Catalogue doc comment and the native prompt strings: allow "user-registered MCP tools, each labelled unmanaged".
  - DESIGN_RULES Permissions: add "An MCP server is an **unmanaged** capability: Eevee enforces launch, selection, review and output bounds, not the server's effects."
  - DESIGN_RULES:381 (basket describes what Eevee sends): MCP tool schemas and results count as sent bytes.
- **Design:**
  - Registration: `/mcp add` opens a review of exact argv, cwd, env **names** (values masked, from the existing credential store), and the unmanaged label. Stored per project (or personal). Stdio only in this task; Streamable HTTP is a follow-up.
  - Connection: `/mcp connect <server>` launches through the runner with direct argv and owned process tree, performs `initialize` + `tools/list` once, and shows the tool list; the user selects which tools join the request (reusing the `/tools` selection and VA13's "not sent" disclosure).
  - Calls: each `tools/call` gets a review (server, tool, JSON arguments, unmanaged label) unless an XC01 rule of kind `Mcp { server, tool }` matches. Server `readOnlyHint`/annotations are displayed but never used to auto-approve.
  - Results: untrusted, bounded (64 KiB per call), rendered with control sequences stripped, stored in the receipt.
  - Refused protocol features: sampling (server asking the model), elicitation, resources/prompts in this task; roots expose only the project root.
- **Accept:** fixture Python stdio server with `echo` and `write_note` tools: registration review, selection, reviewed call, rule-allowed call, crash mid-call → tool error result and run continues, cancel kills the server tree, unselected tools never appear in the request body, Windows ConPTY launch.
- **Watch out:** a server can do anything its process can; the UI must never call it sandboxed. Server-supplied tool descriptions are untrusted text and are shown, not obeyed.

### XC06 — Form behavior profiles in provider packets

- [ ] Make each form change how the model works, through a visible, editable instruction profile.
- **Priority:** 3. **Needs:** F-track paired quality benchmark before any quality claim. **Builds on:** forms table (DESIGN_RULES:166-176), EV18 classifier, workflow recipes ([src/workspace/workflows.rs:1181](../src/workspace/workflows.rs#L1181)).
- **Today:** forms change appearance and, through the classifier, the operation; the packet never contains form information (DESIGN_RULES:245, `task.md` "Keep partner flavor out of provider packets"). Evolution is therefore cosmetic for the model.
- **Contract to change:** DESIGN_RULES:245 → "Names, nicknames, species lore and teaching provenance never enter provider packets. A form's **behavior profile** — neutral, user-visible instruction text — enters the packet when enabled and is shown as its own basket item and receipt bytes." `task.md` rule wording updated the same way; handoffs include the profile only when the user selects it.
- **Design:**
  - `FormProfile { form, instructions (≤ 2 KiB, neutral language, no species names), preferred_checks: Vec<argv> (suggestions only), schema_version }`.
  - Built-in defaults derived from the forms table "Specialty"/"Exit evidence" columns, e.g. Jolteon: "Prefer running the smallest relevant command and report exit status before claiming success"; Espeon: "Propose an ordered plan with tradeoffs before editing".
  - `/form-profile` shows, edits (external editor), resets or disables the current form's profile; `/form-profile off` disables all profiles for the project. Profiles can be carried in an Evolution Stone recipe as reviewed text (recipe import stays untrusted and disabled until reviewed).
  - Basket chip "Profile: Jolteon · 312 B" with remove-for-this-Send.
- **Accept:** receipt bytes match the displayed profile exactly; species/nickname strings never appear in any packet (fixture scan); disable removes it from the next Send; recipe import arrives disabled; paired benchmark results reported separately (no claim without them).
- **Watch out:** profiles shift model behavior and therefore evolution evidence; keep the classifier input free of profile text so evolution stays based on user requests.

### XC07 — Similar-request moves and a larger move library

- [ ] Group near-identical requests into one learned move and allow more than four stored moves.
- **Priority:** 3. **Needs:** VA09 (slots) or absorbs it. **Builds on:** GL01 grouping (`loose_key`, [src/workspace/game.rs:82](../src/workspace/game.rs#L82)), habits (DESIGN_RULES:255), move inventory (DESIGN_RULES:233).
- **Today:** grouping matches only case/whitespace/trailing-punctuation variants; habits require byte-equal instructions; at most four active moves per partner/form.
- **Contract to change:**
  - DESIGN_RULES:255 "interior Unicode codepoints, case, spacing and literal source/path bytes are never rewritten or merged" → applies to **saved** text only; **proposals** may group by the similarity rule below, and saving shows the exact differences.
  - DESIGN_RULES:233 "at most four active moves" → "four **quick** moves per partner/form plus a library of up to 32"; the quick four keep today's UI.
  - DESIGN_RULES:245 "No progression score…" — **kept**.
- **Similarity rule (deterministic):** tokenize; replace paths, quoted text, numbers and identifiers with typed slots; strip a closed list of Korean particles/endings and English articles; group when slot-normalized token sequences are equal **and** classifier form/operation are equal **and** the leading verb token is equal. No embeddings or model calls.
- **Accept:** "fix the bug in src/a.rs" / "Fix bug in src/b.rs" group into one parameterized move; "delete src/a.rs" never groups with "fix src/a.rs"; Korean fixtures ("a.rs 버그 고쳐줘" / "b.rs의 버그 고쳐줘"); saving shows the variant table; library over 32 routes through the existing forget/replace review; old TMs import unchanged.
- **Watch out:** false grouping is the risk; every proposal stays dismissible and nothing is learned without the existing teach review.

---

## Visibility

### XC09 — Live token and estimated cost meter for this launch, task and request

- [ ] Show how many tokens this `ev` launch, the current task and the running request have used, and a labelled cost estimate, in the always-visible chrome.
- **Priority:** 1. **Needs:** —. **Builds on:** provider-reported usage (`workspace::usage::{Counters, Semantics, Report, Record}`, [src/workspace/usage.rs](../src/workspace/usage.rs)), live per-request snapshots (`planning::State::usage_report`, [src/app/planning/usage.rs:52](../src/app/planning/usage.rs#L52)), durable `records::Operation::Usage` rows ([src/store/records.rs:536](../src/store/records.rs#L536)), status row ([src/ui/chrome.rs](../src/ui/chrome.rs)).
- **Today:** usage counters are already collected per request (input/output/cache read/cache write/reasoning, with subset semantics) and saved, but nothing sums them or shows them while you work. Cost is never computed: the usage module states "These are neither local estimates nor prices" ([src/workspace/usage.rs:1](../src/workspace/usage.rs#L1)), and the party board prints a fixed "Provider usage/cost: unknown (not reported)" even when the provider did report usage ([src/app/cooperation_ui.rs:64](../src/app/cooperation_ui.rs#L64)).
- **Contract to change:**
  - DESIGN_RULES:300 "Unknown totals, tokenizer, remote completion and billing remain unknown" → "Unknown totals, tokenizer, remote completion and **billing** remain unknown. A local cost **estimate** computed from reported counters and a dated price table may be shown; it is always labelled `est.`, names its price source and date, and is never called billing."
  - `usage.rs` module doc: counters stay raw provider data; estimates live in a separate `workspace::pricing` module and are never written back into usage records.
  - DESIGN_RULES:219 "Visible cost … never hides per-partner cost" — **kept**; the meter adds a per-partner breakdown, never merges it away.
- **Scopes ("session" defined):**
  - **Launch** — every request dispatched since this `ev` process started, across tasks and partners. This is "the current session". In memory only; resets on restart. Provider-side sessions (`/session-new`) are a different concept and are not used for this.
  - **Task** — every saved usage record of the current task, including earlier launches (read from the store).
  - **Request** — the running or last request, updated live from partial snapshots.
- **Aggregation rules (truthful sums):**
  - Sum `Measurement::attributable` only; cumulative-scope reports without a known baseline count as **unknown**, never as their raw cumulative value.
  - Use `Semantics` to avoid double counting: cached-read and reasoning tokens are shown as parts of input/output when the provider says they are subsets, as separate amounts when it says they are not, and the request's **cost** is unknown when the relation is unknown (tokens are still shown, marked `?`).
  - A request with no reported usage (e.g. ACP routes without accounting) adds to an "unreported" count instead of adding zero. Totals with any unknown part are shown with a lower-bound mark: `≥ 48.2k tok`.
- **Pricing:**
  - `Price { provider, model (exact or prefix), input, cached_input, cache_write, output, reasoning (defaults to output), unit: per 1M tokens, currency: USD, source: BuiltIn { as_of } | User }`.
  - The built-in table is compiled in with an `as_of` date and covers the common native API models. It is **never fetched over the network** (startup stays offline). `ev --version` and `/usage` show its date; after 90 days the estimate is labelled stale.
  - `/prices` opens the table as reviewed TOML in the external editor; user rows override built-in ones; stored as a preference. The meter recomputes immediately.
  - Route kinds: **API-key routes** → token × price; **account/subscription routes** (ChatGPT account, Codex/Claude CLI logins) → tokens only, cost shown as `plan`; **loopback/local endpoints** (Ollama, LM Studio, vLLM on loopback) → `local`, no cost; **unknown model** → tokens only, `no price`.
  - Long-context or tiered prices: one optional `above_tokens` tier per row; anything more complex is a user row or `no price`.
- **UX:**
  - Status row, right side (before the mode chip): `Launch 48.2k tok · est. $0.31`. While a request runs: `Req 6.1k ↑ · Launch 48.2k · est. $0.31`, refreshed on the existing activity tick without delaying provider bytes.
  - Compact (<90 columns): `48k · $0.31`; below 70 columns the chip moves into `/usage` only, never pushing route/model out of the header.
  - Unknown parts: `≥48.2k tok · est. $0.31+?`; `plan`/`local` replace the dollar amount.
  - `/usage` panel: rows Request / Task / Launch × columns input · cached · output · reasoning · est. cost; per-partner and per-model breakdown; unreported-request count; price source/date per row; `d` shows the exact counters of the selected request.
  - Fix the party board: show the owner's actual reported counters and estimate instead of the fixed "unknown" line, keeping "not billing".
  - `/usage-meter off|on` hides/shows the chip (saved preference). Optional `/usage-budget <USD>`: a one-time notice when the launch estimate crosses it; it never blocks, pauses or cancels work.
- **Storage:** none for Launch/Request (memory) or Task (existing records). New preference fields: user price rows, meter visibility, budget. Before XC08 these follow the current strict preference contract (older readers refuse a non-default value; record that in release notes).
- **Accept:**
  - Unit fixtures: Responses report with cached tokens inside input → cached not added twice; Anthropic-style cache write separate → priced separately; reasoning-in-output true/false/unknown → three different, correct displays; cumulative report without baseline → unknown, not counted.
  - Live: loopback provider streaming three partial usage snapshots → the request chip updates on each and the final total equals the saved record.
  - Launch total spans two tasks and two partners, per-partner breakdown sums correctly, and resets after restart; Task total survives restart.
  - Unreported ACP-style request → `≥` mark and "1 request unreported"; account route → `plan`; loopback endpoint → `local`; unknown model → `no price`.
  - Editing `/prices` recomputes without restart; stale built-in date shows the stale label; no network request at startup (existing offline startup test).
  - Party board shows the real counters; PTY captures at 120×36 and 60×18 while idle, during a live request and with unknown parts.
- **Watch out:** a stale or wrong price is worse than none. Always show the source and date, never round a missing price to zero, and keep the word "estimate" in every surface. Bytes and local token estimates in the context basket remain a separate, pre-send measure and must not be mixed into the reported totals.

---

## Storage

### XC08 — Forward-compatible storage with critical-field marking

- [ ] Let an older `ev` read and write data from a newer minor version when the new data is safe to ignore, and back up automatically before migrations.
- **Priority:** 1. **Needs:** —. **Builds on:** schema admission ([src/store/admission.rs:4](../src/store/admission.rs#L4)), `ev snapshot`, 93 `#[serde(deny_unknown_fields)]` structs.
- **Today:** admission requires exactly Schema20; strict structs refuse any unknown field. A single optional field (e.g. `progress.eager_off`, [src/workspace/progress.rs:37](../src/workspace/progress.rs#L37)) makes the previous release refuse the task, so every upgrade/rollback needs a manual whole-directory backup.
- **Contract to change:**
  - README "`ev` refuses data it cannot read safely rather than converting it" → "… refuses data marked critical that it does not understand; ignorable additions are preserved."
  - `task.md` "older-writer-refusal contract" → refusal on **major** bump or unknown **critical** field only.
- **Design:**
  - Version = `user_version` (major) + `eevee_meta(minor, critical_fields JSON)` table.
  - Each persisted JSON struct gets `#[serde(flatten)] extra: BTreeMap<String, Value>` instead of `deny_unknown_fields`; writers round-trip `extra` byte-for-byte (canonical JSON), so an older writer never drops newer fields.
  - A newer writer lists any field whose absence would change behavior (privacy, cost, effects — `eager_off` is the example) in `critical_fields`; an older reader that sees an unknown critical field refuses as today. Everything else is ignorable.
  - Before any major migration, create an automatic `ev snapshot` into `data/backups/` (keep the last three, quota-accounted, same privacy disclosure as manual snapshots).
  - Conversion of the existing 93 structs is mechanical but must keep strict validation of **known** fields (types, bounds, enums).
- **Accept:** matrix test with a simulated "previous" struct set: non-critical extra → read, edit, write, extra preserved byte-equal; critical unknown → refuse with the field name; major bump → refuse; automatic snapshot created and restorable with `ev snapshot restore`; real Schema18/19 fixtures still upgrade.
- **Watch out:** the benefit starts only for releases **after** XC08 ships; readers up to the current version still refuse. Record that in the release notes.
