<div align="center">

<img src="https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/133.png" width="192" height="192" alt="Original Eevee pixel sprite with a transparent background">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.png">
    <img src="docs/assets/wordmark-light.png" width="332" height="68" alt="eevee">
  </picture>
</h1>

**A local-first terminal workbench for LLM-assisted development.**

One persistent partner. Nine task-specific forms. Your work stays local until you choose otherwise.

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

```sh
ev update --check   # read-only release check
ev update           # upgrade in place
ev uninstall        # remove the binary; keep user data
```
Install and `ev update` show the same animation from release lookup through installation, respecting UTF-8 and `NO_COLOR`. Redirected output, CI and dumb terminals use static phase messages; `ev update --check` remains read-only.


Use a terminal of at least **60 × 18** (120 × 36 recommended).

1. Type in the bottom composer. **Enter** saves and sends the request; **Ctrl+S** saves locally only. No separate Goal setup or routine approval popup.
2. `/provider` chooses a service. **Codex** is one entry with native responses (no CLI/tools) or CLI agent (tools/edits) as connection methods. `/model` follows **model → reasoning level → editor** when reasoning is supported; `/effort` opens the level directly.
3. Approve actual effects when needed: commands, file changes, write-enabled sessions, project-supplied executables, external credential helpers and unmanaged agent connections. Advanced `/new` or `/resume` keeps exact-payload review available.
4. Read responses inline and type the next request. The last provider/method, model and effort are restored on restart; credentials, sessions and grants are not.
5. Use `/run` → `/command-edit` for a separately reviewed local command such as `["cargo","test"]`.

**Provider:** on a ready row, **Enter** uses the provider directly; on the active row it opens models. **Right** opens connection/setup actions. Unready rows keep the setup path, and switching an owned provider context still uses its existing review.

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
| `/connect` / `/provider` | Manage provider sign-in / select the execution provider |
| `/model` / `/effort` / `/models-refresh` | Local model and reasoning pickers / reviewed catalogue refresh |
| `/new` / `/resume` | Review a fresh request / resume an owned same-task session |
| `/role` / `/forms` | Plan, Implement or Review / choose an evolution |
| `/goal` / `/next` / `/criteria` / `/criteria-edit` | Edit task facts, inspect criteria, or edit criteria directly |
| `/tasks` / `/task-new` | Browse saved tasks / create one |
| `/reset` | Review a fresh local task; save the old task and drafts for `/tasks`, without erasing files or history |
| `/init` | Prepare an editable Plan request for a repository-specific `AGENTS.md` proposal; no automatic send or file write |
| `/files` / `@` / `/range` | Source context / attach files / attach line ranges |
| `/run` / `/command-edit` | Local execution / author argv or an explicit shell object |
| `/diff` / `/history` / `/decision` | Worktree diff / provider journal / save a proposal as next action |
| `/handoff` / `/memory` / `/workflows` | Portable handoff / approved notes / diagnosis, refactor, review and recipe desk |
| `/workflow-diagnosis` / `/workflow-refactor` / `/workflow-review` / `/workflow-recipe` | Open the chosen specialist desk directly |
| `/sessions` / `/paths` | Review session metadata inspection / edit the implementation path draft directly |
| `/images` / `/inspect` / `/companion` | Sprites / details / companion |
| `/quit` / Ctrl+C | Safe exit review |

Pasted text never sends a request or executes a command. Model/effort pickers and `/models-refresh` fetch metadata without a second popup for qualified connections, including on a fresh home, without task content. Risky executable/unmanaged connection effects still require review. Errors retain a retry path and stale rows. Pickers distinguish the saved choice from keyboard focus with color and text markers; catalogue provenance and errors remain available in details. After reasoning selection (or immediately for models without it), return to the original draft/caret without sending.

`/reset` defaults to Stay and waits for durable task/context saves before switching, preserving old work in `/tasks`. Finish or explicitly cancel active work first. `/init` selects Espeon / Plan, clears write paths and prepares an editable project-guide proposal; replacing a nonempty Request requires confirmation. It sends nothing and overwrites no `AGENTS.md` by itself. **Enter** sends when you are ready; other drafts remain retained.

**Missing newer Codex models?** Open **Codex** in `/provider` and choose **native responses** for live account discovery and streaming without an installed CLI. It uses an existing Codex session read-only or an environment token, only against the official backend. The CLI method remains available for agent tools/edits and shows its executable/catalogue provenance. Both preserve exact configured/custom IDs despite stale lists, never silently switch models, and retain previous rows with an error on failed refresh. Model search is local; Ctrl+U clears it.

**SDK / ACP model choices.** Their lists are informational too: an exact custom ID may be accepted through the integration's official selector even when omitted by discovery. Real selector support and exact model acknowledgment remain required. Claude SDK effort is requested through official Options; applied effort is unknown and provider caps may apply. Existing optional Claude installations must explicitly update the complete bridge to revision 3, not only its SDK package.

**Home and responses.** At 90 columns or wider, the partner card stays visible beside the response; smaller and linear layouts keep full-width output with `/companion` for the partner. Replies stream inline without an extra output window or artificial typing delay. The header shows the single live progress indicator; the response labels its own complete/streaming/partial state. Earlier output stays attributed to its earlier request. Completed words can appear before a newline; unfinished tokens and ambiguous credential prefixes remain withheld for best-effort redaction. PgUp/PgDn pause follow and Alt+End returns to the newest output without stealing ordinary draft input. Exact provenance and technical details remain optional. The spinner represents actual preparation/connection/response activity, not estimated progress; permission reviews and idle screens do not animate. `--no-animation` keeps the indicator static without slowing streamed text.

**Request receipt.** Every provider preview breaks the exact payload Eevee sends into instructions, task facts, attachments, handoff and memories, with exact bytes and a labelled local token estimate (`eevee-local-v1`, CJK-aware). It runs offline. Provider-added context, usage and billing stay **unknown**, not zero.

## Connect a provider or agent

Native services do not require a provider CLI, Node or an SDK sidecar. Keys are resolved in memory from environment variables; user-scoped configuration contains endpoint/model settings and **environment-variable names**, never literal credentials or shell commands. ChatGPT can read an existing provider-owned session file at the disclosed connection boundary; it does not copy, refresh or rewrite that file. No credentials enter task storage.

| Native service | Selection | Credential |
| --- | --- | --- |
| Codex / native method | Codex → native responses; explicit shortcut `/provider-chatgpt` | Existing `$CODEX_HOME/auth.json` (default `~/.codex/auth.json`), or `CODEX_ACCESS_TOKEN` / `CODEX_ACCOUNT_ID` |
| OpenAI Responses | `/provider-openai` | `OPENAI_API_KEY`; optional `OPENAI_BASE_URL`, `OPENAI_MODEL` |
| Anthropic Messages | `/provider-anthropic` | `ANTHROPIC_API_KEY`; optional `ANTHROPIC_BASE_URL`, `ANTHROPIC_MODEL`. No subscription OAuth reuse. |
| Google Gemini | `/provider-google` | `GEMINI_API_KEY` or `GOOGLE_API_KEY`; optional `GEMINI_BASE_URL`, `GEMINI_MODEL` |
| Compatible / local service | `/provider-compatible` | `EV_API_BASE_URL`, `EV_API_MODEL`; `EV_API_KEY`, or explicit `EV_API_KEYLESS=1` for a keyless service |

Native routes stream text without tools, hooks, implicit project reads or local writes; Implement replies are proposals, not applied edits. Use the CLI/controlled-agent method for tool-driven edits. Native calls are fresh, stateless requests. Enter sends the committed request without a second Run popup; cancellation closes the client but cannot prove remote work/billing stopped. Custom endpoints require HTTPS except loopback HTTP; redirects are refused.

The local Schema 10 store remembers last-used provider/method and per-route model/effort, including exact custom IDs without a successful catalogue. `EV_PROVIDER` explicitly overrides the startup route (`chatgpt` for Codex native, `codex` for CLI, or another service/agent name); that route's own stored model remains available. With no stored preference, configured native routes are preferred, then CLI Codex. Default model selection follows provider configuration rather than inventing an observed ID. Preference failures stay visible with `/storage-retry`; exit waits for commit. No credentials, sessions or permissions are restored, and older binaries refuse the migrated store rather than rewriting it.

For persistent **nonsecret** settings, use `$XDG_CONFIG_HOME/eevee/providers.toml` (Linux; default `~/.config/eevee/providers.toml`), `~/Library/Application Support/eevee/providers.toml` (macOS), or `%APPDATA%\eevee\providers.toml` (Windows). `EV_PROVIDERS_CONFIG` selects an absolute user-owned file outside the project. Environment settings override the file:

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
| macOS Apple Silicon | [ev-darwin-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.8/ev-darwin-arm64.tar.gz) |
| macOS Intel | [ev-darwin-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.8/ev-darwin-x86_64.tar.gz) |
| Linux x86_64 | [ev-linux-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.8/ev-linux-x86_64.tar.gz) |
| Linux arm64 | [ev-linux-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.8/ev-linux-arm64.tar.gz) |
| Windows x86_64 | [ev-windows-x86_64.zip](https://github.com/kimmandoo/eevee/releases/download/v0.1.8/ev-windows-x86_64.zip) |

Installers verify SHA-256, refuse to overwrite commands they don't own, and never edit your shell profile or persistent PATH — the last step on each platform is yours.

### macOS

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.6/install.sh
sh ./install-ev.sh --version v0.1.6          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && exec zsh
```

Builds are not notarized. The script installs via `curl`, which sets no quarantine flag; if you unpacked the archive from a browser download and macOS blocks `ev`, run `xattr -d com.apple.quarantine ~/.local/bin/ev`.

### Linux and WSL

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.6/install.sh
sh ./install-ev.sh --version v0.1.6          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc && exec bash
```

Inside WSL, use this Linux build, not the Windows one. For provider sign-in on WSL, prefer Codex **device authorization**.

### Windows

Run in **PowerShell** (5.1 or 7, no admin). Windows blocks downloaded `.ps1` files by default, so run the installer as a script block instead of `.\install.ps1`:

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/download/v0.1.6/install.ps1))) -Version v0.1.6
```

It installs `%LOCALAPPDATA%\ev\bin\ev.exe`. Use it right away, and keep it on PATH for new terminals:

```powershell
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
[Environment]::SetEnvironmentVariable('Path', "$env:LOCALAPPDATA\ev\bin;" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')
```

Prefer a saved script? Run it with a process-only bypass: `powershell -NoProfile -ExecutionPolicy Bypass -File .\install-ev.ps1 -Version v0.1.6`. Windows Terminal shows the sprites (Sixel); other consoles fall back to text.

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
| `src/store/` | SQLite store: `tasks`, `memories`, `journal`, `runs`, `spool`, `schema`, `admission`, `recovery` |
| `src/workspace/` | Local work: `repo`, `context`, `runner`, `receipt`, `privacy`, `handoff`, `memory`, `workflows` |
| `src/providers.rs`, `src/providers/` | Provider request/event contract; `acp`, `codex`, `claude`, `agent` preflight, `auth` |
| `src/platform/` | OS boundary: `terminal`, `input`, `native`, `distribution` |

Large modules keep their unit tests in a sibling `tests.rs` (for example `src/store/tests.rs`).

Please keep the approval, storage and cancellation contracts intact, add a focused regression for behavior fixes, and never commit credentials, task data or downloaded artwork.

| Document | Purpose |
| --- | --- |
| [Design rules](docs/DESIGN_RULES.md) | Product, visual, privacy and lifecycle contract |
| [Agent compatibility](docs/AGENT_COMPATIBILITY.md) | Protocols, authentication and control boundaries |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Architecture and acceptance gates |
| [Task backlog](task.md) | Open work index and recommended order; details in `tasks/` |
| [Work checkpoint](docs/WORK_CHECKPOINT.md) | Verification evidence and platform limits |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Terminal and operational diagnostics |

## License and attribution

Original code, documentation prose, and the custom geometric wordmark are licensed under [Apache-2.0](LICENSE). See the [project NOTICE](NOTICE) and [asset licensing and attribution](docs/ASSET_LICENSES.md) for the separate third-party scope.

The README's transparent Eevee sprite is third-party Pokémon pixel art, referenced from a [pinned PokéAPI/sprites revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5). Its [upstream notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) explicitly attributes image copyright to **The Pokémon Company**. The project's Apache-2.0 license does not cover that artwork or grant trademark, redistribution, or commercial-use rights.

No franchise image bytes are bundled in the repository or native packages. Remote display, installation downloads, and explicit runtime downloads do not establish rights clearance. This is an independent, unofficial project with no affiliation or endorsement; the software release grants no third-party image or trademark rights. The original wordmark has an editable [SVG source](docs/assets/wordmark.svg), not an embedded character image or third-party font.
