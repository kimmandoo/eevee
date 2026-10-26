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
[![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](Cargo.toml)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)

[Install](#install) · [Quick start](#quick-start) · [Providers](#providers) · [Safety model](#safety-model) · [Build from source](#build-from-source) · [Contributing](#contributing)

</div>

---

<p align="center">
  <img src="docs/assets/screenshot.png" width="900" alt="ev at 120×36: the agent created snake.py, ran its self-test under auto-approve, and reported the result; the partner panel shows an Eevee evolving into Jolteon">
  <br>
  <sub>The real <code>ev</code> binary at 120×36 with <code>/auto-approve</code> on, driven by a scripted local model: it writes <code>snake.py</code>, runs its self-test and reports back.</sub>
</p>

`ev` is a single native binary that turns any folder into an agent workspace. Type a request; the model you connected reads your project, writes files, runs builds and tests, reads the real errors and fixes them — the same loop you know from Codex or Claude Code, with any provider and with every effect shown to you first.

It also happens to come with a companion: a party of Eevee forms that evolve with the kind of work you do. Purely cosmetic — every form can code.

## Highlights

- **Any model.** OpenAI (account or API), Anthropic, Google Gemini, and any OpenAI-compatible endpoint — including local servers like Ollama, vLLM and LM Studio. External agents (Codex, Claude Agent SDK, OpenCode / Gemini / Devin over ACP) work too.
- **A real coding loop.** Discover, read, create, edit, rename and delete files; run builds, tests and programs; feed actual output back to the model. Generated code streams into the transcript as it is written.
- **Review, or auto-approve.** Each file change and command opens a compact review with the exact diff or argv. When you trust the run, `/auto-approve` (or **Shift+A** in a review) lets the rest of the task proceed without prompts.
- **Local-first.** Tasks, drafts, transcripts and run history live in local SQLite. No account, server or Node runtime needed — the workspace works offline until you connect a model.
- **Built for the terminal.** Rust + ratatui, works from 60×18 up, mouse and keyboard, Kitty/Sixel sprites with text fallback, ASCII/mono modes.
- **Cross-platform.** macOS (arm64, x86_64), Linux and WSL (x86_64, arm64), Windows (x86_64). Every target is built and tested in CI.

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

Installers verify SHA-256 checksums and never edit your shell profile or persistent `PATH`. Prebuilt archives and checksums are on the [releases page](https://github.com/kimmandoo/eevee/releases/latest); see [offline and pinned installs](#offline-and-pinned-installs) below.

```sh
ev update --check   # see whether a newer release exists
ev update           # upgrade in place
ev uninstall        # remove the binary; your data stays
```

> [!IMPORTANT]
> Before upgrading, exit every running `ev` and back up the **whole data directory**. Data formats can change between versions even when the schema number does not; `ev` refuses data it cannot read safely rather than converting it.

### Offline and pinned installs

<details>
<summary>Pinned versions, offline archives and persistent PATH</summary>

Pin a version:

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/latest/download/install.sh
sh ./install-ev.sh --version v0.1.32
```

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/latest/download/install.ps1))) -Version v0.1.32
```

Offline: download the matching archive, `SHA256SUMS` and installer from one release, then:

```sh
sh ./install-ev.sh --archive ./ev-linux-x86_64.tar.gz --checksums ./SHA256SUMS
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Archive "$PWD\ev-windows-x86_64.zip" -Checksums "$PWD\SHA256SUMS"
```

To keep `ev` on `PATH`, add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile, or on Windows add `%LOCALAPPDATA%\ev\bin` to your user `Path`. macOS builds are not notarized; WSL uses the Linux packages.

</details>

## Quick start

```sh
cd your-project
ev
```

1. **Connect** — `/connect`, pick a provider, pick a model. Setup never sends your draft.
2. **Ask** — type what you want and press **Enter**:
   > Create a snake game in Python, run it, and fix anything that breaks.
3. **Review** — file changes and commands appear for review automatically. Allow once, deny, or press **Shift+A** to allow the rest of this task.

The folder you launch from is the project boundary. Git is optional. No `/init` is required; `/init` only drafts an `AGENTS.md` guide for the project if you want one.

### Everyday controls

| Want to… | Use |
| --- | --- |
| Find any action | `/` or **Ctrl+K** |
| Connect / switch model / reasoning effort | `/connect` · `/model` · `/effort` |
| Attach a file or line range as context | `@` → pick → **Enter** (**r** for a range) |
| Auto-approve for this task / stop | `/auto-approve` · `/auto-approve-off` |
| Reopen a deferred review | **Alt+A** or `/approvals` |
| Read earlier output | wheel or **PgUp/PgDn**; **Alt+End** follows live output |
| Run your own command | `/command-edit` → `/execute`; history in `/run` |
| Restrict which files may be written | `/paths` (`[]` clears it) |
| Plan without writing | `/role-plan`, or say "read only" |
| Start a fresh chat / new task | `/chat-new` · `/reset` · `/tasks` |
| Cancel / quit safely | `/cancel` · `/cancel-all` · `/quit` |

```sh
ev --theme light          # light palette
ev --ascii --color mono   # plain terminals
ev --no-animation         # static indicators
ev --data-dir /abs/path   # separate, isolated storage
ev --help
```

## Providers

| Connection | How |
| --- | --- |
| **OpenAI account** | Browser or device login inside `ev`; no Codex install needed |
| **OpenAI API** | API key in setup, or `OPENAI_API_KEY` |
| **Anthropic** | API key in setup, or `ANTHROPIC_API_KEY` |
| **Google Gemini** | API key in setup, `GEMINI_API_KEY` or `GOOGLE_API_KEY` |
| **OpenAI-compatible / local** | Endpoint, model, protocol (Chat Completions or Responses), optional key. HTTPS required except loopback |
| **External agents** | Codex app-server, Claude Agent SDK sidecar (Node 24+), OpenCode / Gemini / Devin via ACP |

The method you choose decides the transport — `ev` never silently falls back to another account, billing route, protocol or model. Exact custom model IDs work even when the provider's model list is unavailable. Credentials are session-only by default; opting in stores them as owner-private plaintext under `~/.eevee-connections/` (not an OS keychain).

Details, limits and per-route enforcement: [Agent compatibility](docs/AGENT_COMPATIBILITY.md).

## Safety model

`ev` mediates the native tools it gives the model. In short:

- **Files** — every create/edit/rename/delete is frozen against the exact bytes and paths it will touch, then reviewed. Edits of existing files show just the changed region. Stale content, links, protected paths and your exclusions are refused. Up to 1024 files per request, 4 MiB each.
- **Commands** — shown as a readable argv with working directory and resolved executable before they run. Nonzero, cancelled or unknown outcomes are never reported as success, and nothing is replayed automatically.
- **Auto-approve** — an explicit, visible (`AUTO-APPROVE` in the header), in-memory grant for the current partner and task only. Questions still ask; task change or restart ends it.
- **Ownership** — approvals belong to the request that asked. Evolution, restart and saved settings never restore a grant.

> [!WARNING]
> Commands run with your user's permissions and inherited environment. `ev` is **not an OS sandbox**: an approved command can touch anything you can. External agents' own tools follow their own policies. Cancellation stops work but does not roll back completed effects.

Your data lives in `~/.local/share/eevee` (Linux), `~/Library/Application Support/eevee` (macOS) or `%LOCALAPPDATA%\eevee` (Windows). Keep it outside your project tree. Full contracts: [Design rules](docs/DESIGN_RULES.md).

## The party

Your companion is more than decoration — but never a permission system.

- **Nine forms** chosen automatically from what you ask (or pinned manually), with a small evolution moment when the specialty changes.
- **Party and cooperation** — up to three partners can work on one task with separate drafts and approvals; only one may edit at a time.
- **Moves and TMs** — save reusable steps per form, export/import them as portable TOML.
- **Held context, formations, ribbons and records** — remember files and notes, save party setups, and browse source-linked history.

Sprites render via Kitty/Sixel when supported; installers fetch artwork separately and source builds can run `ev --download-artwork`.

## Build from source

Requires **Rust 1.90**.

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
npm ci --ignore-scripts --no-audit --no-fund --prefix sidecar && npm test --prefix sidecar   # optional Claude SDK bridge
```

End-to-end terminal journeys live in `scripts/*_smoke.py` (needs `pyte==0.8.2` and `wcwidth==0.2.13`). They drive the real binary in a PTY against deterministic local protocol peers, so they need no API keys:

```sh
python scripts/native_workflows_smoke.py --binary target/release/ev --output /tmp/ev-proof --profile full --jobs 2
```


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

Pokémon sprites are third-party artwork from a [pinned PokéAPI revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5); its [notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) attributes image copyright to **The Pokémon Company**. No franchise image bytes are bundled in this repository or native packages. Remote display/downloads and the software license grant no image, trademark or commercial-use rights.

**Independent and unofficial; no affiliation or endorsement.**
