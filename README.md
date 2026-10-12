# eevee

A local-first Rust terminal workbench for LLM-assisted development: one persistent partner, nine task-specific forms.

## Run

```sh
cargo run --release
cargo run --release -- --theme light
cargo run --release -- --color 256
cargo run --release -- --ascii --color mono
```

Requires interactive stdin and stdout. Verified with Rust 1.97.1 on macOS arm64. Use `cargo run -- --help` for CLI options.

**Phase 1 is complete:** a runnable local terminal shell with nine form identities, responsive panes, form selection, a command palette, drawers, and a session-local goal editor. Agent execution, repository inspection, persistent tasks, and permission grants are not implemented yet. Switching forms never starts tools or changes permissions.

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

Phase 1 was also exercised as a real subprocess in a PTY at 120x36, 100x30, 70x24, 60x18, and 40x10, including normal/error/panic terminal restoration. See the checkpoint for evidence and the troubleshooting notes for terminal automation details.


## Design

- [Product and visual contract](docs/DESIGN_RULES.md)
- [English implementation plan](docs/IMPLEMENTATION_PLAN.md)
- [Conceptual TUI mockup](docs/TUI_MOCKUP.svg)
- [Implementation progress](task.md)
- [Session checkpoint](docs/WORK_CHECKPOINT.md)
- [Terminal troubleshooting](docs/TROUBLESHOOTING.md)

The SVG is a design illustration, not a runtime screenshot. Performance figures in the design contract remain acceptance targets, not measured results.
