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

The local workbench includes durable tasks, repository baselines, source baskets, and explicitly approved local commands with retained evidence. No model/provider is connected and context previews send nothing. Switching forms never grants permission or starts a command.

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
| `t` / `n` | Browse/resume task cards / create a new task after pending saves settle |
| `c` | Open criteria; Space toggles a criterion, `e` edits one-per-line text |
| `a` / `d` | Edit next action / request explicit criterion-reviewed completion |
| Ctrl+S in editor | Retain the field's draft and return without applying it |
| `w` | Inspect repository baseline/raw diff; `r` refreshes that read-only view |
| `v` | Browse source files/ranges and the task-owned context basket |
| `j` | Open local command approval, live output, history and recovery |
| `p` | Inspect a local handoff preview; in the picker, inspect the previewed role |
| `l` | Toggle sanctuary / linear text layout |
| Ctrl+K | Open the command palette |
| Tab / Shift+Tab | Cycle visible panes |
| Enter | Open the focused pane action or confirm a selection |
| Esc | Close the current overlay, or offer keep/discard for a changed draft |
| `?` | Show keyboard help |
| `q` | Request quit outside text entry; type a literal `q` inside the editor |
| Ctrl+C | Request safe exit; wait for pending saves or explicitly review unsaved data |

Destructive dialogs start on Keep/Stay. Tab/Right explicitly selects the action, then Enter confirms; repeated Enter cannot turn neutral inspection into approval. Active-run task/form changes and quit require Stay or Cancel-and-wait. Cancellation is not rollback.

Windows uses a single console-record reader with VT input enabled: bracketed paste stays literal, Alt-code release text is distinguished from ordinary key releases, and UTF-16 surrogate pairs are joined. The terminal guard restores the exact original input-mode bits on normal/error/panic exit. POSIX retains Crossterm's input reader.

The layout uses three panes at 120+ columns, two at 90–119, and one with drawers at 60–89. The normal minimum is 60x18. Local focus, selections, scroll offsets and drafts survive overlays/resize. The picker uses a 3x3 grid at >=90x30, otherwise a list and portrait; `p` exposes role/artifact details. Rendering occurs on state changes, with bounded waits while work/approval is pending and blocking input when idle. Linear mode exposes the same local actions without artwork; screen-reader compatibility is not claimed.

Auto color uses advertised true-color support, then 256-color support, otherwise monochrome. Nonempty `NO_COLOR` overrides color flags. `TERM=dumb` also selects ASCII; a non-UTF-8 locale selects ASCII borders. Dark and light themes are explicit choices.

## Durable local tasks

`t` browses saved task cards; `n` creates another task in the current project. `g`, `c`→`e`, and `a` edit the goal, criteria, and next action. Enter applies locally; Ctrl+S retains a per-field draft without applying it. Editing/navigation changes are saved by a bounded worker. Only the matching committed revision shows **Storage: saved**; retained drafts are labelled separately. Restart restores the active task, form, focus/layout, and drafts. Explicit `--layout` overrides remembered layout for that invocation.

| Platform | Default data directory |
| --- | --- |
| macOS | `~/Library/Application Support/eevee` |
| Linux | `$XDG_DATA_HOME/eevee`, or `~/.local/share/eevee` |
| Windows | `%LOCALAPPDATA%\\eevee` |

Use `ev --data-dir PATH` for isolation. `workbench.sqlite3` stores schema-versioned tasks; `workbench.lock` admits one writer across directory aliases. Another instance shows a storage error rather than taking over. Press `r` to retry after resolving conflict/disk/permission errors; `i` shows details. Pending saves must settle before switching or creating a task. Normal exit finishes task/draft saves; failed storage requires an explicit informed exit. Esc discard removes only the selected field's draft, not applied task data.

Completion requires at least one criterion, all criteria checked, no retained goal/criteria draft, and a deliberate neutral-focus confirmation. It records your review, not verified execution evidence. Editing a completed goal or criteria first requires explicit reopening. Bounds are 256 tasks per project, 64 criteria of at most 256 UTF-8 bytes each, and 4096 bytes per goal/next-action/draft.

Schemas 0–3 migrate transactionally to schema 4, retaining task lifecycle metadata and adding command drafts, task-owned excerpts and execution journals. Identifiable credentials are masked before persistence. Corrupt, foreign, or newer schemas are never reset. Interrupted single-database rollback migrations are first recovered and validated on private copies, bounded to 64 MiB per database/journal; WAL/SHM and multi-database recovery are refused rather than guessed. Preserve originals for manual recovery if automatic recovery is unavailable.

`ev --data-dir PATH --export-data NEW_FILE.json` exports readable known **task rows only**, without creating/migrating the database or overwriting the output. It is not an excerpt/output backup, repair, or guarantee that corrupt data is readable. Project/data paths must be Unicode; display escaping never changes stored project identity.

In task cards, `h` switches active/archived views; `a` archives and `r` restores. Archive is reversible and does not complete a task. Delete fetches an authoritative preview of tasks/drafts and owned excerpt/run IDs, starts on **Stay**, and requires confirmation. PgUp/PgDown scrolls; `n`/`p` pages large artifact lists. `x` previews archived records eligible under `--retention-days N` (default 30). Active work prevents archive/deletion/purge. Retention never silently evicts data. A physical-cleanup failure reports committed metadata plus a warning, not a fictitious rollback.

`--quota-kib N` limits shared live SQLite pages, retained output and committed capture reservations (default 65536, minimum 64). Schema overhead counts; physical database files may retain free pages, and transactions/migrations require temporary disk space. Lowering the quota preserves existing records. Failed task saves retain drafts; reclaim eligible data or increase capacity, then `r` retries task storage. Context/journal storage has its own explicit `s` retry, never automatic command replay.

Identifiable token prefixes, credential assignments, Bearer credentials, URL userinfo and private-key blocks are masked before task persistence. Detection is incomplete; volatile editor text may remain unmasked. Review exports before sharing. Secret-bearing project identities are rejected rather than rewritten into another project's identity. Local purge does not delete repository changes, exported files, or provider sessions, and does not guarantee forensic secure erasure of SQLite pages/journals/backups.

## Repository evidence

`w` opens the initial session baseline and current raw HEAD-to-worktree diff; `r` refreshes without replacing the baseline. Pre-existing dirty work stays labelled **pre-existing**; later external changes are **unattributed**, never presumed agent-authored. A non-Git directory remains usable for durable tasks, with repository inspection explicitly unsupported.

The bounded worker records raw-byte SHA-256 identities, HEAD/index identities and tracked/untracked changes without writing user files or the index. Git is used only for metadata/raw objects; repository filters, textconv, external diff, fsmonitor, hooks and network transports are not invoked. The diff is a raw full-file comparison, not filter-normalized Git output. Ignored untracked files and submodule contents are outside coverage; races, unavailable reads and truncation are explicit, not a clean verdict. Symlink targets are not read.

Capture limits: 4096 paths, 2 MiB per file, 32 MiB aggregate file/object captures, and 256 KiB diff. Git commands have a 10-second deadline and scanning checks a 30-second deadline; blocked filesystem syscalls are not claimed interruptible. `python3 scripts/repository_smoke.py --binary target/release/ev --output target/repository-smoke` exercises the actual TUI at 120x36/60x18, external edits, byte/index preservation and non-Git task resumption.

## Source context

`v` opens eligible repository files. `/` filters literal, case-sensitive filenames; `r` selects a one-based inclusive `start:end` range; Enter inspects line 1. `p` pins the inspected capture after storage validation. `b` toggles files/basket, Delete removes a reference, and `x` neutrally reviews purging captured bytes while retaining unavailable metadata. `u` refreshes repository evidence.

The worker excludes ignored untracked files, credential paths, binary/non-UTF-8/oversized files and links outside its safe read boundary. Repeat `--exclude-context PATH` for up to 32 repository-relative prefixes. Exclusion matching is conservatively case-insensitive on all hosts, so it can exclude case-distinct siblings on a case-sensitive filesystem; stored source identities are never case-folded or merged. Limits: 2 MiB source files, 64 KiB per capture, 16 excerpts per task. Secret masking happens before persistence; private-key masking that changes line numbering refuses the range instead of silently shifting it.

`a` previews the exact JSON basket envelope as escaped text, with UTF-8 bytes and a labelled bytes/4 token estimate; `n`/`p` pages large envelopes. `d` repeats source identity, current ignore/exclusion, opened-handle containment and persisted-reference checks. Changed/missing/purged captures refuse assembly; no replacement bytes are silently substituted. **Neither action sends data.** A future provider may add rules, history or independent reads; this basket is not a complete effective prompt or a provider sandbox.

## Local commands and evidence

Open `j`, then `e` to edit a command. Direct input is a JSON argv array, for example `["cargo","test"]`; arguments are never implicitly shell-split. A shell requires all three explicit fields, e.g. `{"shell":"/bin/sh","dialect":"posix","text":"printf hello"}`. Native Windows dialects are `cmd` and `powershell`; provide the actual shell executable. Direct `.cmd`/`.bat` execution is refused. The UI command field is bounded to 4096 UTF-8 bytes.

Apply saves the command and prepares fresh source/executable/cwd identities, **not execution**. `v` opens a neutral one-shot review showing exact escaped arguments/shell text, canonical paths, executable hash/identity, expiry, capture reservation and enforcement limits. Esc closes without granting; `x` denies. An explicit affirmative selection plus Enter admits only the correlated current request, after a committed storage reservation and dispatch-time identity checks. Editing invalidates old permission; expired/changed/double answers cannot revive it.

`--spool-kib N` reserves 1–8192 KiB per command (default 64), plus 256 KiB journal headroom under the shared quota. Local execution owns a POSIX process group or a suspended-process Windows kill-on-close Job before user code starts. Stdin is null; filesystem/network effects and inherited environment are **not sandboxed**. POSIX sessions that escape the group or external effects cannot be claimed contained. Cancellation escalates within a bounded interval, records unresolved closure as outcome unknown, and never claims rollback.

Live output strips terminal controls and masks recognizable credentials best-effort. Capture/queue limits continue draining without unbounded memory or a blocked child; sticky gap/truncation indicators distinguish missing live display from incomplete retained bytes. Scroll/search (`/`, `n`/`N`) pauses follow, `f` follows again, and a new-output count preserves the current anchor. `h` opens history; Enter reads retained output in bounded pages (`[`/`]`); `d` shows full evidence/errors; `p` neutrally reviews output purge. Limits: 256 runs per task, 256 KiB/4096 lines in the live window, 64 KiB retained pages. No evidence is silently evicted.

Results preserve before/after source coverage, timing, exit/signal, cancellation and partial/unavailable flags. Inputs changed during a run are ambiguous; a later explicit `r` observation can make old results stale. Matching hashes are not an atomic snapshot, live watch, correctness proof or automatic task completion. Restart reconciles prior reservations/runs as interrupted without restoring ownership or replaying commands. Task/form/quit transitions wait for owned work and the final journal acknowledgement; unknown outcomes or failed persistence require separate informed exit.


## Development checks

```sh
cargo +1.90.0 test --locked --all-targets
cargo +1.90.0 clippy --locked --all-targets -- -D warnings
cargo +1.90.0 build --locked --release
python3 scripts/workbench_smoke.py --binary target/release/ev --output target/workbench-smoke
python3 scripts/repository_smoke.py --binary target/release/ev --output target/repository-smoke
python3 scripts/context_smoke.py --binary target/release/ev --output target/context-smoke
python3 scripts/execution_smoke.py --binary target/release/ev --output target/execution-smoke
python3 scripts/execution_recovery_smoke.py --binary target/release/ev --output target/execution-recovery-smoke
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

Section A and its five-row native core checkpoint are complete. Section B and R01/R02/UX02 passed local acceptance: 155 Rust tests, warning-free Clippy/release build, 11 workbench, 3 repository, 10 context, 8 execution, 6 execution-recovery, and 34 core terminal scenarios. The current macOS arm64 archive was installed offline and its context/execution/recovery surfaces exercised. P02 remains blocked before native runner startup by GitHub account billing; older five-platform installation artifacts do not contain these B/C changes.

Native core support is limited to the recorded OS/architecture/PTY or ConPTY tuples above. Windows ARM64, older OS versions, and untested GUI terminals remain unclaimed. Cross-compilation is not runtime proof. P02 gates filesystem/data-directory and owned process-tree lifecycle after recovery UX. P03 gates native archives/checksums and install/upgrade/uninstall preserving user data, after integrated acceptance and rights/provider review. Native-image and agent capabilities are qualified separately by OS/version/terminal.

The [design contract](docs/DESIGN_RULES.md) and [implementation plan](docs/IMPLEMENTATION_PLAN.md) govern active-run switching/quit and context revalidation. Excerpts and output now share the storage privacy/quota/purge policy. Providers, automated memory, and public Release/remote bootstrap remain unimplemented or withheld as recorded in the task gates.

Later integration targets: Codex app-server first, an optional official Claude Agent SDK bridge, and one ACP client exercised against OpenCode and Gemini. A form selects the workflow, not the vendor. Switching agents uses a reviewed fresh handoff; native provider sessions and grants are not portable. The Rust core/local UI needs no Node runtime; only the optional Claude SDK integration would add one. Interface research and a local Codex version/help check are not runtime interoperability verification.

The SVGs are design illustrations, not runtime screenshots. Actual artwork rendering is implemented; external agents remain unimplemented. Performance figures remain scoped acceptance targets, not measured results. Public distribution requires both artwork/name rights review and provider auth/branding/terms review.
