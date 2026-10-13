<div align="center">

<img src="https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/133.png" width="96" height="96" alt="Eevee pixel sprite">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.png">
    <img src="docs/assets/wordmark-light.png" width="332" height="68" alt="eevee">
  </picture>
</h1>

**A local-first terminal workbench for LLM-assisted development.**

One persistent partner. Nine task-specific forms. Your work stays local until you choose otherwise.

<sub>BUILT WITH RUST &nbsp; / &nbsp; KEYBOARD-FIRST &nbsp; / &nbsp; APACHE-2.0</sub>

[Quick start](#quick-start) · [Workflow](#workflow) · [Agents](#connect-an-agent) · [Documentation](#documentation) · [Development](#development)

</div>

---

Eevee brings durable tasks, repository context, reviewed commands, and optional coding agents into a native Rust TUI. The executable is **`ev`**. You can use the local workbench without an account, API key, Node runtime, or model connection.

> **Project status:** Active development. Local workflows and scoped agent integrations are implemented; authenticated interoperability and current native-platform parity remain separate acceptance gates. Public releases are withheld. See the [backlog](task.md) and [verification checkpoint](docs/WORK_CHECKPOINT.md) for exact coverage.

## Why Eevee

- **Keep the task, not just the chat.** Save goals, acceptance criteria, next actions, and unapplied drafts. Resume them after restarting.
- **Know what changed.** Inspect repository baselines and raw diffs; pre-existing edits remain distinct from later, unattributed changes.
- **Choose the context.** Capture specific source ranges and review the outgoing payload before a model request.
- **Review before running.** Local commands, provider requests, edits, and handoffs have separate, explicit approval steps.
- **Keep the evidence.** Retain bounded command output and provider-origin journals without treating a plan as verified execution.
- **Switch the workflow, not your identity.** Nine forms organize task roles. Forms do not lock you to a particular provider; portable handoffs carry facts, not sessions or permissions.

## Quick start

Build the current source with **Rust 1.90 or later**:

```sh
git clone https://github.com/kimmandoo/eevee.git
cd eevee
cargo build --locked --release
./target/release/ev --images off
```

On Windows, run `.\target\release\ev.exe --images off` after building.

Use an interactive terminal of at least **60 × 18** cells; **120 × 36** leaves room for all three panes. Authentication review requires **80 × 24**. No artwork or provider is needed to start.

1. Press **`g`** to write a goal; Enter applies it. Wait for **Storage: saved**.
2. Press **`c`**, then **`e`**, to add acceptance criteria.
3. Press **`v`** to inspect and capture source context, or **`j`** to review a local command.
4. Connect an agent with **`o`** when you want one. Open **`e`** to separately review a model request.

```sh
./target/release/ev --theme light
./target/release/ev --layout linear --images off
./target/release/ev --ascii --color mono
./target/release/ev --data-dir /absolute/path/to/isolated-data
./target/release/ev --help
```

Startup, form selection, context previews, and manual handoffs do **not** submit model requests.

## Workflow

### Capture → review → act → verify

| Surface | What it does | Boundary |
| --- | --- | --- |
| Tasks | Goals, criteria, drafts, next actions, archive and resume | Completion records your review, not a passing test |
| Repository | Initial baseline and current raw worktree diff | External changes are unattributed; no automatic reset or commit |
| Context | Selected files/ranges and exact payload preview | Source identity is rechecked before dispatch |
| Local commands | Reviewed argv or explicit shell, output and run history | Owned process cleanup, not a filesystem/network sandbox |
| Agents | Planning, selected-file edits, questions and once-only approvals | Capability and enforcement differ by adapter |
| Handoffs | Reviewed JSON/Markdown export and import | Imported claims remain unverified; no credentials or grants |

For a local command, open `j` → `e` and enter a JSON argv array, such as `["cargo","test"]`. Saving prepares a review; it does not execute. Open `v` inside that drawer to review the exact command and effects, then explicitly authorize it. Shell syntax requires an explicit shell object, not an implicitly parsed string.

For an agent request, open `e` → `n` for a new-session preview, or `r` to review resuming an eligible same-task session. Read to the end with PgDn/End, then Tab → Enter to authorize once. Questions and tool permissions have their own review. Cancellation invalidates pending answers; an unconfirmed outcome stays **unknown**.

In the agent drawer, `f` selects Plan/Implement and `x` selects existing files. Codex's controlled POSIX write route accepts **1–8 regular UTF-8 files, at most 12 KiB each**. Every replacement requires review of the exact before/after patch. Protected, unselected, linked, or changed-baseline paths are refused; native edit/command escalation is denied. Other platforms refuse this controlled route. Inspect `w` afterward and deliberately run a separate local check with `j`.

Select a completed, visible decision and press `d` to open it in the next-action editor; Enter saves it. Decisions over 4096 UTF-8 bytes are refused rather than silently truncated. `h` in the agent drawer shows provider-origin journals, separately from local command evidence.

## Connect an agent

Authentication is only needed for the provider operation you choose—not for local tasks, source browsing, or local commands. Subscription access, API billing, and cloud credentials are distinct routes.

Open **`o`** to discover installed profiles without launching them:

| Key inside `o` | Action |
| --- | --- |
| `1`–`4` / arrows | Select a profile |
| Enter | Review an explicit, bounded `--version` probe |
| `m` | Select a supported login method |
| `l` | Review starting the official login flow |
| `s` | Review the official status/inventory command, where supported |
| `r` | Refresh passive discovery |

All reviews start on **Stay**. Read to the end, then Tab → Enter to authorize. Login is not an implicit side effect of discovery or a failed model request.

| Provider | Integration | Supported connection route |
| --- | --- | --- |
| Codex **0.152.1** | Private stdio app-server | Official browser OAuth or device authorization; official login status |
| Claude Agent SDK **0.3.296** | Optional Node sidecar; embedded CLI **2.1.296** | Anthropic API key, Amazon Bedrock, or Google Vertex AI; **no claude.ai subscription OAuth reuse** |
| OpenCode **1.18.0** | ACP1 via `acp --pure` | Official `auth login --pure` provider/method chooser; available OAuth/API-key methods belong to OpenCode |
| Gemini CLI **0.28.0** | ACP1 for model requests | Official native CLI chooser for Google OAuth (browser/manual code), API key, or Vertex |

Other provider versions fail closed. Installed, authenticated, eligible, and successfully tested are separate states. OpenCode's credential inventory or a login process exiting successfully does not by itself prove authentication. Gemini 0.28.0 mixes interactive OAuth prompts into its ACP output, so authentication deliberately uses its **official native CLI**, not a custom parser or token collector. Choose the method there (`/auth` if already connected), then exit back to Eevee. This is an explicitly external workflow: the CLI can initialize provider services and continue into chat; anything you type there is handled by Gemini and can start model work. Eevee sends no task or prompt automatically, and native exit leaves authentication **unknown** until separately checked.

The reviewed operation temporarily hands the terminal to the official provider. It may open a browser, contact its service, and modify its own credential/configuration stores. Native auth output is not retained in Eevee's task logs, but can remain in your terminal scrollback. Approval expires after five minutes; login is bounded to ten minutes and status to fifteen seconds. Ctrl+C cancels the native flow and returns to the local task. Authentication does not replay a task: prior Run consent and cached model sessions are invalidated, and a new explicit request is required.

### Optional Claude SDK

The local workbench and other adapters do not need Node. The sidecar requires **Node 24+**:

```sh
npm ci --prefix sidecar --ignore-scripts --no-audit --no-fund
export EEVEE_CLAUDE_SIDECAR="$PWD/sidecar/main.ts"
node sidecar/main.ts --inspect
```

Use a permitted `ANTHROPIC_API_KEY`, or explicitly select Bedrock/Vertex. For provider-managed AWS SSO/profile/workload or Google ADC/workload chains, opt in with `EEVEE_CLAUDE_CLOUD_AUTH=provider-chain` **and** the selected `CLAUDE_CODE_USE_BEDROCK`/`CLAUDE_CODE_USE_VERTEX` route. Review the [credential and cloud-policy contract](docs/AGENT_COMPATIBILITY.md#claude-agent-sdk): these chains can read/refresh caches, execute credential helpers, and contact identity or metadata endpoints. Eevee isolates Claude SDK configuration and verifies the effective backend; it never falls back to claude.ai subscription tokens.

SDK inspection imports the real SDK without a model request; it is not authenticated-query proof. Selected-path SDK Write/Edit hooks are **agent policy, not atomic OS confinement**. ACP provider-owned tools are also external policy; controlled ACP implementation is refused. [Compatibility details →](docs/AGENT_COMPATIBILITY.md)

## Keyboard reference

| Key | Action |
| --- | --- |
| `g` / `a` | Edit goal / next action |
| `c` | Open criteria; `e` edits, Space toggles |
| `t` / `n` | Browse task cards / create a task |
| `f` | Choose a task-specific form |
| `w` / `v` | Inspect repository / source context |
| `j` | Local command review, output and history |
| `o` / `e` | Agent connection / model-request drawer |
| `z` | Reviewed portable handoff export/import |
| `i` / `b` | Inspector / companion drawer |
| `l` | Toggle sanctuary / linear layout |
| Ctrl+K / `?` | Command palette / keyboard help |
| Ctrl+S in editor | Retain a draft without applying it |
| Ctrl+N in editor | Insert a newline |
| Esc | Back, cancel review, or review discarding a changed draft |
| `q` / Ctrl+C | Request safe exit; pending saves and active work are not bypassed |

Keys are contextual; the active drawer's footer is authoritative. Enter never turns an untouched neutral review into approval. Small terminals can still cancel and exit. Cancellation is not rollback.

## Data and trust

- **Local storage:** SQLite tasks, drafts, excerpts, and journals under the platform data directory. `--data-dir PATH` isolates an instance. A lock prevents competing writers; schema upgrades are transactional, and corrupt/newer databases are not silently reset.
- **Save state:** only the matching committed revision shows **Storage: saved**. Failed writes preserve drafts and expose explicit retry. Journal retries save observations; they never rerun a provider or command.
- **Privacy:** recognizable credentials and terminal controls are filtered before persistence, but masking is incomplete. Review anything you export. Volatile editor/native-provider output can still contain secrets.
- **Provider boundaries:** configuration, instructions, hooks, MCP, independent reads, and resumed history can add effects beyond the displayed payload. A trust review or provider read-only policy is **not** an Eevee sandbox.
- **Portable handoffs:** version-1 JSON/Markdown carries committed task facts, selected sources/decisions, and unresolved work—not native sessions, permissions, hidden reasoning, or credentials. Import cannot complete a task or create execution evidence.
- **Recovery:** restart does not restore process ownership or replay commands/model turns. Unresolved outcomes remain visible. Deletion/purge requires explicit review and does not imply forensic secure erasure.

<details>
<summary>Storage locations, bounds, and recovery commands</summary>

| Platform | Default data directory |
| --- | --- |
| macOS | `~/Library/Application Support/eevee` |
| Linux | `$XDG_DATA_HOME/eevee`, or `~/.local/share/eevee` |
| Windows | `%LOCALAPPDATA%\eevee` |

Schemas 0–4 migrate to schema 5. Bounds include 256 tasks/project, 64 criteria/task, 4096 UTF-8 bytes per goal/next-action/draft, and 16 source excerpts/task. Source files are bounded to 2 MiB, individual captures to 64 KiB. Credential paths, binary/ignored files, and unsafe links are excluded; `--exclude-context PATH` adds case-insensitive repository-relative exclusions.

`--quota-kib N` controls shared storage (default 65536, minimum 64); `--spool-kib N` bounds each command's output reservation (1–8192, default 64). Quotas do not silently evict evidence. Task storage retries with `r`; context/local journal storage has a separate `s` retry, and provider journals use `r` in their history view.

`ev --data-dir PATH --export-data NEW_FILE.json` exports readable known **task rows only**, without migration or overwrite. It is not an output/excerpt backup or database repair. Keep original files when recovery is refused. [Troubleshooting →](docs/TROUBLESHOOTING.md)

</details>

<details>
<summary>Optional artwork and terminal graphics</summary>

No images are bundled or downloaded on startup/form selection. `--images off` skips image reads. Supply local `133.png`, `134.png`, `135.png`, `136.png`, `196.png`, `197.png`, `470.png`, `471.png`, and `700.png`, or deliberately fetch the pinned source set:

```sh
./target/release/ev --download-artwork --assets-dir ./local-artwork
./target/release/ev --assets-dir ./local-artwork --images cells
```

Downloads require an empty destination or complete matching cache; existing artwork is not silently replaced. `sources.json` records source revision, attribution, and hashes. Static PNGs are bounded to 4 MiB and 1024 × 1024; APNG is refused. Do not commit downloaded artwork.

Native Kitty graphics are qualified only on macOS arm64 / Ghostty 1.3.1 without SSH/tmux/screen, with usable pixel dimensions. Other routes use image-derived cells. Mono/ASCII/NO_COLOR never emits colored native graphics. Linear mode offers the same local actions without art; screen-reader compatibility is not claimed.

</details>

## Installation and platform status

Building source is the way to get the current implementation. Older [verification artifacts](https://github.com/kimmandoo/eevee/actions/runs/38282091481) contain revision **`ea8ade7`**, not the current workbench/provider changes; availability depends on GitHub artifact retention and may require sign-in. Public Releases and remote `curl | sh` installation are intentionally withheld.

<details>
<summary>Install an older verification package</summary>

Download and unzip the outer `ev-*` workflow artifact. Inspect its installer before running it.

| Machine | Inner archive |
| --- | --- |
| macOS Apple Silicon | `ev-darwin-arm64.tar.gz` |
| macOS Intel | `ev-darwin-x86_64.tar.gz` |
| Linux x86_64 | `ev-linux-x86_64.tar.gz` |
| Linux arm64 | `ev-linux-arm64.tar.gz` |
| Windows x86_64 | `ev-windows-x86_64.zip` |

```sh
# macOS/Linux: substitute the matching archive above.
sh ./install.sh --archive ./ev-darwin-arm64.tar.gz --checksums ./SHA256SUMS
export PATH="$HOME/.local/bin:$PATH"
ev --version
```

```powershell
# Windows PowerShell
.\install.ps1 -Archive (Join-Path $PWD 'ev-windows-x86_64.zip') -Checksums (Join-Path $PWD 'SHA256SUMS')
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
ev --version
```

Installation needs no Rust, Node, Python, account, or sudo once downloaded. Defaults are `~/.local/bin` and `%LOCALAPPDATA%\ev\bin`; `EV_INSTALL_DIR` or the installer's directory flag overrides them. No permanent PATH/profile changes are made. Close running `ev` on Windows before updating.

Installers verify SHA-256 before extraction and preserve an owned prior installation on failure; modified/unowned commands are not overwritten. Checksums are not publisher signatures. Packages are unsigned and macOS packages are not notarized—follow your organization's approval policy rather than disabling system security.

To remove this preview installation, confirm the paths and manually remove the installed executable and adjacent `.ev-install` metadata. Task/cache/provider data is retained. There is no `ev uninstall` or implicit data purge.

</details>

The [historical native core run](https://github.com/kimmandoo/eevee/actions/runs/38274013013) qualified five macOS/Linux/Windows OS–architecture–PTY/ConPTY tuples at revision `9d49447`; it does not certify today's provider or persistence changes. Current provider evidence is Linux/WSL x86_64. Windows cross-checking is not native runtime proof, Windows ARM64 is unqualified, and arbitrary GUI terminals are not implied supported. See the [checkpoint](docs/WORK_CHECKPOINT.md) for version-specific results and blockers.

## Documentation

| Document | Purpose |
| --- | --- |
| [Design rules](docs/DESIGN_RULES.md) | Product, visual, privacy, and lifecycle contract |
| [Agent compatibility](docs/AGENT_COMPATIBILITY.md) | Supported protocols, authentication, and control boundaries |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) | Architecture and acceptance gates |
| [Task backlog](task.md) | Completed, active, and blocked work |
| [Work checkpoint](docs/WORK_CHECKPOINT.md) | Observed verification evidence and native-platform limits |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Terminal and operational diagnostics |

[Conceptual layout](docs/TUI_MOCKUP.svg) · [Permission-review design](docs/TUI_REQUEST_MOCKUP.svg) — design illustrations, not runtime screenshots.

## Development

```sh
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release
```

For real terminal smoke checks, install `pyte==0.8.2` and `wcwidth==0.2.13` in a Python virtual environment; native Windows additionally needs `pywinpty==2.0.15`. The separate SDK policy suite requires Node 24.

```sh
python scripts/terminal_smoke.py --binary target/release/ev --output target/native-smoke
python scripts/workbench_smoke.py --binary target/release/ev --output target/workbench-smoke
python scripts/handoff_smoke.py --binary target/release/ev --output target/handoff-smoke
npm ci --ignore-scripts --no-audit --no-fund --prefix sidecar
npm test --prefix sidecar
```

The [CI workflow](.github/workflows/native.yml) runs on pull requests, pushes to `main`, and manual dispatch. Five native macOS/Linux/Windows rows retain Rust 1.90 formatting, tests, Clippy, release builds, dependency notices, installation/upgrade/refusal checks and installed-binary PTY/ConPTY workbench, repository, context, execution, recovery and handoff smoke. A separate Node job checks SDK policy without provider credentials. Terminal smoke belongs in CI because unit tests cannot establish real terminal restoration or installed-binary behavior; automated smoke does not log into providers or send model requests.

Actions are pinned to immutable revisions, checkout credentials are not persisted, and Rust dependency/tool caches are saved only from `main`. Packages remain verification artifacts, not public Releases. A passing row covers its observed OS/architecture/terminal host—not arbitrary desktop terminals or live provider eligibility.

Repository, context, execution, recovery, authentication, planning, implementation, and provider journeys have dedicated scripts under `scripts/`. Live provider checks are **opt-in** (`--live`), require the pinned runtimes, and may contact providers or incur usage. OAuth initiation/cancellation is not proof of completed human authentication; SDK inspection is not live cloud/model acceptance. The detailed checkpoint records those distinctions.

Contributions should preserve the existing approval, storage, and cancellation contracts. Include a focused regression for a behavioral bug, exercise the changed path in the real TUI, and document the observed result and any untested platform boundary. Do not commit credentials, private task data, downloaded artwork, or generated smoke captures.

## License and attribution

Original code, documentation prose, and the custom geometric wordmark are licensed under [Apache-2.0](LICENSE). See the [project NOTICE](NOTICE) and [asset licensing and attribution](docs/ASSET_LICENSES.md) for the separate third-party scope.

The README's Eevee pixel sprite is third-party Pokémon artwork, referenced from a [pinned PokéAPI/sprites revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5). Its [upstream notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) explicitly attributes image copyright to **The Pokémon Company**. The project's Apache-2.0 license does not cover that artwork or grant trademark, redistribution, or commercial-use rights.

No franchise image bytes are bundled in the repository or native packages. Remote display and optional downloads do not establish rights clearance. This is an independent, unofficial project with no affiliation or endorsement; making the repository public does not waive the existing rights/provider-policy release gates. The original wordmark has an editable [SVG source](docs/assets/wordmark.svg), not an embedded character image or third-party font.
