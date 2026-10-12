# eevee

A local-first Rust terminal workbench for LLM-assisted development: one persistent partner, nine task-specific forms.

## Run

```sh
cargo run --release
cargo run --release -- --theme light
cargo run --release -- --color 256
cargo run --release -- --ascii --color mono
cargo run --release -- --layout linear --images off
```

Requires interactive stdin and stdout. The minimum Rust toolchain is 1.90.0. The current local companion has been exercised on macOS arm64 in a real PTY and Ghostty 1.3.1; other native platform claims require the CI evidence described below. Use `cargo run -- --help` for CLI options.

The sanctuary, nine-species artwork, grid/list picker, local handoff preview, bounded scrolling, and grapheme-aware goal editor are implemented. Agent execution, repository inspection, persistent tasks, and permission grants remain unavailable. Switching forms never starts tools or changes permissions.

Goals exist only until exit, are limited to 4096 UTF-8 bytes, and are never sent to a model. Enter applies an edit locally; Ctrl+N inserts a newline. Shift+Enter works only when the terminal reports it distinctly. Esc on a changed draft asks whether to keep or discard it. Backspace/VT Ctrl+H and Delete remove complete graphemes; a truncated paste leaves a visible warning.

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
| `g` | Edit the session-local goal |
| `i` / `b` | Open the inspector / companion drawer |
| `p` | Inspect a local handoff preview; in the picker, inspect the previewed role |
| `l` | Toggle sanctuary / linear text layout |
| Ctrl+K | Open the command palette |
| Tab / Shift+Tab | Cycle visible panes |
| Enter | Open the focused pane action or confirm a selection |
| Esc | Close the current overlay, or offer keep/discard for a changed draft |
| `?` | Show keyboard help |
| `q` | Request quit outside text entry; type a literal `q` inside the editor |
| Ctrl+C | Request safe exit; neutral Stay protects a session goal or changed draft |

Destructive dialogs start on Keep/Stay. Tab/Right explicitly selects discard, then Enter confirms; repeated Enter cannot turn neutral inspection into approval. No active jobs exist yet.

Windows uses a single console-record reader with VT input enabled: bracketed paste stays literal, Alt-code release text is distinguished from ordinary key releases, and UTF-16 surrogate pairs are joined. The terminal guard restores the exact original input-mode bits on normal/error/panic exit. POSIX retains Crossterm's input reader.

The layout uses three panes at 120+ columns, two at 90–119, and one with drawers at 60–89. Below 60 columns or 18 rows, resize guidance preserves the draft; back and safe exit remain available. Local focus, selections, and scroll offsets survive overlays and resize. The picker uses a 3x3 grid at >=90x30, otherwise a selected-centered list and portrait; `p` exposes role/artifact details at compact sizes. Rendering is event-driven with no idle animation or timer. Linear mode exposes the same local actions without requiring artwork; screen-reader compatibility is not claimed without real assistive-terminal evidence.

Auto color uses advertised true-color support, then 256-color support, otherwise monochrome. Nonempty `NO_COLOR` overrides color flags. `TERM=dumb` also selects ASCII; a non-UTF-8 locale selects ASCII borders. Dark and light themes are explicit choices.

## Development checks

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Run `python scripts/terminal_smoke.py --binary target/release/eevee --output target/native-smoke` after building. Install `pyte==0.8.2` and `wcwidth==0.2.13`; Windows also needs `pywinpty==2.0.15`. The harness launches the real binary in POSIX PTYs or native Windows ConPTY, records screen/host/console evidence, and builds one isolated lifecycle probe for normal/error/panic restoration. It does not certify a GUI terminal or native images. The native workflow runs the five OS/architecture rows and uploads results; see the checkpoint for observed outcomes rather than inferred support.


## Design

- [Product and visual contract](docs/DESIGN_RULES.md)
- [English implementation plan](docs/IMPLEMENTATION_PLAN.md)
- [Agent compatibility, control boundaries, and handoff contract](docs/AGENT_COMPATIBILITY.md)
- [Conceptual TUI mockup](docs/TUI_MOCKUP.svg)
- [Planned permission-review interaction](docs/TUI_REQUEST_MOCKUP.svg)
- [Actionable backlog and per-task acceptance](task.md)
- [Session checkpoint](docs/WORK_CHECKPOINT.md)
- [Terminal troubleshooting](docs/TROUBLESHOOTING.md)

The section-A companion runtime is implemented; its completion gates and any platform evidence blockers are recorded in [task.md](task.md) and the checkpoint. The next product increment is **E01 — Durable goal and save acknowledgement**, after UX01, without pretending that a session draft is already saved. P01 must retain any unverified native rows, and P02 later proves filesystem/process parity; artwork and provider capability gates are independent.

Platform **targets, not support claims**: macOS arm64/x86_64, Linux x86_64/arm64, and native Windows x86_64 with Windows Terminal/ConPTY (WSL is not a substitute). Windows ARM64 and other variants are unclaimed. P01 records actual minimum OS/toolchain/terminal and native build/test/terminal evidence per row; cross-compilation is not runtime proof. P02 gates filesystem/data-directory and owned process-tree lifecycle after recovery UX. P03 gates native archives/checksums and install/upgrade/uninstall preserving user data, after integrated acceptance and rights/provider review. Native-image and agent capabilities are qualified separately by OS/version/terminal.

The [design contract](docs/DESIGN_RULES.md) and [implementation plan](docs/IMPLEMENTATION_PLAN.md) also cover planned transactional persistence, active-run task switching/quit, dispatch-time context revalidation, and E03 archive/delete, redaction, quotas, and retention. Those later workbench safeguards are not implemented by the local companion.

Later integration targets: Codex app-server first, an optional official Claude Agent SDK bridge, and one ACP client exercised against OpenCode and Gemini. A form selects the workflow, not the vendor. Switching agents uses a reviewed fresh handoff; native provider sessions and grants are not portable. The Rust core/local UI needs no Node runtime; only the optional Claude SDK integration would add one. Interface research and a local Codex version/help check are not runtime interoperability verification.

The SVGs are design illustrations, not runtime screenshots. Actual artwork rendering is implemented; external agents remain unimplemented. Performance figures remain scoped acceptance targets, not measured results. Public distribution requires both artwork/name rights review and provider auth/branding/terms review.
