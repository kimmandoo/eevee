# eevee

A local-first Rust terminal workbench for LLM-assisted development: one persistent partner, nine task-specific forms.

## Install a verification binary

The executable is **`ev`**; the project and existing `eevee` cache locations keep their names. Code is licensed under [Apache-2.0](LICENSE). No franchise image bytes are packaged, and the code license grants no image/trademark rights.

Public Releases and remote `curl | sh` bootstrapping are deliberately **withheld** pending the existing rights/release gates. Download the native `ev-*` artifact from the [verified installation run](https://github.com/kimmandoo/eevee/actions/runs/38282091481), then unzip that outer artifact. Those packages contain source `ea8ade7`; later workbench changes are not implied. GitHub may require sign-in for CI artifact downloads; running or installing the downloaded package requires no account, Rust, Python, Node, or sudo.

| Machine | CI artifact / inner archive |
| --- | --- |
| macOS Apple Silicon | `ev-darwin-arm64` / `ev-darwin-arm64.tar.gz` |
| macOS Intel | `ev-darwin-x86_64` / `ev-darwin-x86_64.tar.gz` |
| Linux x86_64 | `ev-linux-x86_64` / `ev-linux-x86_64.tar.gz` |
| Linux arm64 | `ev-linux-arm64` / `ev-linux-arm64.tar.gz` |
| Windows x86_64 | `ev-windows-x86_64` / `ev-windows-x86_64.zip` |

The artifact contains its archive, `SHA256SUMS`, `install.sh`, and `install.ps1`. Inspect the appropriate installer before running it.

macOS/Linux, from the extracted artifact directory:

```sh
sh ./install.sh --archive ./ev-darwin-arm64.tar.gz --checksums ./SHA256SUMS
export PATH="$HOME/.local/bin:$PATH"
ev --version
ev
```

Choose the archive matching your machine from the table. Default destination is `~/.local/bin`; `--install-dir /absolute/path` or `EV_INSTALL_DIR` overrides it. The installer rejects symlinked destination components rather than silently redirecting installation.

Windows PowerShell, from the extracted artifact directory:

```powershell
.\install.ps1 -Archive (Join-Path $PWD 'ev-windows-x86_64.zip') -Checksums (Join-Path $PWD 'SHA256SUMS')
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
ev --version
ev
```

Default Windows destination is `%LOCALAPPDATA%\ev\bin`; `-InstallDir` or `EV_INSTALL_DIR` accepts an absolute destination. Windows ARM64/emulation is not qualified. Windows builds use a static C runtime. These packages/installers are unsigned, and macOS packages are not notarized; OS trust/execution policy may block them. Review the files and follow your organization's approval policy; do not disable system security or execution policy wholesale.

Installers verify SHA-256 **before** extraction, accept only the executable plus `LICENSE`/`THIRD_PARTY_NOTICES`, and bound archive/file sizes. Hashes detect corruption but are not publisher signatures: obtain both archive and sums from the same trusted workflow run. Existing unowned or modified `ev` commands are never overwritten. Owned updates stage the new executable and license metadata and preserve the previous installation on failures; rerun the installer with another verified package to upgrade. Close running `ev` on Windows before updating.

No shell profile or permanent PATH is changed. Metadata is installed in `.ev-install` beside the executable. No task/cache/provider data is removed. To remove this preview installation, manually remove only your installed `ev`/`ev.exe` and its `.ev-install` metadata after confirming the paths; no purge or `ev uninstall` command is implemented.

## Run

```sh
cargo run --release
cargo run --release -- --theme light
cargo run --release -- --color 256
cargo run --release -- --ascii --color mono
cargo run --release -- --layout linear --images off
```

Requires interactive stdin and stdout. The minimum Rust toolchain is 1.90.0. All five qualified native core baselines below passed build/test/lint and real PTY/ConPTY interaction. Native images were separately exercised on macOS arm64/Ghostty 1.3.1. Use `cargo run -- --help` for CLI options.

The sanctuary, nine-species artwork, grid/list picker, local handoff preview, bounded scrolling, grapheme-aware goal editor, and one durable goal/form per project are implemented. Agent execution, repository inspection, multiple task cards, and permission grants remain unavailable. Switching forms never starts tools or changes permissions.

Goals are limited to 4096 UTF-8 bytes and never sent to a model. Enter applies an edit and requests a local save; the header distinguishes saving, committed, and failed revisions. Ctrl+N inserts a newline. Shift+Enter works only when the terminal reports it distinctly. Esc on a changed draft asks whether to keep or discard it. Backspace/VT Ctrl+H and Delete remove complete graphemes; a truncated paste leaves a visible warning.

### Artwork and terminal graphics

No image bytes are bundled and no download occurs on startup or form selection. Supply a local directory containing `133.png`, `134.png`, `135.png`, `136.png`, `196.png`, `197.png`, `470.png`, `471.png`, and `700.png`, or deliberately download the pinned PokeAPI set:

```sh
cargo run --release -- --download-artwork --assets-dir ./local-artwork
cargo run --release -- --assets-dir ./local-artwork --images cells
```

Downloads require an empty directory or an already-complete matching cache; existing user artwork is never silently replaced. `sources.json` records the pinned revision, URLs, attribution, and SHA-256 hashes. Unmanifested local PNGs are labelled source-unverified. Static PNG limits: 4 MiB/file, 1024x1024 dimensions, 16 MiB decoder allocation; APNG is rejected. The cell cache is bounded to 36 entries and 131,072 cells.

Default artwork cache: macOS `~/Library/Caches/eevee/artwork`, Linux `$XDG_CACHE_HOME/eevee/artwork` or `~/.cache/eevee/artwork`, Windows `%LOCALAPPDATA%\\eevee\\artwork`. `--assets-dir` overrides it. `--images auto` uses the verified native Kitty route only on macOS arm64 with Ghostty 1.3.1, usable kernel-reported pixel dimensions, and no SSH/tmux/screen layer; every other route uses image-derived cells. Sixel, iTerm2, other terminal versions, and other native OS/architecture tuples are not enabled without their own real-terminal acceptance. `--images cells` bypasses native discovery; `--images off` skips all image reads. NO_COLOR/mono/ASCII never emits colored native graphics.

Artwork rights remain with the original rights holders. Downloading or attributing images is not a distribution license; do not commit the local image directory or imply affiliation.

## Keyboard

| Key | Action |
| --- | --- |
| `f` | Open the evolution picker |
| `1`–`9`, arrows | Preview a form inside the grid/list; Enter selects, Esc keeps the current form |
| `g` | Edit the durable project goal; Enter applies and saves |
| `i` / `b` | Open the inspector / companion drawer |
| `r` | Retry failed storage outside text entry |
| `p` | Inspect a local handoff preview; in the picker, inspect the previewed role |
| `l` | Toggle sanctuary / linear text layout |
| Ctrl+K | Open the command palette |
| Tab / Shift+Tab | Cycle visible panes |
| Enter | Open the focused pane action or confirm a selection |
| Esc | Close the current overlay, or offer keep/discard for a changed draft |
| `?` | Show keyboard help |
| `q` | Request quit outside text entry; type a literal `q` inside the editor |
| Ctrl+C | Request safe exit; wait for pending saves or explicitly review unsaved data |

Destructive dialogs start on Keep/Stay. Tab/Right explicitly selects discard, then Enter confirms; repeated Enter cannot turn neutral inspection into approval. No active jobs exist yet.

Windows uses a single console-record reader with VT input enabled: bracketed paste stays literal, Alt-code release text is distinguished from ordinary key releases, and UTF-16 surrogate pairs are joined. The terminal guard restores the exact original input-mode bits on normal/error/panic exit. POSIX retains Crossterm's input reader.

The layout uses three panes at 120+ columns, two at 90–119, and one with drawers at 60–89. Below 60 columns or 18 rows, resize guidance preserves the draft; back and safe exit remain available. Local focus, selections, and scroll offsets survive overlays and resize. The picker uses a 3x3 grid at >=90x30, otherwise a selected-centered list and portrait; `p` exposes role/artifact details at compact sizes. Rendering is event-driven with no idle animation or timer. Linear mode exposes the same local actions without requiring artwork; screen-reader compatibility is not claimed without real assistive-terminal evidence.

Auto color uses advertised true-color support, then 256-color support, otherwise monochrome. Nonempty `NO_COLOR` overrides color flags. `TERM=dumb` also selects ASCII; a non-UTF-8 locale selects ASCII borders. Dark and light themes are explicit choices.

## Durable local goal

`g` edits the current project's goal; Enter applies it and requests a local SQLite save. Only a committed revision shows **Storage: saved**. Selecting a form saves it with the goal. Restarting in the same canonical project directory restores both; another project has separate state.

| Platform | Default data directory |
| --- | --- |
| macOS | `~/Library/Application Support/eevee` |
| Linux | `$XDG_DATA_HOME/eevee`, or `~/.local/share/eevee` |
| Windows | `%LOCALAPPDATA%\\eevee` |

Use `ev --data-dir PATH` for isolation. `workbench.sqlite3` stores schema-versioned goals; `workbench.lock` admits one writer across directory aliases. Another instance shows a storage error rather than taking over. Press `r` to retry after resolving the conflict or disk/permission error; `i` shows details. Goal text stays editable on failure. Applied changes finish saving before a confirmed normal exit; failed storage requires an explicit informed exit. An unapplied editor draft is not yet durable.

Schema 0 (`goals(project TEXT PRIMARY KEY, goal TEXT NOT NULL, form INTEGER NOT NULL)`) migrates transactionally to schema 1 (`projects`, including revision). Corrupt, foreign, or newer schemas are never reset. Interrupted single-database rollback migrations are first recovered and validated on private copies, bounded to 64 MiB per database/journal; WAL/SHM and multi-database recovery are refused rather than guessed. Preserve the original files for manual recovery if automatic recovery is unavailable.

`ev --data-dir PATH --export-data NEW_FILE.json` exports readable known goal rows without creating/migrating the database or overwriting the output. Export is not a repair or a guarantee that a corrupt database is readable. Project/data paths must be Unicode; display escaping does not change stored project identity.

## Development checks

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Run `python scripts/terminal_smoke.py --binary target/release/ev --output target/native-smoke` after building. Install `pyte==0.8.2` and `wcwidth==0.2.13`; Windows also needs `pywinpty==2.0.15`. The harness launches the real binary in POSIX PTYs or native Windows ConPTY, records screen/host/console evidence, and builds one isolated lifecycle probe for normal/error/panic restoration. CI also generates dependency license texts with pinned cargo-about 0.9.2, packages `ev`, and exercises fresh install, real-source upgrade, refusal/rollback, and the installed binary through the same terminal harness. It does not certify a GUI terminal or native images; see the checkpoint for observed package outcomes.

### Verified native core baselines

[Native run 38274013013](https://github.com/kimmandoo/eevee/actions/runs/38274013013) tested revision `9d49447` with Rust 1.90.0, Python 3.12.9, pyte 0.8.2, and wcwidth 0.2.13. These are the oldest **observed** baselines, not proof that earlier versions work:

| OS / architecture | Observed OS and terminal transport | Real smoke scenarios |
| --- | --- | --- |
| macOS arm64 | macOS 14.8.9, Darwin 23.6.0 POSIX PTY | 34 passed |
| macOS x86_64 | macOS 15.7.9, Darwin 24.6.0 POSIX PTY | 34 passed |
| Linux x86_64 | Ubuntu 22.04.5, kernel 6.8.0-1064-azure POSIX PTY | 34 passed |
| Linux arm64 | Ubuntu 22.04.5, kernel 6.8.0-1064-azure POSIX PTY | 34 passed |
| Windows x86_64 | Server 2022 build 10.0.20348, native ConPTY via pywinpty 2.0.15/backend 0 | 34 passed |

Each row passed locked tests, warning-free Clippy, release build, seven display modes/four sizes, literal CJK/combining/astral paste, resize/back/selection, safe exit, non-TTY CLI checks, and normal/error/panic restoration. Windows checked actual primary-screen contents and original console-mode bits rather than requiring a VT marker ConPTY did not relay. Its legacy console path composes displayed glyphs and freshly repaints state-changed frames; idle has no timer. This is a correctness choice, not a performance benchmark.

These historical core rows do not certify arbitrary GUI terminals, native images, providers, or filesystem/process parity. The separate [installation run 38282091481](https://github.com/kimmandoo/eevee/actions/runs/38282091481), source `ea8ade7`, passed all five native package rows: each recorded 14 installation-suite cases (13 installer cases plus its terminal suite) and 34 installed-binary PTY/ConPTY scenarios. Its verified `ev-*` artifacts are downloadable from that run. Native-image qualification remains limited to macOS arm64/Ghostty 1.3.1; providers and full P03 acceptance remain unavailable.


## Design

- [Product and visual contract](docs/DESIGN_RULES.md)
- [English implementation plan](docs/IMPLEMENTATION_PLAN.md)
- [Agent compatibility, control boundaries, and handoff contract](docs/AGENT_COMPATIBILITY.md)
- [Conceptual TUI mockup](docs/TUI_MOCKUP.svg)
- [Planned permission-review interaction](docs/TUI_REQUEST_MOCKUP.svg)
- [Actionable backlog and per-task acceptance](task.md)
- [Session checkpoint](docs/WORK_CHECKPOINT.md)
- [Terminal troubleshooting](docs/TROUBLESHOOTING.md)

Section A and its five-row native core checkpoint are complete. E01 added locally verified durable goal/form storage; multiple task cards are the next increment, followed by the remaining B/C tasks requested in [task.md](task.md). P02 separately proves filesystem/process parity across the native matrix; local storage acceptance alone is not that claim.

Native core support is limited to the recorded OS/architecture/PTY or ConPTY tuples above. Windows ARM64, older OS versions, and untested GUI terminals remain unclaimed. Cross-compilation is not runtime proof. P02 gates filesystem/data-directory and owned process-tree lifecycle after recovery UX. P03 gates native archives/checksums and install/upgrade/uninstall preserving user data, after integrated acceptance and rights/provider review. Native-image and agent capabilities are qualified separately by OS/version/terminal.

The [design contract](docs/DESIGN_RULES.md) and [implementation plan](docs/IMPLEMENTATION_PLAN.md) also cover active-run task switching/quit, dispatch-time context revalidation, and E03 archive/delete, redaction, quotas, and retention. These remain subsequent workbench tasks; transactional single-goal storage is described above.

Later integration targets: Codex app-server first, an optional official Claude Agent SDK bridge, and one ACP client exercised against OpenCode and Gemini. A form selects the workflow, not the vendor. Switching agents uses a reviewed fresh handoff; native provider sessions and grants are not portable. The Rust core/local UI needs no Node runtime; only the optional Claude SDK integration would add one. Interface research and a local Codex version/help check are not runtime interoperability verification.

The SVGs are design illustrations, not runtime screenshots. Actual artwork rendering is implemented; external agents remain unimplemented. Performance figures remain scoped acceptance targets, not measured results. Public distribution requires both artwork/name rights review and provider auth/branding/terms review.
