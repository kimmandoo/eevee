<div align="center">

<img src="https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/133.png" width="128" height="128" alt="Eevee pixel sprite">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.png">
    <img src="docs/assets/wordmark-light.png" width="300" height="61" alt="eevee">
  </picture>
</h1>

**A local-first coding agent for your terminal — bring any model, review every change.**

[![Native CI](https://github.com/kimmandoo/eevee/actions/workflows/native.yml/badge.svg?branch=main)](https://github.com/kimmandoo/eevee/actions/workflows/native.yml)
[![Release](https://img.shields.io/github/v/release/kimmandoo/eevee?sort=semver)](https://github.com/kimmandoo/eevee/releases/latest)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)

[Install](#install) · [Quick start](#quick-start) · [Meet the party](#meet-the-party) · [Providers](#providers) · [Safety model](#safety-model) · [Your data](#your-data) · [Build from source](#build-from-source)

</div>

---

<p align="center">
  <img src="docs/assets/screenshot.png" width="900" alt="The ev terminal app with an open response, highlighted Rust code and diagnostics, and the Eevee pixel portrait">
  <br>
  <sub>Captured in Ghostty on macOS. The response comes from a deterministic local fixture, not a live model. <a href="docs/ASSET_LICENSES.md">Artwork attribution and rights limits.</a></sub>
</p>

`ev` is a single native binary that turns any folder into an agent workspace. Type a request; the model you connected reads your project, writes files, runs builds and tests, reads the real errors and fixes them — with any provider, and with every effect shown to you before it happens.

It also comes with a companion: a party of Eevee forms that evolve with the kind of work in progress. Every form can code; the form changes how the work is framed, never what it is allowed to do.

## Highlights

- **Any model.** OpenAI (account or API), Anthropic, Google Gemini and any OpenAI-compatible endpoint, including local Ollama, vLLM and LM Studio. External agents work too: Codex, the Claude Agent SDK, and OpenCode / Gemini / Devin over ACP.
- **A real coding loop.** Discover, read, create, edit, rename and delete files; run builds, tests and programs; feed the actual output back to the model. Code streams into the transcript as it is written.
- **You review every effect.** Each file change and command opens a compact review with the exact diff or argv. Trust a run with `/auto-approve` (this task) or `/auto-approve-session` (**Shift+A**, this session).
- **Rules, not forgotten grants.** Turn a reviewed `cargo test` or `tests/**` edit into a project rule with `/rule-allow`. Deletes, shells and unknown effects always ask again.
- **Steer without cancelling.** Press Enter while the model works to add a correction at the next tool boundary, or queue your next request.
- **Clear outcomes.** Failures name their cause and a next step; unfinished work shows as Incomplete, and uncertain results stay Unknown instead of looking successful.
- **Local-first.** Tasks, drafts, transcripts and run history live in local SQLite. No account, server or Node runtime needed.
- **Built for the terminal.** Rust + ratatui, 60×18 and up, mouse and keyboard, Kitty/Sixel sprites with text fallback, ASCII/mono modes. macOS, Linux, WSL and Windows, all built and tested in CI.

See the [changelog](CHANGELOG.md) for what changed in each release.

## Install

**macOS / Linux / WSL**

```sh
curl -fsSL https://github.com/kimmandoo/eevee/releases/latest/download/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
```

**Windows (PowerShell 5.1 / 7, no admin)**

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/latest/download/install.ps1)))
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
```

Installers verify SHA-256 checksums, never edit your shell profile or persistent `PATH`, and on Windows return to the calling PowerShell session without changing execution policy.

```sh
ev update --check   # is there a newer release?
ev update           # upgrade in place
ev uninstall        # remove the binary; your data stays
```

<details>
<summary>Pinned versions, offline archives and persistent PATH</summary>

Pin a release by its tag (see [Releases](https://github.com/kimmandoo/eevee/releases)):

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/latest/download/install.sh
sh ./install-ev.sh --version vX.Y.Z
```

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/latest/download/install.ps1))) -Version vX.Y.Z
```

Offline: download the matching archive, `SHA256SUMS` and installer from one release, then:

```sh
sh ./install-ev.sh --archive ./ev-linux-x86_64.tar.gz --checksums ./SHA256SUMS
```

```powershell
powershell -NoProfile -File .\install.ps1 -Archive "$PWD\ev-windows-x86_64.zip" -Checksums "$PWD\SHA256SUMS"
```

To keep `ev` on `PATH`, add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile, or on Windows add `%LOCALAPPDATA%\ev\bin` to your user `Path`. macOS builds are not notarized and Windows packages are unsigned; WSL uses the Linux packages.

</details>

<details>
<summary>Windows troubleshooting</summary>

If Windows says `ev.exe` contains a virus or potentially unwanted software, **leave the block in place**. `Unblock-File` and execution-policy changes do not resolve an antivirus detection, and you should not disable Defender or add an exclusion. Check **Windows Security → Virus & threat protection → Protection history** and report the threat name, the `ev.exe` path, the release version and the Defender security-intelligence version. A matching SHA-256 proves the bytes came from the release, not that a file is safe.

If `ev.exe` is missing but `.ev-install` remains, do not delete the metadata or your task data: rerun the installer, which repairs that state after validating its ownership marker. Modified or unowned files still refuse.

If the window closes immediately, run these from an already-open PowerShell window and include the output and terminal (Windows Terminal or classic console) in a report:

```powershell
(Get-Command ev.exe).Source
ev.exe --version
ev.exe
"exit=$LASTEXITCODE"
```

`ev` needs a real terminal; do not pipe or redirect its input or output.

</details>

## Quick start

```sh
cd your-project
ev
```

1. **Connect** — `/connect`, pick a provider, pick a model. Setup never sends your draft.
2. **Ask** — type what you want and press **Enter**:
   > Create a snake game in Python, run it, and fix anything that breaks.
3. **Review** — inspect each proposed change or command, then choose and confirm. File reviews start with **no choice**: **Right** selects Allow once, **Left** selects Deny, and **Enter** confirms the visible choice.

The folder you launch from is the project boundary. Git is optional; `/init` drafts an `AGENTS.md` guide if you want one.

### Coding, running and downloading

Ordinary coding requests expose file and command tools automatically; importing a tool catalog is optional. Ask for the whole result—for example, “create the program, build it, fix the real errors, run it and download the required resource.” Builds, programs and requested downloads use the same reviewed command runner. A download needs an installed downloader or runtime; fetching bytes does not authorize executing them.

`/tools` shows the selected route's source, editing, execution and download capabilities, plus eligible selected MCP tools. Explicit Plan/Review disables file changes and commands; `/role-implement` restores ordinary coding for the next Send. MCP connections still require separate launch review and explicit tool selection.

Tool calls stay in the response transcript, including on wide terminals: `[..]` awaits a result, `[ok]` reports success, `[!!]` reports failure, `[--]` reports a stopped request/tool and `[?]` an unknown result. A request ending without a tool result never becomes success or keeps running indefinitely; the note distinguishes a stopped request from proof of remote tool cancellation. Command rows retain the actual exit and bounded output; `/verbose` exposes complete retained detail. A definite compiler failure can be corrected within the same request while its todo remains actionable. Denied permissions and unknown effects never authorize replay.

### Sessions

```sh
ev                  # fresh session; saved work remains saved
ev --continue       # last selected saved session in this project
ev --resume         # choose saved work; Esc returns to a blank session
ev --resume 12      # exact project-local task number
```

Inside `ev`, `/new` opens fresh work and `/resume` opens the saved-work picker; neither sends anything. Saved facts, drafts and carets come back. Provider threads, process-local chat history and approvals never resume after a restart.

### Reviews

| In the review | What to do |
| --- | --- |
| Inspect a file change | **Enter** on arrival, or **d** for the complete change and exact request. Enter in details does not approve. |
| Decide | **Left/Right** chooses Deny / Allow once, then **Enter** confirms. |
| Inspect a command | Argv, working directory, resolved executable and effects; **d** opens full details, **PgUp/PgDn** scroll. |
| Defer | **Esc** keeps the request pending and your draft intact; **Alt+A** or `/approvals` reopens it. |
| Grant broader trust | **Shift+A** in a review grants this session; `/auto-approve` grants this task. |

Hidden or undersized reviews, skipped scope, typing, paste and held keys cannot approve an effect. Highlighted code and diagnostics help reading; they are not proof that code ran or passed.

### Everyday controls

| Want to… | Use |
| --- | --- |
| Find any action | `/` or **Ctrl+K** |
| Connect / switch model / reasoning effort | `/connect` · `/model` · `/effort` |
| Attach a file or line range as context | `@` → pick → **Enter** (**r** for a range) |
| Auto-approve this task / this session / stop | `/auto-approve` · `/auto-approve-session` · `/auto-approve-off` |
| Reopen a deferred review | **Alt+A** or `/approvals` |
| Read earlier output | wheel or **PgUp/PgDn**; **Alt+End** follows live output |
| Inspect available tools / why execution is disabled | `/tools` |
| See raw tool output and IDs | `/verbose` |
| Run your own command | `/command-edit` → `/execute`; history in `/run` |
| Restrict which files may be written | `/paths` (`[]` clears it) |
| Plan or review without writing | `/role-plan` · `/role-review` |
| Track the plan | `/todo` |
| Start fresh / reopen saved work / clear native chat | `/new` · `/resume` · `/chat-new` |
| Stop / quit | **Ctrl+C** stops work, again (or while idle) saves and exits · `/cancel` · `/quit` |

```sh
ev --theme light          # light palette
ev --ascii --color mono   # plain terminals
ev --no-animation         # static indicators
ev --data-dir /abs/path   # separate, isolated storage
ev --help
```

`ev` preserves Korean, Chinese, Japanese and combining characters, but your terminal chooses the font. For clean CJK monospace text, select a face such as **D2Coding** or **Sarasa Mono K** in your terminal profile.

## Meet the party

In automatic mode (`[Auto]`), Send selects the adopted plan's focus role. The running request can adapt its form once, then stays visually stable until the next Send; changing todo roles still updates progress without repeatedly evolving. The selected behavior profile is frozen for that request, not stacked or replaced on each appearance change. Pin a form with `/forms`; `/evolution-preview` shows the next Send's choice. Forms are reversible and never grant permissions.

| Form | Role | Good for |
| --- | --- | --- |
| **Eevee** | All-rounder: takes a new request and picks the right specialist | starting a task, general questions |
| **Vaporeon** | Explorer: reads the code and finds where things are | "where is…", "how does … work" |
| **Jolteon** | Runner: builds, tests and runs commands, and shows the real output | "run the tests", "build it" |
| **Flareon** | Builder: writes and fixes code | "add …", "fix …", "change …" |
| **Espeon** | Planner: designs the approach and weighs options first | "how should we …", "plan …" |
| **Umbreon** | Detective: finds why something breaks | "why does it crash", "the build is broken" |
| **Leafeon** | Gardener: cleans up and speeds up code without changing behavior | "refactor …", "make … faster" |
| **Glaceon** | Reviewer: checks changes for bugs and risks | "review my changes" |
| **Sylveon** | Writer: docs, summaries and handoff notes | "update the README", "summarize what we did" |

- **Party and cooperation** — up to three partners can work on one task with separate drafts and approvals; only one edits at a time. The lead can also send up to three read-only helpers.
- **Moves and TMs** — save reusable steps with typed parameters, keep up to 32 in `/move-library` (four on quick slots in `/move`), and export or import them as portable TOML. Repeated similar requests can *propose* a move; learning always needs your confirmation, and it saves steps, never permissions.
- **Held context, formations, ribbons and records** — remember files and notes, save party setups and browse source-linked history.

Sprites render via Kitty/Sixel when supported. Installers fetch the artwork separately; source builds can run `ev --download-artwork`.

## Providers

| Connection | How |
| --- | --- |
| **OpenAI account** | Browser or device login inside `ev`; no Codex install needed |
| **OpenAI API** | API key in setup, or `OPENAI_API_KEY` |
| **Anthropic** | API key in setup, or `ANTHROPIC_API_KEY` |
| **Google Gemini** | API key in setup, `GEMINI_API_KEY` or `GOOGLE_API_KEY` |
| **OpenAI-compatible / local** | Endpoint, model, protocol (Chat Completions or Responses) and optional key; HTTPS required except loopback |
| **External agents** | Codex app-server, Claude Agent SDK sidecar, OpenCode / Gemini / Devin via ACP |

The method you choose decides the transport: `ev` never silently falls back to another account, billing route, protocol or model, and exact custom model IDs work even when a provider's model list is unavailable. Credentials are saved by default as owner-private plaintext under `~/.eevee-connections/` (not an OS keychain), so logins survive restarts and updates; choose **Storage → Session only** to keep a credential in memory. Saved account grants still expire and need an explicit reconnect.

On remote and WSL sessions, prefer **Device login** for an OpenAI account. If a browser callback cannot reach the terminal, paste the complete redirect URL into the hidden prompt.

**Claude Agent SDK sidecar (optional, Node 24+).** Install the bridge from a matching checkout with `npm run setup --prefix sidecar` (or `npm ci --prefix sidecar` to reproduce the lockfile), then point `EEVEE_CLAUDE_SIDECAR` at the absolute path of `sidecar/main.ts`. Replace main, policy and dependencies together; outdated or mixed bridges are refused before model submission rather than silently keeping older tool/turn limits. It uses Anthropic API, Console or Bedrock/Vertex credentials; claude.ai subscription logins cannot be reused.

Native HTTP requests the provider provably did not process (connection refused, HTTP 429/503/529) are resent unchanged up to three times with a visible, cancellable wait; `ev --retry off` turns this off. Nothing is resent once a response has started. Details, limits and per-route enforcement: [Agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Safety model

`ev` mediates the native tools it gives the model.

- **Files** — every create/edit/rename/delete is frozen against the exact bytes and paths it will touch, then reviewed; edits show just the changed region. Stale content, links, protected paths and your exclusions are refused.
- **Commands** — shown as readable argv with working directory and resolved executable before they run. Nonzero, cancelled or unknown outcomes are never reported as success, and nothing is replayed automatically.
- **Auto-approve** — an explicit, visible, in-memory grant for the lead partner that ends on `/new`, switching tasks, `/auto-approve-off` or quit. It is never saved. Questions, other partners, helpers and MCP calls still ask.
- **Rules** — created from a request under review; each pins the project, executable and argument prefix (or write paths), is listed in `/rules` and can be revoked instantly. Deletes, renames, shells and unknown effects never match.
- **Ownership** — approvals belong to the request that asked. Evolution, restarts and saved settings never restore a grant.

> [!WARNING]
> Commands run with your user's permissions and environment. `ev` is **not an OS sandbox**: an approved command can touch anything you can. External agents' own tools follow their own policies. Cancellation stops work but does not roll back completed effects.

Full contracts: [Design rules](docs/DESIGN_RULES.md).

## Your data

Data lives in `~/.local/share/eevee` (Linux), `~/Library/Application Support/eevee` (macOS) or `%LOCALAPPDATA%\eevee` (Windows). Keep it outside your project tree.

> [!IMPORTANT]
> Before upgrading, exit every running `ev` and back up the whole data directory. `ev` refuses data it cannot read safely instead of converting it, and leaves the refused bytes unchanged. Until the first stable release there are no storage migrations: a store from an older storage schema is refused, so export its tasks with `ev --export-data PATH` and move the data directory aside to start fresh.

```sh
ev --export-data tasks.json                                                       # readable tasks to a new JSON file
ev snapshot create --data-dir SOURCE --destination NEW --acknowledge-private-data # complete offline backup
ev snapshot verify --source BACKUP
ev snapshot restore --source BACKUP --destination NEW --acknowledge-private-data
```

Snapshots copy **all** app data, including private task text and any saved credentials; they are not encrypted.

## Build from source

Requires the Rust version in [`Cargo.toml`](Cargo.toml) (`rust-version`).

```sh
git clone https://github.com/kimmandoo/eevee.git
cd eevee
cargo build --locked --release
./target/release/ev
```

Run the same checks as CI:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
python -m unittest discover -s scripts -p 'test_*.py'
npm ci --ignore-scripts --no-audit --no-fund --prefix sidecar && npm test --prefix sidecar   # optional Claude SDK bridge
```

Terminal journeys drive the real binary against disposable projects and deterministic local peers — no API keys. They need Python 3.12 with `pyte` and `wcwidth`:

```sh
python scripts/native_workflows_smoke.py --binary target/release/ev --output /tmp/ev-shared --profile shared --jobs 3
```

Every push builds, packages, installs offline and exercises file consent on all five native targets; [expanded protocol regressions](https://github.com/kimmandoo/eevee/actions/workflows/terminal-regression.yml) run weekly or on demand.

## Contributing

Issues and pull requests are welcome.

- **Bugs:** include your OS, terminal and size, `ev --version`, provider route, and what you expected versus saw. Never paste credentials or private task data.
- **Changes:** keep `fmt`, `clippy` and `test` green, add behavior-focused tests, and add an entry to [CHANGELOG.md](CHANGELOG.md). Commit messages follow `type(scope): subject`, e.g. `fix(files): refuse aliased parents`.
- **Contracts:** [docs/DESIGN_RULES.md](docs/DESIGN_RULES.md) is binding. Changes that touch approvals, storage, cancellation or provider boundaries should update it in the same PR.
- **UI text** lives in `messages/` and is compiled into the binary.

| Docs | For |
| --- | --- |
| [Design rules](docs/DESIGN_RULES.md) | Product behavior, privacy and lifecycle contracts |
| [Agent compatibility](docs/AGENT_COMPATIBILITY.md) | Providers, transports and enforcement limits |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Terminal, install and runtime problems |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) · [Roadmap](task.md) | Architecture and upcoming work |
| [Changelog](CHANGELOG.md) | What changed, release by release |

## License and attribution

Original code, prose and the geometric wordmark are licensed under **[Apache-2.0](LICENSE)**. See [NOTICE](NOTICE) and [asset attribution](docs/ASSET_LICENSES.md). The wordmark has an editable [SVG source](docs/assets/wordmark.svg).

Pokémon sprites are third-party artwork from a [pinned PokéAPI revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5); its [notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) attributes image copyright to **The Pokémon Company**. Native packages contain no franchise image bytes. The documentation screenshot shows rendered third-party artwork, excluded from Apache-2.0; see [capture provenance and rights limits](docs/ASSET_LICENSES.md). Display, downloads and the software license grant no image, trademark or commercial-use rights.

**Independent and unofficial; no affiliation or endorsement.**
