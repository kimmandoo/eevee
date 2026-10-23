<div align="center">

<img src="https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/133.png" width="192" height="192" alt="Original Eevee pixel sprite with a transparent background">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.png">
    <img src="docs/assets/wordmark-light.png" width="332" height="68" alt="eevee">
  </picture>
</h1>

**A local-first terminal workbench for LLM-assisted development.**

One persistent task. Your party. Nine task-specific forms. Your work stays local until you choose otherwise.

<sub>BUILT WITH RUST &nbsp; / &nbsp; KEYBOARD-FIRST &nbsp; / &nbsp; APACHE-2.0</sub>

[Quick start](#quick-start) · [Commands](#commands) · [Agents](#connect-an-agent) · [Install](#installation) · [Development](#development)

</div>

---

Eevee brings durable tasks, repository context, reviewed commands, and optional coding agents into a native Rust TUI. The executable is **`ev`**. The local workbench needs no account, API key, Node runtime, or model connection.

- **Keep the task, not just the chat.** Goals, acceptance criteria, next actions and drafts survive restarts.
- **Know what changed.** Repository baselines and raw diffs; pre-existing edits stay distinct from later changes.
- **Choose the context.** Attach specific files or line ranges and read the exact outgoing payload, with a local token receipt, before any model request.
- **Review before running.** Local commands, provider requests, edits and handoffs each need explicit, one-time approval.
- **Switch the workflow, not your identity.** Nine forms organize task roles without locking you to a provider.

## Quick start

**macOS / Linux / WSL**

```sh
curl -fsSL https://github.com/kimmandoo/eevee/releases/latest/download/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
ev
```

**Windows (PowerShell)** — works under the default execution policy; no admin needed:

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/latest/download/install.ps1)))
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
ev
```

Keeping `ev` on PATH, pinned versions and offline installs: see [Installation](#installation).

**Choose the project by folder.** Run `ev` inside the directory whose files you want to work on. Tasks, attachments, exclusions, file tools and provider cwd use that canonical folder—even inside a larger Git repository. Plain folders support browsing and file work without requiring `git init`; Git status/diff remain optional evidence. Task selection stays within that project. Exit and launch from another folder to change projects.

Application data is never project source context, even with an in-project `--data-dir`. Keep the data directory separate from the project itself: choosing the project root, or its ancestor, as storage intentionally makes that whole subtree unavailable to source tools. Previously attached storage files are retained as history but refused on send.

```sh
ev update --check   # read-only release check
ev update           # upgrade in place
ev uninstall        # remove the binary; keep user data
```
Install and `ev update` share truthful animated phases, respecting UTF-8 and `NO_COLOR`. Use POSIX installer `--no-animation`, PowerShell installer `-NoAnimation`, or `ev update --no-animation` for static phases. Redirected output, CI and dumb terminals remain static and escape-free; `ev update --check` stays read-only.


Use a terminal of at least **60 × 18** (120 × 36 recommended).

1. Type in the bottom composer. **Enter** saves and sends the request; **Ctrl+S** saves locally only. No separate Goal setup or routine approval popup.
2. `/provider` chooses an explicit route; **Codex native / scoped coding** and **Codex CLI** retain distinct connection and session policies. `/connect openai` opens masked API setup directly; **Save & choose model** selects that route and opens metadata selection, never sends the draft. `/model` follows **model → supported reasoning level → editor**; `/effort` opens the level directly.
3. Approve actual effects when needed: commands, file changes, write-enabled sessions, project-supplied executables, external credential helpers and unmanaged agent connections. `/session-new` or `/session-resume` opens advanced exact-payload review.
4. Read responses inline and type the next request. Nonsecret provider/model/effort choices survive restart. Explicitly saved private connections can be reused; native chat history, provider sessions and grants are not restored.
5. `/command-edit` authors an explicit local command in Jolteon; `/execute` reviews it for one-shot execution. `/run` inspects evidence, without executing anything.

**Provider:** type to filter the ten explicit routes. Ready **Enter** selects the route and opens models; unready rows open setup. **Right** exposes setup/diagnostics without switching. Required login or existing-context switch reviews remain separate; their neutral Back never launches a provider. Missing connection setup preserves the exact Request and requires a deliberate later send.

**Models:** exact IDs/names rank first. From staged reasoning, **Left** returns to the same model search; **Esc** keeps both previous settings. **Ctrl+D** shows complete provenance/errors. `/model <exact-id>` skips model search; `/effort <supported-value>` sets a level directly (`/effort default` resets it unless the provider advertises that literal value). These controls never send the draft.

**Files:** type `@`, filter a filename and press **Enter** for its bounded preview; **Enter** attaches locally and returns to the same draft after storage succeeds. **r** selects a line range instead; **Esc** cancels. Saved files have a `[+]` marker. In `/files`, **Tab** switches files/attachments and **Del** opens removal review; the `@` chooser also offers **Del** for an attached file.
File search accepts case-insensitive abbreviations, scoped paths and spaces, for example `@SRC/APP/CMP NTS`. Saved paths and line ranges stay visible beside the draft. Attachment records are independent of draft text: deleting an `@` token does not silently unpin a saved capture.


Use `/` (Ctrl+K) to search actions by name or description. The centered chooser supports multiple search words in any order (`desk review`, `attach`), preserves the draft and shows unavailable reasons. Arrows/Tab/Enter/Esc navigate; repeated visits do not add extra Back steps. On confirmation surfaces, **y** confirms and **n** cancels; offered provider permission prompts also accept **a** for **allow once**, never a permanent grant. These letters remain ordinary text inside editors; paste and held-key repeats never approve.

```sh
ev --theme light            # cream palette
ev --ascii --color mono     # plain-terminal fallback
ev --data-dir /abs/path     # isolated data directory
ev --no-animation           # static activity indicator; streaming stays enabled
ev --help
```

## Commands

| Command | Action |
| --- | --- |
| `/connect` / `/connect <route>` / `/provider` | Connections selector / direct named setup / execution-provider selector; never send a task |
| `/model` / `/effort` / `/models-refresh` | Local model and reasoning pickers / reviewed catalogue refresh |
| `/new` / `/resume` | Send the saved Request as an ordinary new turn / owned same-task continuation, consistently from every view |
| `/session-new` / `/session-resume` | Advanced exact-payload review for a fresh session / owned continuation |
| `/chat-new` | Reset this task/partner/native-route/role's process-local chat; keep draft, displayed output and durable evidence |
| `/role` / `/forms` | Plan, Implement or Review / choose an evolution |
| `/nickname` | Set a local partner nickname (up to 12 graphemes); Enter saves, empty clears, Esc keeps the old name. Names never become automatic provider or handoff context. |
| `/party` / `/lead <name>` | Inspect/add local partners or choose the lead; fronting a partner never sends or cancels its work. |
| `/party-board` | Inspect working/waiting partners and each request's own model/receipt; choose an owner to front. Compact home shows a count instead of three full rows. |
| `/formation [path]` / `/formation-save [path]` | Preview a saved formation or review saving the current party as create-only TOML; map roles to existing partners before explicit application. |
| `/approvals` | Focus the oldest pending command, permission or question. Escape defers without answering; each answer remains bound to its owner/request. |
| `/cancel` / `/cancel-all` | Cancel only the lead's work / review every owned request before cancelling all and waiting for settlement. |
| `/journal-discard` | After a definite terminal outcome, neutrally review discarding only provably unsaved failed journal write attempts. Committed observations remain; a coverage-gap notice remains visible. Unknown outcomes and ambiguous commits refuse; nothing is replayed. |
| `/release-partner [name]` | Review removing an idle non-last partner's personal/draft/intended work state; retain tasks and evidence with historical name snapshots. |
| `/suggestion-dismiss` / Forms picker `d` | Hide the current task's exact evidence-based form suggestion, including after restart; a new triggering run can recommend again. Does not select a form or start work. |
| `/move` / form surface `m` | Open this partner/form's four-slot move menu; Enter stages the ordinary review, `d` opens optional details. Escape restores the original draft/caret. |
| `/teach-move` / `/move-reminder` | Review teaching the current approved run, selected pins, last submitted user instruction or task facts / revisit forgotten and postponed moves. |
| `/move-import [path]` / `/move-export [path]` | Review a portable TM's local recipient/name/scope/slot or export the selected move as create-only TOML. Move-menu `i`/`e` opens the same paths. |
| `/move-retry` / `/move-discard` | Retry the exact failed move-store operation or review discarding its unsaved intent; neither path runs the move. |
| `/carry-over` / Companion `c` | Reopen the last form switch's retained-state counts and capability difference. The one-line strip clears on the next key/paste without consuming it; Companion explains C/P/H/F. |
| `/partner-summary` / `/routes` | Inspect factual partner records / accepted form routes (not execution evidence). |
| `/dex` / `/dex-resolve` | Inspect project failures / review an eligible historical resolution. |
| `/habits` / `/habit-approve` / `/habit-dismiss` | Inspect repeated authored instructions, explicitly approve project memory, or durably dismiss. |
| `/pairings` / `/save-pairing` / `/remove-pairing` | Inspect, save or remove this project/partner/form's route/model/effort choice. |
| `/flavor-off` / `/flavor-on` | Persistently hide/show factual activity without changing safety feedback. |
| `/motion-off` / `/motion-on` / `/reduced-motion` | Persist static motion, allow motion, or toggle the saved preference without changing requests or permission. |
| `/records-retry` / `/records-discard` | Retry an exact failed write / review discarding unsaved intent; neither replays work. |
| `/goal` / `/next` / `/criteria` / `/criteria-edit` | Edit task facts, inspect criteria, or edit criteria directly |
| `/tasks` / `/task-new` | Browse saved tasks / create one |
| `/reset` | Review a fresh local task; save the old task and drafts for `/tasks`, without erasing files or history |
| `/init` | Prepare an editable Plan request for a repository-specific `AGENTS.md` proposal; no automatic send or file write |
| `/files` / `@` / `/search` / `/range` | Source context / inline attachment / direct filename search / selected exact-range review |
| `/command-edit` / `/execute` | Author explicit argv or a shell object / review the exact prepared command for one-shot execution |
| `/run` / `/runs` | Inspect local execution evidence / retained run history; neither command executes work |
| `/locations` / `/pin-location` (in `/run`) | Parse only the current retained output window/page; review a bounded source capture with the selected run as provenance. Location Enter opens the same Pin range review. |
| `/rerun` | Stage an observed stale check with exact argv/cwd for a new Jolteon approval; cancellation/denial starts nothing and the old run stays unchanged. |
| `/diff` / `/history` / `/decision` | Worktree diff / provider journal / save a proposal as next action |
| `/response` | Front the current provider response/request drawer; `/provider` always remains the connection selector |
| `/handoff` / `/memory` / `/workflows` | Portable handoff / approved notes / diagnosis, refactor, review and recipe desk |
| `/create-memory` / `/export-json` / `/export-markdown` / `/import-handoff` / `/edit-hypothesis` | Open the existing editor/path/review directly, without a preliminary workspace visit |
| `/workflow-diagnosis` / `/workflow-refactor` / `/workflow-review` / `/workflow-recipe` | Open the chosen specialist desk directly |
| `/specialist-fix` (Diagnosis/Review desk) | Stage a supported hypothesis or unresolved request-changes finding as a Flareon fix; choose own form or an idle matching partner. |
| `/decision-criteria` / `/confirm-decision-criteria` (Recipe desk) | Edit items from a deliberately saved Espeon decision, inspect duplicates, then explicitly confirm new unchecked criteria. `/relay-decision` stages references for an idle Eevee instead. |
| `/refactor-baseline` / `/fresh-baseline` (Refactor desk) | Inspect eligible retained runs with age/revision and rejection reasons, or choose a separately approved fresh baseline. |
| `/sessions` / `/paths` | Review session metadata inspection / edit the implementation path draft directly |
| `/images` / `/inspect` / `/companion` | Sprites / details / companion |
| `/quit` / Ctrl+C | Safe exit review |

Pasted text never sends a request or executes a command. Model/effort pickers and `/models-refresh` fetch metadata without a second popup for qualified connections, including on a fresh home, without task content. Risky executable/unmanaged connection effects still require review. Errors retain a retry path and stale rows. Pickers distinguish the saved choice from keyboard focus with color and text markers; catalogue provenance and errors remain available in details. After reasoning selection (or immediately for models without it), return to the original draft/caret without sending.

`/reset` defaults to Stay and waits for durable task/context saves before switching, preserving old work in `/tasks`. Finish or explicitly cancel active work first. `/init` selects Espeon / Plan, clears write paths and prepares an editable project-guide proposal; replacing a nonempty Request requires confirmation. It sends nothing and overwrites no `AGENTS.md` by itself. **Enter** sends when you are ready; other drafts remain retained.

**Missing newer Codex models?** Select **Codex native / scoped coding** or `/provider-chatgpt` for account discovery and streaming against the official backend. It accepts a deliberately selected app token, existing readable Codex session or environment token; no installed CLI is needed for inference. Explicit browser/device sign-in delegates to the official installed CLI, while **Load CLI token** imports only a user-selected readable token into masked setup. Keychain-only and expired credentials are reported, not guessed or silently refreshed. The separate CLI route retains app-server tools, controlled writes and owned sessions. Both preserve exact custom IDs and stale rows on discovery failure.

**SDK / ACP model choices.** Lists are informational: exact custom IDs still require the integration's official selector and acknowledgment. Claude SDK effort remains requested, not proven applied. v0.1.20 requires complete bridge revision 9 files: replace the complete `sidecar` source and dependency files together, then reproduce dependencies with `npm ci --prefix sidecar`. A dependency-only update does not update policy; older bridges refuse before prompts. The only SDK MCP tool is host-owned, read-only filtered project discovery; external MCP servers remain disabled.

**Home and responses.** At 90 columns or wider, the partner card stays beside output; smaller/linear layouts keep full-width output and `/companion`. Responses, plans and tool observations form one continuously scrollable transcript, with original owner/request attribution instead of a separate page per response. Only changed segments reflow. Streamed words appear without artificial typing; incomplete credential prefixes remain withheld for best-effort redaction. One header indicator represents actual preparation/connection/response activity, not estimated progress; reviews and idle screens do not animate. `--no-animation` keeps it static without slowing text.

**Everyday navigation.** `/` or Ctrl+K puts connection, model, files and tasks before advanced session actions. Search accepts localized descriptions and keywords such as `attachment`, `stop`, `evidence`, `첨부` and `중지`; duplicate connection/form aliases do not crowd the list, while exact `/connect` and form aliases still work. In source previews and payloads, Tab returns straight to Files; `/basket` opens attachments. Selecting the starred current task returns without reloading or interrupting its response. Accepting the unchanged model/effort pair returns directly; `/effort` changes reasoning separately, and a changed model still stages reasoning and any required session review.

**Motion controls.** `/motion-off` persists across restart and applies immediately, including during owned work. `/motion-on` allows suitable-terminal activity and deliberate picker motion. An explicit `--animation` or `--no-animation` overrides that saved choice for one launch; the last CLI choice wins and never rewrites the preference. Unsuitable terminals remain static. `NO_COLOR` controls color, not motion; streamed text is never artificially delayed.

**Request receipt.** Every provider preview breaks the exact payload Eevee sends into instructions, task facts, attachments, handoff and memories, with exact bytes and a labelled local token estimate (`eevee-local-v1`, CJK-aware). It runs offline. Provider-added context and unreported usage/billing stay **unknown**, not zero; reported counters retain their own response/turn/session/pipeline scope instead of becoming an invented full-context total.

**Language and reading controls.** `ev --language ko` or `EV_LANGUAGE=ko ev` selects Korean; `--language en` selects English. CLI overrides the environment; explicit `auto` resolves the first nonempty `LC_ALL`, `LC_MESSAGES`, `LANG`, with English fallback. Embedded Fluent resources support additional BCP47 locales and per-key fallback without screen edits. Provider/model IDs, raw scope, original errors and your text remain unchanged. Korean requires UTF-8 even with `--ascii`.

To add a display language, add Fluent `.ftl` resources under `locales/<BCP47-tag>/` using the existing English message IDs and named arguments. The build discovers and embeds the locale automatically; partial catalogs fall back per key to English. Keep English complete, preserve key meanings and argument names, and do not add language branches to screens. Invalid resources and inconsistent required arguments fail the build.

Wheel input inside output scrolls three lines and pauses following. PgUp/PgDn move a viewport across response boundaries; nonediting Up/Down move lines. Alt+Up/Down jump to a segment for inspection; Home/Alt+Home reaches transcript start, and nonediting End or Alt+End follows the actual latest output. Paused item/byte anchors survive streaming, later responses and resize. Ordinary editor arrows/Home/End retain draft/caret behavior; overlays/reviews keep input ownership.

Attachment chips identify exact saved captures, with inspect/remove controls, worker-observed freshness and exact serialized contribution rather than raw-text bytes. Inspection preserves drafts/carets. No live-watch promise is implied. Reported provider usage retains optional input/output/cache/reasoning and explicit scope; cumulative session/pipeline counts are not invented request totals, and unknown usage/billing remains unknown.

## Connect a provider or agent

Native API services need no provider CLI, Node or SDK sidecar. `/connect <route>` accepts masked credentials, endpoint/default-model settings and explicit Compatible protocol/keyless choices. **Save & choose model** explicitly selects the route and may fetch metadata; **Ctrl+S** saves locally only. Neither submits the retained Request. Ordinary private typing/navigation remains local; saving a connection or confirming removal requires committed idle task state.

Session connections stay in memory. Selecting private persistence writes only that route's bounded file under `~/.eevee-connections/` (`%USERPROFILE%\.eevee-connections\` on Windows): owner-private **plaintext, not encryption or an OS keychain**. POSIX no-follow/identity checks, macOS ACL checks and protected Windows owner ACLs refuse unsafe existing files/directories. Source precedence is session → saved app profile → existing environment/provider configuration. **Forget app connection** neutrally reviews only that route's app file/session; it does not revoke a remote token or touch official provider stores, tasks, evidence or other routes. Existing environment/provider sources may become available afterward. Credentials never enter Request drafts, task storage, journals or logs.

| Native service | Selection | Credential |
| --- | --- | --- |
| Codex native / ChatGPT | `/provider-chatgpt` | Masked access-token/account setup; explicit readable CLI-token import; existing `$CODEX_HOME/auth.json` or `CODEX_ACCESS_TOKEN` / `CODEX_ACCOUNT_ID` |
| OpenAI Responses | `/provider-openai` | In-app API key, or `OPENAI_API_KEY`; optional `OPENAI_BASE_URL`, `OPENAI_MODEL` |
| Anthropic Messages | `/provider-anthropic` | In-app API key, or `ANTHROPIC_API_KEY`; optional endpoint/model. No subscription OAuth reuse. |
| Google Gemini | `/provider-google` | In-app API key, or `GEMINI_API_KEY` / `GOOGLE_API_KEY`; optional endpoint/model |
| Compatible / local service | `/provider-compatible` | In-app endpoint/model/protocol and key or explicit keyless mode; environment configuration remains available |

**Automatic evolution and code generation.** Describe the task in the ordinary Eevee composer. On explicit send, the same partner locally selects the appropriate form; no extra classifier/model request is made. Choosing a specialist manually fixes that form; `/eevee` restores automatic selection. Current form plus automatic/manual mode persist per partner. Typing, browsing and incoming events never evolve or start work. Automatic transitions preserve the selected provider/model and do not restore another form's pairing or opaque session. Explicit read-only choices remain non-writing; changes still need exact one-shot consent.

Native/Codex noninspection Plan/Review expose bounded `eevee_list_files` and `eevee_read_file` without a writer lease or mutation approval. Implement additionally exposes `eevee_write_file` and `eevee_edit_file`. Discovery includes plain non-Git projects; reads return actual source, line metadata and SHA256. Targeted edits require the observed digest and one unique literal match, preserving unrelated dirty content and LF/CRLF. The model must use actual write/edit tools for requested changes, not claim that a displayed code block created a file. `/paths` optionally restricts exact write targets; `[]` clears that restriction locally, without granting directory writes.

Discovery returns `/`-separated tool paths on every host and matches literal queries against that spelling (`src/` matches that exact substring). Unicode filename bytes/spelling and the native filesystem identities used for safe access are not normalized.

Native/Codex retain eight distinct write targets up to 64 KiB each. **Deny** is neutral; **Allow once** binds exact Create/Replace, path, content and missing parents (`d` opens full details, bounded to 256 KiB/32,768 lines). Only approved parents are created, and new publication never overwrites an appeared file. Stale bytes/ancestors, links/reparse points and protected/excluded paths refuse. These native tools add no shell, hooks, MCP or automatic execution. SDK combines host-owned filtered discovery with guarded native source tools and exact Write/Edit consent; its external observational policy is not atomic confinement.

ACP routes support project filesystem callbacks and an explicitly reviewed external-policy implementation session. Client file writes use the same exact consent and executor; the agent's own tools/configuration remain outside that mediated boundary. `/paths` restricts client writes, not arbitrary external tool effects. Structured file results distinguish actual client-applied bytes from provider-reported success/failure. Terminal callbacks and managed independent Review remain unavailable.

Current-owner file proposals **automatically open** from the safe Request/Workspace, retaining the draft and caret. Press **Enter** to engage inspection, then **a/y** to allow once or **n** to deny; arriving typing, paste and repeated/modified keys cannot grant anything. **Esc** defers without answering or immediately reopening. Other sheets/editors and background owners are not interrupted; **Enter** from the owning Request or **Alt+A** still reopens a deferred review. Generic permissions/questions retain deliberate opening. File review shows Create/Modify, exact file/parent scope and actual code; no `/approvals` search or `/paths` preselection is needed for ordinary generation.

**Live file previews.** Generated files have a fixed filename/status, separate numbered code surface and structural gutter, distinct from prose in monochrome or mid-card scrolling. An emphasized writing edge follows actual received source; whole-call proposals display immediately without invented token streaming. Generating/received code is **not saved**: exact approval precedes mediated effects, and provider-reported success/failure remains distinct from verified Applied or Unknown. Cancellation never claims an unfinished proposal succeeded. Redacted previews stay outside canonical replies, decisions, journals and source evidence; paused scrolling and future drafts/carets remain owned.

**Evolution popup.** A real automatic/manual form change shows a themed, localized previous→next form receipt once, behind approvals, other sheets and live generated code. Only a deliberate form-picker commit may show a restrained 240 ms transition; automatic/named-command/formation changes and reduced-motion/text-only routes show the complete static result. A key or paste skips visible motion and reaches the original editor once. Resize or a higher-priority surface stops motion without replay; same-form selection, task restoration and restart never replay it. Evolution never approves a file change or animates the idle portrait.

Native file tools use bounded request-local continuations (at most eight HTTP rounds and sixteen calls), one durable request lease (writing only for Implement) and one authored dispatch receipt. Tool fragments never execute. Only completed public user/assistant pairs enter canonical history; private reasoning, signed tool metadata and transient tool transcripts are not persisted there. History is process-local and scoped to the task, partner, connection and role. An automatic role transition starts a disclosed fresh owned context while keeping the visible transcript; `/chat-new` remains the explicit reset for the current role. Managed Review stays independent. Account/endpoint drift refuses old-context transmission; no retry, account fallback or automatic resend is introduced. Custom endpoints require HTTPS except loopback HTTP; redirects are refused.

A canceled/disconnected native request releases local ownership only after its reader and storage settle. Known completed client writes remain on disk; partial model output and remote completion/billing remain unverified. A later explicit request is allowed only when local effects are settled. Uncertain filesystem effects and external-agent effects remain blocking even after changing the selected route.

Since Schema 10, task-independent preference rows remember nonsecret provider/model/effort, not credentials, sessions or permissions. `EV_PROVIDER` explicitly overrides startup routing. Exact configured/custom IDs remain usable without a successful catalogue; no model is invented as an observed default. Preference failures stay visible with `/storage-retry`; exit waits for commit. App-owned credentials are separately and explicitly persisted as described above.

**Schema history:** v0.1.11/0.1.12 used 13, v0.1.13 used 14, v0.1.14–v0.1.16 used 15, and v0.1.17–v0.1.19 used 16. **v0.1.20 migrates through PK04's Schema 17 held references to Schema 18 milestone references. Cleanly exit all instances and back up the entire data directory before opening it**, including database, journals and outputs. Migration preserves prior task/partner/context/evidence data; older Schema 16/17 writers refuse the newer store, and downgrade requires restoring the full backup. Earlier public packages are immutable. Complete Claude SDK bridge revision **9** source/policy/dependency files are required, not only an SDK package update.

**v0.1.22 preference compatibility:** Schema 18 and SDK bridge 9 remain unchanged, but enabling persistent reduced motion extends the strict preference JSON. **Exit all instances and back up the complete data directory before upgrading.** v0.1.21 refuses a store carrying the new setting without changing its bytes; reinstalling an older binary is not a supported downgrade. Restore the full pre-upgrade backup instead.

**Moves.** Each partner remembers up to four moves per form, visible across its project and personal scopes. Teach exact approved argv/cwd, reusable pin selectors, a submitted user-authored instruction or goal/criteria; previews never package provider output or memories. Move names are editable local labels, not capabilities or provider context. Every use revalidates current inputs and opens normal command, pin or full-receipt request review; task templates require explicit application. Repeating an eligible approved run or dispatched pin set three times creates a local suggestion, never automatic work. Not now waits three further uses, Never persists, and `/move-reminder` revisits forgotten/postponed moves. A full form offers an explicit scoped replacement or Give up learning. Details retain authoritative journal outcomes, separately showing denial/cancellation and unknown provider completion. Party teaching copies steps/provenance, not grants.

**Formations.** `/formation-save <literal path>` reviews a bounded schema-2 recipe with the party's form/request role slots; `y` creates a new file and never overwrites. `/formation <literal path>` shows explicit local mappings: Up/Down selects a role, Left/Right selects an existing partner or leaves it unmapped, `d` inspects the complete definition, and `y` applies. Nonempty Request drafts and carets win; empty drafts may receive the reviewed proposal. Missing slots never create partners, duplicate/busy/stale targets refuse atomically, and applying never sends, runs, selects context or transfers a session/grant. Esc cancels; neutral Enter does not apply.

**Portable TMs.** `/move-export <literal path>` reviews the selected move's inert schema-1 TOML, bounded to 64 KiB. `/move-import <literal path>` reviews its form/kind/steps and lets you choose a local partner, name, project/personal scope and explicit free/replacement slot. All nine forms and four step kinds round-trip; foreign database IDs, teacher authority, sessions and prior success are not imported. Unsupported versions, unsafe paths and recognizable secrets refuse; an existing export remains byte-identical. Missing pin files are accepted as unavailable selectors until fresh ordinary use-time capture/review, never automatically substituted or attached.

**Held context (PK04).** The normal workspace shows the saved identity, concise inclusion state and fixed `/held` inspect control at 60×18 and 120×36, including long/CJK names. `/held` inspects the current project's partner/form slot and its local request receipt. `/held-file` opens Files; after inspecting actual source, `/held-set-file` remembers that file identity without attaching it. `/held-memory` opens approved Bond memories; `/held-set-memory` remembers the highlighted exact approved revision without selecting it. A form/owner/task entry proposes the saved item as a removable chip. `/held-accept` uses current source review/Pin or explicit memory selection; `/held-dismiss` removes only this visit's chip and `/held-clear` clears the saved slot. Neither clearing nor saving changes independently selected context. Ordinary `/unpin` and `/select-memory` control accepted context.

Held proposals never enter an outgoing request until explicitly accepted. Current exclusions, source identity/freshness and memory revision/visibility are revalidated before send; unavailable items do not substitute new bytes or broader scope. A proposal arising when automatic evolution submits a request is for the next request, not the already-frozen one. Held labels/metadata stay out of provider packets and handoffs, add no model calls or permissions, and do not cover safety/live-code feedback.

**Ribbons (PK06).** `/ribbons` opens optional project history for the first recorded effective accepted review, verified-support handoff and explicitly learned move. These are source-linked historical receipts, not current correctness, scores or unlocks. Each retains the actual producer and original task/artifact or move revision; later events never replace it. Partner summaries show only that partner's nonhidden history while flavor is on. In the inspector, Up/Down selects, `h` or `/ribbon-hide`/`/ribbon-show` changes durable visibility, and Enter or `/ribbon-inspect` opens ordinary evidence inspection. Foreign source tasks/owners/forms require deliberate selection; inspection never sends, executes or exports.

Requested acceptance, unverified exports, provider text and teaching previews/candidates never qualify. Current/stale/unavailable state and its reason stay inspectable; hiding retains the source, while purging its evidence removes receipt payloads. Content-free kind tombstones prevent repeated first awards until final project purge. No ribbon metadata enters provider context or portable handoffs, and no model call, permission or automatic action is added.

**Cooperation.** Up to three partners can own work on one task, with independent requests, transports, drafts, approvals, receipts and cancellation. Edit-capable work takes one durable per-task lease; another writer is refused with the owner's identity. Commands overlapping that lease are permanently labelled `revision moving`, never current verification. This coordinates Eevee-owned work, not arbitrary filesystem effects or a provider sandbox. Per-request provider usage/billing remains unknown when unreported; local payload estimates are labelled separately.

**Form relays.** The specialist desk can stage supported hypotheses or review findings as a bounded Flareon fix, import a chosen Espeon decision as editable criteria, and reuse eligible passing checks as Leafeon baselines. Relays show source references and offer an idle matching partner instead of changing the sender's form; they never submit work or transfer permissions. Changes touching a relayed finding's ranges require fresh independent review, not automatic closure.

Cancel-all and quit retain neutral Stay and wait for terminal outcomes and durable settlement. Crash recovery never replays requests or restores grants. Releasing an idle partner removes current drafts, moves, reminders and pairings; historical evidence retains producer snapshots. Explicit project/partner/form pairings are independent of global defaults: remembered selection never connects or grants access, and unavailable routes never silently fall back.

**Records and habits.** Summaries count distinct actually produced tasks and saved moves, not levels or affection. Legacy producers stay unknown; routes prove navigation only. Dex resolutions are reviewed historical leads, not causal claims or present verification. Three distinct actually dispatched matching authored instructions may propose a habit. Only explicit project-memory consent enables exact-repeat substitution, while the exact approved memory revision is selected. Drafts and prior conversation history remain unchanged; provider output never becomes an inferred preference.

Clean exit waits for local preference/history persistence without asking to cancel nonexistent work. Active requests/commands, unknown outcomes, failed journals and unsaved task drafts retain their own safety decisions.

Without an app-owned connection override, persistent **nonsecret** settings use `$XDG_CONFIG_HOME/eevee/providers.toml` (Linux; default `~/.config/eevee/providers.toml`), `~/Library/Application Support/eevee/providers.toml` (macOS), or `%APPDATA%\eevee\providers.toml` (Windows). `EV_PROVIDERS_CONFIG` selects an absolute user-owned file outside the project. Environment settings override that nonsecret file:

```toml
[openai]
model = "your-model-id"
api_key_env = "MY_OPENAI_KEY"

[compatible]
base_url = "http://127.0.0.1:11434/v1"
model = "your-local-model"
keyless = true
protocol = "chat_completions" # or "responses"
```

ChatGPT keeps its existing nonsecret `CODEX_HOME/config.toml` model/effort defaults; `CODEX_MODEL` and `CODEX_REASONING_EFFORT` override them. Selecting a different model does not carry the old model's configured effort into it. A missing/expired/keychain-only session is reported explicitly; the native connection never silently runs a login or upgrades Codex.

Existing external-agent connections remain available:

| Provider | Integration | Sign-in |
| --- | --- | --- |
| Codex latest stable | stdio app-server | Browser OAuth or device authorization |
| Claude Agent SDK latest stable | Optional Node 24+ sidecar | API key, Console OAuth via `ant`, Bedrock or Vertex (no claude.ai subscription reuse) |
| OpenCode latest stable | ACP (`acp --pure`) | OpenCode's own `auth login` |
| Gemini CLI latest stable | ACP (`--acp`) | Gemini's native CLI (OAuth, API key or Vertex) |
| Devin CLI latest stable | ACP (local agent) | Website OAuth (in-memory session credential) or the CLI's browser OAuth / manual token |

Installed-agent admission binds executable/configuration identity to the review and validates actual protocol and permission capabilities. Release numbers, including prerelease labels, are evidence rather than exact-version allowlists. Native services do not use these executable probes. On WSL/SSH, the optional Codex CLI login supports device authorization instead of a localhost callback.

Explicitly install or update the latest stable CLI, then **Refresh installed providers** in `/connect`:

```sh
npm install -g @openai/codex@latest
npm install -g opencode-ai@latest
npm install -g @google/gemini-cli@latest
```

For Devin, use its [official installer](https://docs.devin.ai/cli) and `devin update`. For Anthropic Console login, install/update `ant` with `go install github.com/anthropics/anthropic-cli/cmd/ant@latest` or the [official Homebrew instructions](https://github.com/anthropics/anthropic-cli#installation). These are explicit user actions; opening Eevee never silently installs or globally upgrades providers.

The Claude sidecar's `latest` dependency is refreshed by setup; repeat the same command to update:

```sh
npm run setup --prefix sidecar
export EEVEE_CLAUDE_SIDECAR="$PWD/sidecar/main.ts"
```

`npm ci` reproduces the checked-in lock snapshot rather than fetching latest. For an existing independent sidecar installation, `npm install --ignore-scripts --include=optional @anthropic-ai/claude-agent-sdk@latest` explicitly refreshes the SDK. Eevee checks the loaded SDK/embedded CLI against the reviewed dependency tree and bridge policy before releasing a prompt.

Credential routes, enforcement boundaries and per-provider limits: [Agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Data and trust

- Tasks, drafts, excerpts and journals live in local SQLite (`~/.local/share/eevee`, `~/Library/Application Support/eevee`, or `%LOCALAPPDATA%\eevee`). Schema upgrades are transactional; newer or corrupt databases are never silently reset.
- Ordinary typing/navigation autosaves after 500ms of quiet, with at most 2s deferral during continuous edits. `Storage: autosave pending` is not yet durable; `Storage: saved` means SQLite acknowledged the snapshot. During an active provider request, follow-up edits remain visibly pending until its terminal acknowledgment so saves cannot invalidate reviewed input or permission checks. Ctrl+S cannot bypass that ownership boundary; safe exit cancels/waits, then flushes. Outside active requests, explicit saves and task/partner transitions flush immediately.
- Recognizable credentials are filtered before storage, but masking is incomplete; review exports.
- Provider configuration, hooks, MCP and resumed history can add effects beyond the displayed payload. Provider read-only policy is not an Eevee sandbox.
- Restart never replays commands or model turns; unresolved outcomes stay visible.

Sprites are nine original PokeAPI images that installers download and hash-verify into your cache (`ev --download-artwork` for source builds). Images render with Sixel/Kitty when the terminal supports them, otherwise as text. Limits and recovery: [Troubleshooting](docs/TROUBLESHOOTING.md).

## Installation

| Machine | Package |
| --- | --- |
| macOS Apple Silicon | [ev-darwin-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.22/ev-darwin-arm64.tar.gz) |
| macOS Intel | [ev-darwin-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.22/ev-darwin-x86_64.tar.gz) |
| Linux x86_64 | [ev-linux-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.22/ev-linux-x86_64.tar.gz) |
| Linux arm64 | [ev-linux-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.22/ev-linux-arm64.tar.gz) |
| Windows x86_64 | [ev-windows-x86_64.zip](https://github.com/kimmandoo/eevee/releases/download/v0.1.22/ev-windows-x86_64.zip) |

Installers verify SHA-256, refuse to overwrite commands they don't own, and never edit your shell profile or persistent PATH — the last step on each platform is yours.

### macOS

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.22/install.sh
sh ./install-ev.sh --version v0.1.22          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && exec zsh
```

Builds are not notarized. The script installs via `curl`, which sets no quarantine flag; if you unpacked the archive from a browser download and macOS blocks `ev`, run `xattr -d com.apple.quarantine ~/.local/bin/ev`.

### Linux and WSL

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.22/install.sh
sh ./install-ev.sh --version v0.1.22          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc && exec bash
```

Inside WSL, use this Linux build, not the Windows one. For provider sign-in on WSL, prefer Codex **device authorization**.

### Windows

Run in **PowerShell** (5.1 or 7, no admin). Windows blocks downloaded `.ps1` files by default, so run the installer as a script block instead of `.\install.ps1`:

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/download/v0.1.22/install.ps1))) -Version v0.1.22
```

It installs `%LOCALAPPDATA%\ev\bin\ev.exe`. Use it right away, and keep it on PATH for new terminals:

```powershell
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
[Environment]::SetEnvironmentVariable('Path', "$env:LOCALAPPDATA\ev\bin;" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')
```

Prefer a saved script? Run it with a process-only bypass: `powershell -NoProfile -ExecutionPolicy Bypass -File .\install-ev.ps1 -Version v0.1.22`. Windows Terminal shows the sprites (Sixel); other consoles fall back to text.

### Offline

Download the archive for your machine plus `SHA256SUMS` from the table above, then:

```sh
sh ./install-ev.sh --archive ./ev-linux-x86_64.tar.gz --checksums ./SHA256SUMS
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install-ev.ps1 -Archive "$PWD\ev-windows-x86_64.zip" -Checksums "$PWD\SHA256SUMS"
```

Offline installs skip the sprites; run `ev --download-artwork` once you are online.

<details>
<summary>Build from source</summary>

Requires Rust 1.90+:

```sh
git clone https://github.com/kimmandoo/eevee.git && cd eevee
cargo build --locked --release
./target/release/ev
```

</details>

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
npm ci --ignore-scripts --no-audit --no-fund --prefix sidecar && npm test --prefix sidecar
```

[CI](.github/workflows/native.yml) runs those checks once on Linux, then tests, builds, packages and offline-installs on macOS arm64/x86_64, Linux x86_64/arm64 and Windows x86_64. Tagging a commit whose `main` run fully passed [publishes](.github/workflows/release.yml) those exact packages.

Real-terminal journeys live in `scripts/*_smoke.py` (need `pyte==0.8.2` and `wcwidth==0.2.13`), e.g. `python scripts/workbench_smoke.py --binary target/release/ev --output target/workbench-smoke`. Live provider checks are opt-in (`--live`) and may incur usage.

For Windows verification, use an unpackaged Python installation (for example, isolated uv-managed Python), not a Microsoft Store/App Execution Alias interpreter. This host's Store interpreter launched fixture descendants outside the tracked Job, so cancellation correctly remained unknown. This is a smoke-host prerequisite, not a Python dependency of `ev`; see [troubleshooting](docs/TROUBLESHOOTING.md).

Source layout, by role:

| Path | Role |
| --- | --- |
| `src/main.rs`, `config.rs`, `domain.rs` | Entry point, CLI/config parsing, form identities |
| `src/app.rs`, `src/app/` | Interaction state and key handling per surface; `app/planning/{picker,review,journal}.rs` separate planning responsibilities |
| `src/ui.rs`, `src/ui/` | Rendering per surface; `ui/artwork.rs` sprites and native graphics |
| `src/store/` | SQLite task/memory/journal/run persistence, schema/recovery/admission, transactional cooperation in `work`, bounded move/reminder/usage storage in `moves`, and reference-only held preferences in `held` |
| `src/workspace/` | Local work: `repo`, `context`, `runner`, `receipt`, `privacy`, `handoff`, `memory`, `workflows`, typed move validation/naming in `moves`, and held identities/review proofs in `held` |
| `src/providers.rs`, `src/providers/` | Provider request/event contract; `acp`, `codex`, `claude`, `agent` preflight, `auth` |
| `src/platform/` | OS boundary: `terminal`, `input`, `native`, `distribution` |

Large modules keep their unit tests in a sibling `tests.rs` (for example `src/store/tests.rs`).

Please keep the approval, storage and cancellation contracts intact, add a focused regression for behavior fixes, and never commit credentials, task data or downloaded artwork.

| Document | Purpose |
| --- | --- |
| [Design rules](docs/DESIGN_RULES.md) | Product, visual, privacy and lifecycle contract |
| [Agent compatibility](docs/AGENT_COMPATIBILITY.md) | Protocols, authentication and control boundaries |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Architecture and acceptance gates |
| [Task backlog](task.md) | Unfinished work, Pokémon-first milestones and genuine prerequisites |
| [Completed work](tasks/COMPLETED.md) | Archived specifications, stable task IDs and scoped completion evidence |
| [Work checkpoint](docs/WORK_CHECKPOINT.md) | Verification evidence and platform limits |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Terminal and operational diagnostics |

## License and attribution

Original code, documentation prose, and the custom geometric wordmark are licensed under [Apache-2.0](LICENSE). See the [project NOTICE](NOTICE) and [asset licensing and attribution](docs/ASSET_LICENSES.md) for the separate third-party scope.

The README's transparent Eevee sprite is third-party Pokémon pixel art, referenced from a [pinned PokéAPI/sprites revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5). Its [upstream notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) explicitly attributes image copyright to **The Pokémon Company**. The project's Apache-2.0 license does not cover that artwork or grant trademark, redistribution, or commercial-use rights.

No franchise image bytes are bundled in the repository or native packages. Remote display, installation downloads, and explicit runtime downloads do not establish rights clearance. This is an independent, unofficial project with no affiliation or endorsement; the software release grants no third-party image or trademark rights. The original wordmark has an editable [SVG source](docs/assets/wordmark.svg), not an embedded character image or third-party font.
