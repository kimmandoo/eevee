# eevee

A local-first Rust terminal workbench for LLM-assisted development: one persistent partner, nine task-specific forms.

## Run

```sh
cargo run --release
cargo run --release -- --theme light
cargo run --release -- --color 256
cargo run --release -- --ascii --color mono
```

Requires interactive stdin and stdout. The last recorded Phase 1 verification used Rust 1.97.1 on macOS arm64; it is historical evidence, not verification of the current working tree's user-owned source/Cargo edits. Use `cargo run -- --help` for CLI options.

**Phase 1 is recorded complete:** a runnable local terminal shell with nine form identities, responsive panes, form selection, a command palette, drawers, and a session-local goal editor. Agent execution, repository inspection, persistent tasks, and permission grants are not implemented yet. Switching forms never starts tools or changes permissions.

Goals exist only until exit, are limited to 4096 UTF-8 bytes, and are never sent to a model. Enter applies an edit to the current session; Esc discards it. Backspace removes a complete Unicode grapheme.

## Keyboard

| Key | Action |
| --- | --- |
| `f` | Open the evolution picker |
| `1`–`9`, Up/Down | Select a form inside the picker; Enter confirms |
| `g` | Edit the session-local goal |
| `i` / `b` | Open the inspector / companion drawer |
| Ctrl+K | Open the command palette |
| Tab / Shift+Tab | Cycle visible panes |
| Enter | Open the focused pane action or confirm a selection |
| Esc | Dismiss an overlay or cancel an edit |
| `?` | Show keyboard help |
| `q` | Quit outside text entry; type a literal `q` inside the editor |
| Ctrl+C | Exit from any view; no background work exists in this phase |

The layout uses three panes at 120+ columns, two at 90–119, and one with drawers at 60–89. Below 60 columns or 18 rows, resize guidance replaces the workspace; Ctrl+C still exits. Rendering is event-driven with no idle animation or timer.

Auto color uses advertised true-color support, then 256-color support, otherwise monochrome. Nonempty `NO_COLOR` overrides color flags. `TERM=dumb` also selects ASCII; a non-UTF-8 locale selects ASCII borders. Dark and light themes are explicit choices.

## Development checks

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo build --release
```

The last recorded Phase 1 checks also exercised a real subprocess in a PTY at 120x36, 100x30, 70x24, 60x18, and 40x10, including normal/error/panic terminal restoration. These results have not been rerun against current user-owned edits. See the checkpoint for evidence and the troubleshooting notes for terminal automation details.


## Design

- [Product and visual contract](docs/DESIGN_RULES.md)
- [English implementation plan](docs/IMPLEMENTATION_PLAN.md)
- [Agent compatibility, control boundaries, and handoff contract](docs/AGENT_COMPATIBILITY.md)
- [Conceptual TUI mockup](docs/TUI_MOCKUP.svg)
- [Planned permission-review interaction](docs/TUI_REQUEST_MOCKUP.svg)
- [Actionable backlog and per-task acceptance](task.md)
- [Session checkpoint](docs/WORK_CHECKPOINT.md)
- [Terminal troubleshooting](docs/TROUBLESHOOTING.md)

Planned next remains **V01** sanctuary layout/hierarchy, then **P01** native portability and the first ready artwork/input tasks: V02A actual assets/cell rendering, V02B native images, V03 cell-capable selection, and UX01 focus/editing/linear layout. UX01 needs V01, not image support; V03 needs V02A, not V02B; E01 durable tasks needs UX01, while P02 verifies storage across completed P01 targets. Missing platform runners, graphics hardware, or optional agents leaves the affected gate pending rather than blocking independent local work; full platform, visual, and integrated acceptance still require all their evidence. All **31** implementation tasks remain pending.

Platform **targets, not support claims**: macOS arm64/x86_64, Linux x86_64/arm64, and native Windows x86_64 with Windows Terminal/ConPTY (WSL is not a substitute). Windows ARM64 and other variants are unclaimed. P01 records actual minimum OS/toolchain/terminal and native build/test/terminal evidence per row; cross-compilation is not runtime proof. P02 gates filesystem/data-directory and owned process-tree lifecycle after recovery UX. P03 gates native archives/checksums and install/upgrade/uninstall preserving user data, after integrated acceptance and rights/provider review. Native-image and agent capabilities are qualified separately by OS/version/terminal.

The [design contract](docs/DESIGN_RULES.md) and [implementation plan](docs/IMPLEMENTATION_PLAN.md) cover planned transactional persistence, safe task switching/quit, dispatch-time context revalidation, and E03 archive/delete, redaction, quotas, and retention. These are planned safeguards, not features of the last-recorded Phase 1 shell.

Later integration targets: Codex app-server first, an optional official Claude Agent SDK bridge, and one ACP client exercised against OpenCode and Gemini. A form selects the workflow, not the vendor. Switching agents uses a reviewed fresh handoff; native provider sessions and grants are not portable. The Rust core/local UI needs no Node runtime; only the optional Claude SDK integration would add one. Interface research and a local Codex version/help check are not runtime interoperability verification.

The SVGs are design illustrations, not runtime screenshots. Actual artwork rendering and external agents are not implemented. Performance figures remain scoped acceptance targets, not measured results. Public distribution requires both artwork/name rights review and provider auth/branding/terms review.
