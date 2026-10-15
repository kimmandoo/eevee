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

Use a terminal of at least **60 × 18** (120 × 36 recommended).

1. Type a goal in the bottom **Task composer** and press Enter. It is saved locally; nothing is sent.
2. `/connect` → choose a provider → sign in through its official CLI.
3. `/provider`, `/model`, `/effort` → pick provider, model and reasoning level.
4. `/new` → review the exact request and its receipt → approve once.
5. `/run` → `/command-edit` → enter argv such as `["cargo","test"]` → review → approve once.

Type `@` in the composer to attach files, and `/` (or Ctrl+K) to search every available action. Arrows/Tab/Enter/Esc navigate.

```sh
ev --theme light            # cream palette
ev --ascii --color mono     # plain-terminal fallback
ev --data-dir /abs/path     # isolated data directory
ev --help
```

## Commands

| Command | Action |
| --- | --- |
| `/connect` / `/provider` | Manage provider sign-in / select the execution provider |
| `/model` / `/effort` / `/models-refresh` | Local model and reasoning pickers / reviewed catalogue refresh |
| `/new` / `/resume` | Review a fresh request / resume an owned same-task session |
| `/role` / `/forms` | Plan, Implement or Review / choose an evolution |
| `/goal` / `/next` / `/criteria` | Edit task facts and completion criteria |
| `/tasks` / `/task-new` | Browse saved tasks / create one |
| `/files` / `@` / `/range` | Source context / attach files / attach line ranges |
| `/run` / `/command-edit` | Local execution / author argv or an explicit shell object |
| `/diff` / `/history` / `/decision` | Worktree diff / provider journal / save a proposal as next action |
| `/handoff` / `/memory` / `/workflows` | Portable handoff / approved notes / diagnosis, refactor, review and recipe desk |
| `/images` / `/inspect` / `/companion` | Sprites / details / companion |
| `/quit` / Ctrl+C | Safe exit review |

No command bypasses approval or executes pasted text. Model catalogues come from each provider's official interface; opening a picker starts no process or network request.

**Request receipt.** Every provider preview breaks the exact payload Eevee sends into instructions, task facts, attachments, handoff and memories, with exact bytes and a labelled local token estimate (`eevee-local-v1`, CJK-aware). It runs offline. Provider-added context, usage and billing stay **unknown**, not zero.

## Connect an agent

Authentication is only needed for provider requests. Login runs in the provider's official CLI after its own review, and Eevee never collects those tokens; the Devin website OAuth route is the exception — Eevee drives that flow itself and keeps the returned session credential in memory only (injected as `WINDSURF_API_KEY` into `devin acp`, never persisted).

| Provider | Integration | Sign-in |
| --- | --- | --- |
| Codex **0.152.1** | stdio app-server | Browser OAuth or device authorization |
| Claude Agent SDK **0.3.296** | Optional Node 24 sidecar | API key, Console OAuth via `ant` **1.40.0**, Bedrock or Vertex (no claude.ai subscription reuse) |
| OpenCode **1.18.0** | ACP (`acp --pure`) | OpenCode's own `auth login` |
| Gemini CLI **0.28.0** | ACP | Gemini's native CLI (OAuth, API key or Vertex) |
| Devin CLI **3000.11.3** | ACP (local agent) | Website OAuth (browser callback or pasted redirect; in-memory session credential injected as `WINDSURF_API_KEY`) or the CLI's own browser OAuth / manual token |

Other provider versions fail closed. On WSL/SSH, use Codex **device authorization** instead of a localhost callback. The Claude sidecar:

```sh
npm ci --prefix sidecar --ignore-scripts --no-audit --no-fund
export EEVEE_CLAUDE_SIDECAR="$PWD/sidecar/main.ts"
```

Credential routes, enforcement boundaries and per-provider limits: [Agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Data and trust

- Tasks, drafts, excerpts and journals live in local SQLite (`~/.local/share/eevee`, `~/Library/Application Support/eevee`, or `%LOCALAPPDATA%\eevee`). Schema upgrades are transactional; newer or corrupt databases are never silently reset.
- Recognizable credentials are filtered before storage, but masking is incomplete; review exports.
- Provider configuration, hooks, MCP and resumed history can add effects beyond the displayed payload. Provider read-only policy is not an Eevee sandbox.
- Restart never replays commands or model turns; unresolved outcomes stay visible.

Sprites are nine original PokeAPI images that installers download and hash-verify into your cache (`ev --download-artwork` for source builds). Images render with Sixel/Kitty when the terminal supports them, otherwise as text. Limits and recovery: [Troubleshooting](docs/TROUBLESHOOTING.md).

## Installation

| Machine | Package |
| --- | --- |
| macOS Apple Silicon | [ev-darwin-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.3/ev-darwin-arm64.tar.gz) |
| macOS Intel | [ev-darwin-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.3/ev-darwin-x86_64.tar.gz) |
| Linux x86_64 | [ev-linux-x86_64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.3/ev-linux-x86_64.tar.gz) |
| Linux arm64 | [ev-linux-arm64.tar.gz](https://github.com/kimmandoo/eevee/releases/download/v0.1.3/ev-linux-arm64.tar.gz) |
| Windows x86_64 | [ev-windows-x86_64.zip](https://github.com/kimmandoo/eevee/releases/download/v0.1.3/ev-windows-x86_64.zip) |

Installers verify SHA-256, refuse to overwrite commands they don't own, and never edit your shell profile or persistent PATH — the last step on each platform is yours.

### macOS

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.3/install.sh
sh ./install-ev.sh --version v0.1.3          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && exec zsh
```

Builds are not notarized. The script installs via `curl`, which sets no quarantine flag; if you unpacked the archive from a browser download and macOS blocks `ev`, run `xattr -d com.apple.quarantine ~/.local/bin/ev`.

### Linux and WSL

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.3/install.sh
sh ./install-ev.sh --version v0.1.3          # installs to ~/.local/bin/ev
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc && exec bash
```

Inside WSL, use this Linux build, not the Windows one. For provider sign-in on WSL, prefer Codex **device authorization**.

### Windows

Run in **PowerShell** (5.1 or 7, no admin). Windows blocks downloaded `.ps1` files by default, so run the installer as a script block instead of `.\install.ps1`:

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/download/v0.1.3/install.ps1))) -Version v0.1.3
```

It installs `%LOCALAPPDATA%\ev\bin\ev.exe`. Use it right away, and keep it on PATH for new terminals:

```powershell
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
[Environment]::SetEnvironmentVariable('Path', "$env:LOCALAPPDATA\ev\bin;" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')
```

Prefer a saved script? Run it with a process-only bypass: `powershell -NoProfile -ExecutionPolicy Bypass -File .\install-ev.ps1 -Version v0.1.3`. Windows Terminal shows the sprites (Sixel); other consoles fall back to text.

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
