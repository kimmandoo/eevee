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

The local workbench includes durable tasks, repository baselines, source baskets, explicitly approved local commands with retained evidence, and optional Codex planning. Startup, form selection, and local context previews never submit a model request. Codex requires a separate reviewed planning action.

Goals are limited to 4096 UTF-8 bytes and remain local unless explicitly included in an approved Codex planning request. Unapplied editor drafts are never sent. Enter applies an edit and requests a local save; the header distinguishes saving, committed, and failed revisions. Ctrl+N inserts a newline. Shift+Enter works only when the terminal reports it distinctly. Esc on a changed draft asks whether to keep or discard it. Backspace/VT Ctrl+H and Delete remove complete graphemes; a truncated paste leaves a visible warning.

### Agent preflight

`o` opens the connection drawer for Codex, Claude Agent SDK, OpenCode ACP, and Gemini ACP. Discovery is passive: it inspects executable identity and known configuration sources without running an agent. `1`–`4` or arrows select a profile; `r` refreshes discovery. Installation, observed CLI version, authentication/billing source labels, configuration declarations, OS/architecture, adapter availability, and per-action control boundaries remain distinct.

Enter opens a **neutral trust preview**, not a connection. Review with PgDn/End; only after reaching the end can Tab select **Trust this version probe**, then Enter runs the previewed executable with literal `--version`. Esc cancels; resize clears consent, and terminals below 60×18 cannot approve. Executable and known configuration identities are revalidated before launch. Probes use an outside-project temporary cwd, owned-process cleanup, a three-second deadline (**ten seconds for Gemini's measured Node startup**), and a 16 KiB output limit. Only recognized version text is retained; arbitrary stdout/stderr is withheld.

Even `--version` executes arbitrary code. Consent includes the disclosed inherited environment, interpreters, dependencies, and provider-owned configuration effects; none is claimed sandboxed or fully pinned. Eevee never starts login/model requests, reads credential stores, or changes provider configuration during preflight. Known nonsecret declarations are not authoritative effective policy. Environment variable **names/presence**, never credential values, label possible authentication/billing sources.

Provider releases are pinned: **Codex 0.152.1**, optional **Claude Agent SDK 0.3.296 / embedded CLI 2.1.296 / Node ≥24**, **OpenCode 1.18.0**, and **Gemini CLI 0.28.0**. Other versions fail closed. CLI discovery is not SDK availability or authenticated interoperability. Current provider evidence is Linux/WSL x86_64 only, not native Windows/macOS parity. Claude SDK model scenarios require permitted API/Bedrock/Vertex credentials; the installed claude.ai subscription is not reused. Gemini's real ACP session creation is authentication-blocked on this host.

### Espeon / Codex planning

Save a goal (`g`), optionally capture sources (`v`), then open **`e`**:

1. `n` prepares a **new-thread preview**; `r` previews resuming this task's explicitly created thread from the current Eevee session. Neither connects or sends a prompt.
2. Review executable/configuration effects and the **exact outgoing task/source payload** with PgDn/End. Stay is neutral; Tab → Enter explicitly selects **Trust and run once**.
3. Eevee starts a private owned stdio app-server, checks authentication via its supported account API, selects its advertised default model, and requests Plan mode with provider-controlled read-only/network-restricted policy and user approvals. Task revision, committed excerpts, source content/containment, executable, and known configuration identities are checked again immediately before dispatch. Changes require a new preview.
4. Provider questions retain literal drafts. Choose an option or press `a` to type; `s` opens answer review, then Tab → Enter sends. Commands start on **Deny**; reviewing to the end permits **Allow once** only when the provider offers it. No persistent rule is saved; file-edit approvals and unsupported request types are refused.
5. `c`/Esc cancels the turn. Cancellation invalidates pending answers and waits for provider completion or bounded owned teardown; an unconfirmed outcome stays **unknown**, not successful. `q`/Ctrl+C offers neutral safe exit. Small terminals can cancel/exit but cannot approve new work.
6. Select a completed provider message/plan with arrows and press `d` to copy it into the **Next action editor**. Review/edit and explicitly save with Enter. Existing retained drafts are protected; decisions above 4096 bytes must be shortened. Saved decisions carry provider thread/turn/item provenance and remain **unverified planning**, not local command evidence.

One request is active at a time; task/form/source changes and local command starts cannot bypass it. Each turn captures at most 128 items, 32 KiB per item, and 512 KiB total; overflow stops with an explicit incomplete/unknown result. The session-local viewer retains at most 128 items across resumed turns and labels omitted history. Delta/final snapshots replace the same item rather than duplicating it. Private reasoning and raw stderr are not displayed. Restart retains only explicitly saved task decisions, never automatically reconnecting, restoring a grant, or resubmitting a turn.

Provider configuration, instructions, hooks/MCP, independent reads, and resumed history can add context or effects beyond Eevee's displayed payload. **Provider read-only policy is not an Eevee sandbox**; an approved command may run outside that policy. Provider account, billing, and retained thread history remain provider-owned. Authenticate using the provider's supported tools outside Eevee; Eevee does not import credentials, initiate login, or change global provider configuration.

Live verification is deliberately opt-in and may incur provider usage: `python3 scripts/planning_smoke.py --live --binary target/release/ev`. It requires the terminal-smoke Python dependencies and an already authenticated, compatible Codex installation; it is not wired into CI.

### Flareon / controlled implementation

Inside `e`, `1`–`4` preview an idle agent switch, `f` toggles Plan/Implement, and `x` edits a JSON array of root-relative existing files, e.g. `["src/main.rs"]`. Every switch and run requires fresh review; selection alone never starts a provider. Original sessions remain with their originating agent and operation.

Codex Implement admits **1–8 existing regular UTF-8 files, each at most 12 KiB**, through `eevee_edit_file` under the provider's read-only/network-disabled policy. Each full replacement shows its exact before/after patch, starts on Deny, and requires Allow once. The POSIX anchored/no-follow write route rejects protected, excluded, unselected, symlink/hardlink and changed-baseline paths; other platforms refuse this controlled route. It preserves reviewed dirty content, never resets, commits, merges, creates files or widens scope. Native edit/command escalation is denied. Other provider reads/config/MCP effects remain outside this write route's coverage.

After a change, inspect `w` against the pre-existing dirty baseline and deliberately choose a separate `j` check. `h` inside the agent drawer shows retained **provider-origin journals**, not Jolteon's local execution evidence. Records are append-only and quota-backed; failed persistence retains observations for storage-only retry, never provider replay. Purge is a separate neutral confirmation.

### Optional SDK and ACP peers

The Rust core/local-only workflow needs no Node or SDK. For the optional bridge:

```sh
npm ci --prefix sidecar --ignore-scripts --no-audit --no-fund
export EEVEE_CLAUDE_SIDECAR="$PWD/sidecar/main.ts"
node sidecar/main.ts --inspect
```

Supply an already configured, permitted `ANTHROPIC_API_KEY` or the explicit Bedrock/Vertex route described in [agent compatibility](docs/AGENT_COMPATIBILITY.md). Do not reuse a claude.ai subscription, copy login stores, or use an OAuth/helper fallback. Inspection loads the actual SDK without a model request; it does not prove authenticated query/question/permission/interrupt/resume behavior. SDK settings sources are empty, MCP strict-empty, and shell/delegation/web/worktree tools mandatory-disabled. Its selected-path Write/Edit hook is **agent policy, not atomic OS confinement**.

OpenCode uses a private `acp --pure` child; Gemini 0.28.0 uses `--experimental-acp`, not newer documentation's `--acp`. Both negotiate ACP1. Client filesystem/terminal callbacks and unknown question extensions are refused; provider-owned tools/config/MCP effects are explicitly external policy, not read-only or sandboxed. Only exact once-only permission options are offered. Resume uses only an observed same-task session and negotiated `session/load`; loaded history is not replayed as new output. Controlled ACP implementation is refused.

`s` inside the agent drawer explicitly previews **capability/session-list inspection without a model request**. It calls `session/list` only when negotiated, filters returned metadata to the reviewed project, and never turns listed sessions into resume grants. Gemini's absent method produces a visible refusal without calling it.

### Portable task handoff

`z` opens the manual handoff drawer: `j` JSON export, `m` Markdown export, `i` import. Exports are create-only and require exact review/consent. Version **1** carries only committed task facts, criteria, selected source excerpts/identities, selected decisions and unresolved work. Generated provider-session provenance is stripped from portable facts and outbound task metadata; provider sessions, grants, hidden reasoning and credentials are never portable.

Imported claims stay **unverified** and cannot create execution evidence or complete a task. Unknown/newer fields or schemas, unsafe sources and stale dispatch bindings are rejected. An accepted handoff must still match the current task/source revision immediately before dispatch; `b` inside the agent drawer explicitly deselects a stale handoff. Manual export/import never launches a CLI. Full Codex→Claude→ACP acceptance remains blocked until permitted Claude and Gemini credentials are available.

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
| `o` | Open passive agent preflight and explicit version-probe trust review |
| `e` | Open the selected agent's Plan/Implement drawer; `n`/`r` preview new/resumed turns |
| `p` | Inspect a local handoff preview; in the picker, inspect the previewed role |
| `z` | Open reviewed version-1 JSON/Markdown handoff export/import |
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

Schemas 0–4 migrate transactionally to schema 5, retaining task lifecycle metadata, command drafts, excerpts and local execution journals, and adding task-owned provider-origin journals. Identifiable credentials are masked before persistence. Corrupt, foreign, or newer schemas are never reset. Interrupted single-database rollback migrations are first recovered and validated on private copies, bounded to 64 MiB per database/journal; WAL/SHM and multi-database recovery are refused rather than guessed. Preserve originals for manual recovery if automatic recovery is unavailable.

`ev --data-dir PATH --export-data NEW_FILE.json` exports readable known **task rows only**, without creating/migrating the database or overwriting the output. It is not an excerpt/output backup, repair, or guarantee that corrupt data is readable. Project/data paths must be Unicode; display escaping never changes stored project identity.

In task cards, `h` switches active/archived views; `a` archives and `r` restores. Archive is reversible and does not complete a task. Delete fetches an authoritative preview of tasks/drafts and owned excerpt/run/provider-journal IDs, starts on **Stay**, and requires confirmation. PgUp/PgDown scrolls; `n`/`p` pages large artifact lists. `x` previews archived records eligible under `--retention-days N` (default 30). Active work prevents archive/deletion/purge. Retention never silently evicts data. A physical-cleanup failure reports committed metadata plus a warning, not a fictitious rollback.

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
python3 scripts/handoff_smoke.py --binary target/release/ev --output target/handoff-smoke
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

The [design contract](docs/DESIGN_RULES.md) and [implementation plan](docs/IMPLEMENTATION_PLAN.md) govern active-run switching/quit and context revalidation. Excerpts, local output and provider journals share storage privacy/quota/purge rules; selected decisions use ordinary task storage. Automated memory and public Release/remote bootstrap remain withheld as recorded in the task gates.

Codex planning and bounded POSIX implementation, the optional official Claude SDK bridge, one negotiated ACP client for OpenCode/Gemini, and reviewed manual/cross-agent handoffs are implemented. Implementation is distinct from authenticated interoperability acceptance: missing permitted Claude/Gemini credentials leave A04/A06/full A07 unchecked. A form selects workflow, not vendor; native provider sessions/grants are not portable. The Rust core/local UI needs no Node runtime.

The SVGs are design illustrations, not runtime screenshots. Actual artwork rendering and the scoped Codex planning flow are implemented. Performance figures remain scoped acceptance targets, not measured results. Public distribution requires both artwork/name rights review and provider auth/branding/terms review.
