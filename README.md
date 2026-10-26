<div align="center">

<img src="https://raw.githubusercontent.com/PokeAPI/sprites/0dc105d5b12416932f29b594078282ab4be045f5/sprites/pokemon/133.png" width="128" height="128" alt="Eevee pixel sprite">

<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.png">
    <img src="docs/assets/wordmark-light.png" width="300" height="61" alt="eevee">
  </picture>
</h1>

**Your project. Your party. One terminal workspace.**

Local-first tasks, coding agents and reviewed file changes — built with Rust.

[Get started](#quick-start) · [Daily controls](#everyday-use) · [Providers](#providers) · [Downloads](#installation) · [Docs](#documentation)

<sub>macOS · Linux · Windows · English / 한국어</sub>

</div>

---

| Work on your project | Keep your companions | Stay in control |
| --- | --- | --- |
| Launch `ev` in a folder. Discover files, attach source and ask for changes. | Nine forms, persistent tasks, a party and reusable moves. | Inspect exact changes before allowing them. Keep approvals separate from conversation. |

The local workbench needs **no account, API key or Node runtime**. Connect a provider when you want model assistance.

## Quick start

**macOS / Linux / WSL**

```sh
curl -fsSL https://github.com/kimmandoo/eevee/releases/latest/download/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
ev
```

**Windows · PowerShell 5.1 / 7 · no admin required**

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/latest/download/install.ps1)))
$env:PATH = "$env:LOCALAPPDATA\ev\bin;" + $env:PATH
ev
```

**Start inside your project folder.** That folder is the project boundary, even inside a larger Git repository. Plain folders work too. To change projects, exit and launch `ev` from the other folder.

1. **Connect:** `/connect` → choose a provider → choose a model. Setup never sends your draft.
2. **Ask:** type in the composer and press **Enter**. **Ctrl+S** saves locally without sending.
3. **Review changes:** required permissions appear automatically. Inspect the scope, then allow once or deny.

**No `/init` is required.** Ask a question or request a project plan directly. `/init` is only an optional request to create or update the project's `AGENTS.md` guide; it is not account setup, project registration or permission to write.

Terminal: **60 × 18 minimum**, **120 × 36 recommended**. Add `--language ko` for Korean.

> **Upgrading?** Cleanly exit all instances and back up the **entire data directory**, including database, journals and outputs, before `ev update`. Latest public release is v0.1.31; checked-out v0.1.32 is unreleased Wave5 development. Equal SQL schema does not establish record-reader or lifecycle-rollback compatibility. Offline rollback requires matching build-derived reader identity and data validation; never bypass admission based only on the version or schema number.

## Everyday use

| Want to… | Use |
| --- | --- |
| Find an action | `/` or **Ctrl+K**; search by name or description |
| Connect / change model / reasoning | `/connect` · `/model` · `/effort` |
| Attach source | `@` → choose a file → preview → **Enter**; **r** selects a range |
| Read earlier output | Mouse wheel or **PgUp/PgDn**; **Alt+End** follows the latest output |
| Resume a deferred permission | **Alt+A** or `/approvals`; **Esc** defers without answering |
| Fix a blocked connection | **PgUp/PgDn** reads the reason; **c** opens setup without sending or retrying |
| Choose a form / request policy | `/forms` · `/role` |
| Manage companions / saved tasks | `/party` · `/lead` · `/tasks` |
| Author / run / inspect a local command | `/command-edit` · `/execute` · `/run` |
| Limit exact write targets | `/paths`; `[]` clears the restriction, not the approval requirement |
| Inspect changes / history | `/diff` · `/history` |
| Stop work / exit safely | `/cancel` · `/cancel-all` · `/quit` |

**One continuous transcript.** Responses and tool observations scroll together. Generated code has its own filename, status and numbered code surface; received code is **not yet saved**.

**Neutral permission arrival.** Enter opens the review; it does not approve. Inspect the full scope, then use the displayed decision controls. Typing, paste and held-key repeats never grant permission. Other editors and background partners do not steal focus.

<details>
<summary>Display, language and advanced controls</summary>

```sh
ev --language ko            # Korean; --language en selects English
ev --theme light           # light palette
ev --ascii --color mono    # plain-terminal fallback
ev --no-animation          # static indicators; streaming remains enabled
ev --data-dir /abs/path    # separate, isolated application storage
ev --help
```

- `/motion-off` persists reduced motion; `/motion-on` restores it. CLI overrides last for one launch.
- `EV_LANGUAGE=ko` also selects Korean. CLI wins; automatic selection uses the locale environment with English fallback. Korean still requires UTF-8 in ASCII mode.
- Output scrolling pauses following without moving the draft caret. **Alt+Up/Down** jumps between transcript segments; **Alt+Home** reaches the start.
- `/session-new` and `/session-resume` open advanced exact-payload review. `/chat-new` clears the scoped process-local native conversation, not your draft or durable evidence.
- `/reset` reviews a new local task while retaining the old one. `/init` prepares an editable request to inspect the current project and create or update `AGENTS.md`; Enter starts it, and each file change needs approval. It preserves your explicit read-only policy, selected paths and form.
- Search `/` for the full action list, including workflows, handoffs, memories, records and recovery actions.

</details>

<details>
<summary>Released in 0.1.28</summary>

| Improvement | Use |
| --- | --- |
| Actionable connection failures | `/connect` → diagnostics; **n** opens the displayed next step without replaying the request. HTTP 403 means access refusal, not proven logout. |
| Persistent reading density | `/density` toggles; `/density-compact` and `/density-comfortable` choose explicitly. Independent of theme and reduced motion. |
| Workflows from reviewed work | `/recipe-save` → edit portable steps/scope/criteria → select actual passing commands → inspect exact TOML → **y** creates a new file. |
| Reviewed application update | `/update` → inspect version/source/cache/data scope → explicitly apply; the saved task resumes after replacement. |
| Safe retained-package rollback | `/rollback` → inspect compatibility and scope → explicitly apply. Unknown or incompatible serialized formats refuse; equal SQL schema alone is not enough. |
| Exact context comparison | `/context-compare` or **c** in Sources prepares counts locally. Change attachments to compare the same turn; no model request is sent. |

Recipe preview defaults to staying; **Enter/Esc** cancels without creating a file. Existing destinations are never overwritten. `/recipe-path`, `/load-recipe` and `/preview-recipe` reopen and stage a plan in another project; imported criteria remain unchecked and effects still require fresh review. Authoring drafts survive navigation in the current process, not restart.

These commands shipped in v0.1.28, including the previously unreleased 0.1.27 improvements.

</details>

## Your party, without coding restrictions

**Every form can discover, read, create and edit project files.** Evolution changes the companion's appearance and specialty, not permission. Explicit Plan, independent Review and read-only requests remain nonwriting.

File-based project work supports up to 1024 distinct files per request, each up to 4 MiB; reviews of existing files show only the changed region with context. Native HTTP tool loops allow 1024 calls across 1025 rounds, bounded to 8 MiB per request body and 128 MiB cumulatively. `/auto-approve` (or Shift+A in a file/command review) approves the current partner's file changes and commands for the rest of the task, like other coding agents' auto modes; the header shows `AUTO-APPROVE`, `/auto-approve-off` ends it, and task change or restart clears it. Questions still ask. Earlier completed writes remain if a later operation fails; cancellation is not rollback. Dependency installation and shell-based scaffolding require the separate command-review flow.

Missing native model configuration opens the local model picker and keeps your request. Selecting a model does not send it: press Enter deliberately afterward. A file permission's Enter opens the proposed content; return to its decision controls to allow once or deny after inspecting the complete scope. Merely typing into an arriving popup grants nothing.

v0.1.29 fixes short plan-to-implementation requests: after planning, `Implement it` / `구현해줘` selects reviewed implementation and carries the completed public plan forward. Plan-bound `진행해줘` also works; a generic continuation after analysis does not grant writes. Explicit Plan/Review still wins. Host file tools can create nested folders and save edits even when the provider's own shell is read-only; each changed file still needs exact approval.

Native account connections use Eevee's selected login/profile rather than silently reading CLI `auth.json`. To reuse CLI credentials, explicitly choose **Load CLI** in connection setup. Missing files, OS access denial and unsafe credential permissions are separate errors. Private Windows credentials owned by the current user or a trusted elevated owner are accepted only with a safe ACL; UTF-8 BOM is supported, UTF-16 is not. No credential file or permissions are silently repaired.

### Reviewed context and tools — v0.1.29

| Action | Boundary |
| --- | --- |
| `/optimize` | Preview ranked source ranges under a local token budget, with reasons, omissions and mandatory-pin overflow decisions; apply separately. |
| `/compact` | Preview a deterministic evidence-linked continuation ledger; failed checks and changed sources remain visible. No model summarization. |
| `/context-apply` / `/context-cancel` | Apply a reviewed local packet or cancel without sending. Source changes stop later dispatch. |
| `/remember-source` / Sources `m` | Review adopting an exact checked capture as a bounded memory note; approval and later context selection remain separate. |
| `/receipts` | Inspect request identities and exact local section counts after restart; source-only purge preserves counts, not provider-side history. |
| `/tools` / `/tool-setup` | Search local metadata or review explicit catalog setup. Selecting a schema grants no execution permission; unavailable adapters say so. |
| `/artifacts` / `/artifact-search` | Inspect immutable owned tool output by exact pages, literal search and failure locations; purged, stale or wrong-task handles are refused. |
| `/run-details` | Inspect command identity and execution/storage errors, separately from retained tool artifacts. |

Optimizer counts use a named offline Unicode-boundary tokenizer, distinct from the existing heuristic estimate, provider usage and billing. Revision caches retain local identities and bounded assembly metadata, not a promise of provider caching. Tool catalog exposure is currently supported only by the compatible Chat Completions host-tool route. Artifact pages prioritize retained content; `i` opens full immutable identity and exit details.

Receipt capacity is reserved before sending; a full task refuses without losing the draft. Failed writes offer storage-only retry, never automatic resend. After a definite matching request outcome and a proven rollback, `/receipts` → `d` can explicitly acknowledge an **unsaved receipt gap** through a neutral full-scope review. This releases only the failed local write: the gap stays memory-only and is lost on restart. Ambiguous writes remain blocked.

Adopted source notes preserve exact bytes, including CRLF, and use explicit capture identity rather than matching equal text. They are independently approved data: capture, receipt or artifact purge **does not delete the saved note**. Use `/delete-memory` separately; neither operation promises secure erasure or deletion of provider-owned history.

Filename discovery accepts a project-relative directory scope before scanning. Safe files larger than the 4-MiB mutation limit remain discoverable; bounded reads may inspect sources up to 16 MiB with a full-source digest and an explicit noneditable flag.

| Feature | What it does |
| --- | --- |
| **Nine forms** | Automatic specialty selection or a fixed appearance; a real evolution gets a localized receipt. |
| **Party & cooperation** | Up to three partners can own work on one task, with separate drafts, receipts and approvals. Only one edit-capable owner at a time. |
| **Moves & portable TMs** | Remember up to four moves per form; teach, import or export reusable steps. Each use still needs fresh validation and ordinary review. |
| **Formations** | Save and map party recipes to existing partners. Applying one never sends work or transfers permissions. |
| **Held context** | Remember a file or approved memory for a form. Explicitly accept it before it becomes outgoing context. |
| **Ribbons & records** | Inspect source-linked history, not scores, correctness guarantees or capability unlocks. |

Sprites use Kitty/Sixel when supported and fall back to text. Installers fetch verified artwork separately; source builds can use `ev --download-artwork`.

## Implemented baseline — v0.1.31 Waves 1–4

`/evolution-preview` explains the current Request's predicted appearance, actual operation, automatic/manual mode and explicit policy. Inspection preserves the exact draft/caret and sends nothing; ordinary Send recomputes the decision. Appearance is never a write grant.

Offline snapshots copy the **entire data directory**, including private text and any secrets present there. They are not encrypted or redacted. Stop all Eevee instances first; the source is read without migration or repair. Source and destination must be separate, and destinations must not exist:

```sh
ev snapshot create --data-dir /absolute/eevee-data --destination /absolute/new-backup --acknowledge-private-data
ev snapshot verify --source /absolute/new-backup
ev snapshot restore --source /absolute/new-backup --destination /absolute/new-data --acknowledge-private-data
ev --data-dir /absolute/new-data
```

Restore validates integrity and current-reader compatibility before removing its incomplete marker. It preserves original project identities, task drafts and retained evidence, not active ownership, running work, provider sessions or approvals. It does not merge data, relocate projects or import external credentials. Interrupted/failed destinations remain visibly incomplete and cannot be opened as usable app data; the source and backup are unchanged.

Controlled native/Codex file tools also support exact regular-file deletion and same-project no-overwrite rename after a preceding full-digest read and explicit review. Deletion is irreversible; neither operation promises rollback. Exact edits use 1–32 nonoverlapping literal replacements against one original full SHA-256, with one final-content approval and the existing64-KiB limit. Rust proposals show baseline/proposed parse outcomes **before writing**; this is not compilation, type checking or verification, and never repairs or reformats bytes automatically. Unsupported adapter operations are disclosed rather than delegated to an unreviewed shell.

### Reliable coding, build and run

An ordinary request to build, run or test selects Implement, including after planning; explicit Plan/Review, read-only restrictions, quoted source and negated instructions do not grant execution. Every form can use the selected route's supported tools. These routes adapt the actual tool-loop, explicit command and retained-output behavior of an external reference; Eevee does not import it as a dependency or inherit its permissions.

| Route | Reviewed execution |
| --- | --- |
| Native HTTP | Implement-only `eevee_run_command {cwd, command}` proposes a project-relative cwd and typed Direct/Shell command to the existing reviewed runner. |
| Controlled Codex | The same host command tool and runner; the provider's own read-only shell is not the host execution path. |
| Claude Agent SDK | Complete bridge10 exposes host MCP `eevee_command/run_command` through production policy and a separate command-server transport. Native Bash/Agent remain disabled; SDK writes retain external-policy limitations. |
| OpenCode / Gemini / Devin ACP | Implement-only terminal create/output/wait/kill/release callbacks use the same reviewed runner. Plan and inspection advertise no terminal capability; environment overrides and PTYs are refused. Agent-owned tools remain external policy. |

Every command needs its own exact one-shot approval. The runner inherits the process environment without sanitizing or freezing it and is **not an OS sandbox**; project-relative cwd does not confine command effects. Nonzero exit, cancellation and unknown outcomes are not success. Commands are never automatically replayed; unknown effects fail-stop and retain writer ownership. Native/Codex advertised artifact tools can retrieve full **retained** output; SDK/ACP return bounded output with workbench inspection, not an unadvertised model artifact-retrieval promise.

### Bounded observation, plans and evidence

- **RX03 structural search:** native/Codex Rust expression-pattern search returns digest-bound original byte ranges, lines/columns and named captures. This is bounded syntax matching, not semantic name binding, an AST rewrite or compilation. Invalid/unsupported patterns, parse failures, no match and incomplete scans remain distinct; partial output never proves absence.
- **TC04 observation reuse:** request-private Read/Structural results may be reused only after current source/ancestry/privacy and Git-policy fingerprints. List deliberately bypasses reuse. Cache labels retain original observation time and actual costs; fingerprinting still runs real Git subprocesses/stdio threads and may cost more than recomputation. Commands, tests and builds are never cached.
- **TC02 / TC05 plans:** `/tool-plan` stages a typed 1–32-node DAG. At most three independent Read/List nodes overlap; commands serialize with reads and each other. `/plan-start`, `/plan-resume`, `/plan-cancel`, `/plan-reconcile`, `/plan-serial` and `/plan-history` expose review, recovery and inspection. In the drawer, `e` edits inert JSON, `s`/`r` open neutral reviews, `y` schedules only after complete scope, and each command still needs `/approvals`. `b` separately reviews a read-only serial comparison; `h` and `[`/`]` inspect history. Eight bounded historical plans are retained without eviction; full capacity refuses staging. Restart schedules nothing; resume never repeats confirmed commands, even failed ones, and unknown effects block recovery.
- **D01 impact checks:** `/impact-checks` proposes conservative offline syntax-name reverse-impact checks. Unmapped/shared paths require broader workspace Cargo checks; ignored tests require explicit `--ignored`. `/stage-impact-plan` stages only an inert typed plan, not execution or permission.
- **PK07 history:** fail/pass/fail, source-transition and supported-diagnosis links derive from retained records. A previous resolution is a historical lead, never current verification; missing/stale spool identity remains unavailable. Reviewed evidence-only Umbreon relay transfers references, not execution authority or approvals.

## Unreleased Wave5 configuration — source 0.1.32

- **MV09 moves and TMs:** local teaching/import/replacement review shows exact old/new definitions, indexed ordered argv including empty arguments, cwd, pin-set semantics, instructions and criteria. Name-only edits are distinct. Same-name replacement preserves forgotten originals and their usage IDs; taught copies stay independent. Every ordinary use still needs fresh execution/context consent.
- **PK08 held renewal:** `/held-renew` opens the existing replacement chooser. Use `/held-set-file` from reviewed Sources or `/held-set-memory` on the highlighted approved revision, then deliberately confirm complete old/new review. Unavailable old bodies stay unavailable; source controls are displayed escaped. Renewal changes only one saved reference, never the current basket, selection, frozen request or provider permissions.
- **PT12 formations:** mapping shows separate staging eligibility and cached route/move/held/draft/writer observations. `m`/`h`/`r` open scoped read-only existing inspectors; Escape returns to the mapping without changing lead or drafts. Unknown qualification is not definite unavailability, and inspection performs no discovery/network.

TM schema1, formation schema2, SQL19 and SDK bridge10 remain. Local Wave5 drivers are opt-in; they do not add another default native CI cross-product. Public installers below remain pinned to0.1.31.


## Providers

Open **`/connect openai`** for the unified OpenAI family: account login, API key, compatible/local endpoint or an advanced CLI route. The selected method determines the transport — no silent billing, account, protocol or model fallback.

| Connection | Setup |
| --- | --- |
| **OpenAI account** | Native browser or device login; no Codex installation needed for inference. Device login works without a localhost callback. |
| **OpenAI API** | In-app API key or `OPENAI_API_KEY`. |
| **Anthropic** | In-app API key or `ANTHROPIC_API_KEY`; no subscription OAuth reuse. |
| **Google Gemini** | In-app API key, `GEMINI_API_KEY` or `GOOGLE_API_KEY`. |
| **Compatible / local** | Explicit endpoint, model, protocol and key or keyless mode. HTTPS required except loopback HTTP. |
| **External agents** | Codex app-server, Claude Agent SDK sidecar, or OpenCode / Gemini / Devin ACP. Their dependencies and control boundaries differ. |

- Saving a connection or choosing a model **never sends the retained request**. Exact custom model IDs remain available when catalogue discovery fails.
- Ordinary requests include a bounded current-folder root/filename observation. Source bodies are not automatically attached; discovery grants no write access.
- Native/Codex tools support discovery, reads and exact Create/Replace or hash-bound edits. SDK and ACP support differs: external agent tools are **not an Eevee sandbox**.
- Native conversation history is process-local. Cosmetic evolution preserves same-policy history; an actual policy change starts fresh context while retaining the visible transcript.
- Account grants can expire and require explicit reconnect. No automatic renewal or credential fallback is promised.
- Native account requests use Eevee's selected defaults or explicit environment settings, never unrelated external CLI model files.

**Credential storage:** session-only by default. Explicit private persistence uses owner-private **plaintext**, not encryption or an OS keychain, under `~/.eevee-connections/`. Forgetting an app connection does not revoke remote credentials or modify official provider stores.

[Authentication, SDK setup, configuration and provider limits →](docs/AGENT_COMPATIBILITY.md)

## Data and trust

| Boundary | Behavior |
| --- | --- |
| **Local state** | Tasks, drafts, excerpts and journals live in local SQLite. “Saved” means storage acknowledged the write; pending work is not yet durable. |
| **File changes** | Exact one-shot consent precedes mediated writes. Stale content, unsafe ancestors and protected/excluded paths refuse; a shown code block is not a created file. |
| **Permission ownership** | Decisions belong to the original request and partner. Evolution, restart and remembered settings never restore grants. |
| **Uncertain outcomes** | Cancellation does not undo completed writes or prove remote completion. Unresolved effects stay visible and can block new work. |
| **Privacy** | Review outgoing context and exports. Credential masking is best-effort; reported usage stays scoped and unreported billing remains unknown. |

Default data directories: `~/.local/share/eevee` on Linux, `~/Library/Application Support/eevee` on macOS, `%LOCALAPPDATA%\eevee` on Windows.

**Keep application storage separate from project source.** A project root used as `--data-dir` intentionally excludes that whole subtree from source tools. Provider configuration, hooks, external tools and resumed sessions may have effects beyond Eevee's mediated boundaries.

Unreleased0.1.32 source retains **SQL Schema19**, the strict plan format and complete Claude **bridge revision10** source/policy/dependency files, replaced together. New valid historical same-name moves and stale approved-memory references change record admission despite unchanged serialized shapes: the genuine0.1.31 reader refused both tested fixtures without changing database bytes, and the current reader resumed without automatic replay. Equal SQL schema does not promise older-reader compatibility or bypass conservative build-derived lifecycle rollback checks. Keep a **full-directory pre-upgrade backup**, not a task export or database-only copy.

## Installation

Version-pinned [v0.1.31](https://github.com/kimmandoo/eevee/releases/tag/v0.1.31) packages and installers are listed below. Exact-source Check/all-five-native qualification and all public asset/checksum identities passed; final public installation/runtime evidence is recorded in the [checkpoint](docs/WORK_CHECKPOINT.md). Source builds and controlled peers are not live-provider conformance proof.

| Platform | v0.1.31 package |
| --- | --- |
| macOS · Apple Silicon | [arm64](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/ev-darwin-arm64.tar.gz) |
| macOS · Intel | [x86_64](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/ev-darwin-x86_64.tar.gz) |
| Linux / WSL | [x86_64](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/ev-linux-x86_64.tar.gz) · [arm64](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/ev-linux-arm64.tar.gz) |
| Windows | [x86_64](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/ev-windows-x86_64.zip) |

[Checksums](https://github.com/kimmandoo/eevee/releases/download/v0.1.31/SHA256SUMS) · [Release notes](CHANGELOG.md)

```sh
ev update --check   # read-only release check
ev update           # upgrade in place; back up data first
ev uninstall        # remove the executable, retain user data
```

Installers verify SHA-256, refuse unowned command collisions and **do not change your shell profile or persistent PATH**. macOS builds are not notarized. WSL uses Linux packages, not Windows packages.

<details>
<summary>Pinned versions, persistent PATH and offline installation</summary>

**macOS / Linux**

```sh
curl -fsSLo install-ev.sh https://github.com/kimmandoo/eevee/releases/download/v0.1.31/install.sh
sh ./install-ev.sh --version v0.1.31
```

Add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile (`~/.zshrc` or `~/.bashrc`) for future terminals. If macOS blocks a browser-downloaded archive, inspect its source/checksum before removing quarantine with `xattr -d com.apple.quarantine ~/.local/bin/ev`.

**Windows**

```powershell
& ([scriptblock]::Create((irm https://github.com/kimmandoo/eevee/releases/download/v0.1.31/install.ps1))) -Version v0.1.31
[Environment]::SetEnvironmentVariable('Path', "$env:LOCALAPPDATA\ev\bin;" + [Environment]::GetEnvironmentVariable('Path', 'User'), 'User')
```

**Offline:** download the matching archive, `SHA256SUMS` and installer from the same release, then run:

```sh
sh ./install-ev.sh --archive ./ev-linux-x86_64.tar.gz --checksums ./SHA256SUMS
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Archive "$PWD\ev-windows-x86_64.zip" -Checksums "$PWD\SHA256SUMS"
```

Offline installs skip artwork. Run `ev --download-artwork` when online. Installer `--no-animation` / PowerShell `-NoAnimation` and `ev update --no-animation` use static progress; redirected output and CI remain escape-free.

</details>

## Development

Unreleased source v0.1.32 targets **Rust1.90.0**. Latest public packages remain v0.1.31. Optional complete bridge10 Claude sidecar: **Node24+**. These instructions build the checked-out source independently of package installation.

```sh
git clone https://github.com/kimmandoo/eevee.git
cd eevee
cargo build --locked --release
./target/release/ev
```

<details>
<summary>Checks, terminal smoke runs and localization</summary>

```sh
rustup toolchain install 1.90.0 --profile minimal --component rustfmt --component clippy
cargo +1.90.0 fmt --all -- --check
cargo +1.90.0 clippy --locked --all-targets --all-features -- -D warnings
cargo +1.90.0 test --locked --all-targets --all-features
npm ci --ignore-scripts --no-audit --no-fund --prefix sidecar
npm test --prefix sidecar
```

- [Native CI](.github/workflows/native.yml) tests, packages and offline-installs all five targets. [Release CI](.github/workflows/release.yml) publishes the exact qualified packages.
- Native runtime coverage is explicit rather than repeating the entire cross-product on every architecture. Independent drivers run two at a time; every selected driver must pass, and each run retains its coverage manifest, timings and failure logs.

| Native target | Runtime coverage |
| --- | --- |
| Linux x86_64 | Full English/Korean protocol, controlled Codex, context and tool journeys at both sizes |
| Windows x86_64 | Representative file route and all context/tool journeys in both languages and sizes |
| macOS arm64 | Representative bilingual file route plus Korean context/tool journeys at both sizes |
| Linux arm64, macOS x86_64 | Representative bilingual file route at both sizes; native unit/build/install gates remain |

- Reproduce coverage with `python scripts/native_workflows_smoke.py --binary target/release/ev --output /tmp/ev-native-proof --profile full --jobs 2`; use a fresh output directory. `--plan-only` prints the selected scope without launching work. Individual driver defaults still run their full scope.
- Terminal journeys: `scripts/*_smoke.py`, with `pyte==0.8.2` and `wcwidth==0.2.13`. Live provider checks are opt-in and may incur usage; deterministic peers are not live-account conformance.
- Windows smoke hosts need an unpackaged Python installation, not a Microsoft Store/App Execution Alias interpreter; see [troubleshooting](docs/TROUBLESHOOTING.md).
- Add Fluent resources under `locales/<BCP47-tag>/`. The build embeds them automatically with per-key English fallback; preserve message IDs and named arguments, without screen-level language branches.
- Keep approval, storage and cancellation contracts intact. Never commit credentials, task data or downloaded artwork.

</details>

## Documentation

| Read | For |
| --- | --- |
| [Design rules](docs/DESIGN_RULES.md) | Product behavior, privacy and lifecycle contracts |
| [Agent compatibility](docs/AGENT_COMPATIBILITY.md) | Authentication, transports and enforcement limits |
| [Troubleshooting](docs/TROUBLESHOOTING.md) | Terminal, installation and operational diagnostics |
| [Task backlog](task.md) · [Completed work](tasks/COMPLETED.md) | Upcoming work versus delivered scope |
| [Implementation plan](docs/IMPLEMENTATION_PLAN.md) · [Checkpoint](docs/WORK_CHECKPOINT.md) | Architecture, acceptance gates and verification evidence |

## License and attribution

Original code, prose and the geometric wordmark: **[Apache-2.0](LICENSE)**. See [NOTICE](NOTICE) and [asset attribution](docs/ASSET_LICENSES.md).

Pokémon sprites are third-party artwork from a [pinned PokéAPI revision](https://github.com/PokeAPI/sprites/tree/0dc105d5b12416932f29b594078282ab4be045f5); its [notice](https://github.com/PokeAPI/sprites/blob/0dc105d5b12416932f29b594078282ab4be045f5/LICENCE.txt) attributes image copyright to **The Pokémon Company**. No franchise image bytes are bundled in this repository or native packages. Remote display/downloads and the software license grant no image, trademark or commercial-use rights.

**Independent and unofficial; no affiliation or endorsement.** The original wordmark has an editable [SVG source](docs/assets/wordmark.svg).
