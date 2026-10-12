# Work Checkpoint

## 2026-10-12 — Phase 1 native terminal shell

- Active task: implement the Eevee workbench in small, verified increments. Phase 1 is complete; the full agent workbench remains in progress.
- Exact next action: read this checkpoint, `AGENTS.md`, `task.md`, and Phase 2 of `docs/IMPLEMENTATION_PLAN.md`. Implement durable task creation/resumption and goal storage through a SQLite worker as the next increment; verify create/edit/exit/reopen in a temporary data directory before adding repository evidence.
- Implemented: Rust/Ratatui binary, nine canonical form identities and role-specific empty views, responsive three/two/one-pane layout, dark/light/indexed/monochrome/ASCII modes, keyboard palette/picker/help/drawers, grapheme-safe bounded session goal editing, and terminal restoration on normal/error/panic paths.
- Current boundary: only the cwd and session-local state are real inputs. No repository inspection, provider execution, permissions, persistent tasks, or durable memory exists yet. Goals are discarded on exit. Form changes never invoke a model. The blocking terminal event loop has no idle timer; worker events and async coordination are deferred until there is background work.
- Changed files: `.gitignore`, `Cargo.toml`, `Cargo.lock`, `src/main.rs`, `src/app.rs`, `src/config.rs`, `src/domain.rs`, `src/terminal.rs`, `src/ui.rs`, `README.md`, `CHANGELOG.md`, `task.md`, `docs/DESIGN_RULES.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/TROUBLESHOOTING.md`, and this checkpoint.
- Starting state: `main`, latest commit `e77672f docs(eevee): define evolution roles and Rust TUI implementation plan`. The pre-existing untracked `AGENTS.md` was left untouched and excluded from this session's commit.

### Verification

- Toolchain: `rustc 1.97.1`, `cargo 1.97.1`; macOS arm64.
- `cargo build --all-targets`: succeeded after correcting the startup cursor query.
- `cargo fmt --all`: completed.
- `cargo test --all-targets`: 12 passed, 0 failed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo build --release`: passed; the actual release binary started, switched to Glaceon, and exited with terminal restoration.
- Real subprocess PTY interaction: 120x36, 100x30, 70x24, 60x18, and 40x10; all nine forms; confirm/cancel; palette; help; inspector/companion drawers; visible-pane focus; Unicode/CJK/combining input and long bounded input; live resize; quit and Ctrl+C.
- Color checks: dark/light true color, 256-color, monochrome, NO_COLOR override, and TERM=dumb ASCII fallback. Inspected rasterized real PTY captures, not only widget buffers.
- Lifecycle probes: normal return, returned I/O error, and panic restored terminal settings (excluding the macOS kernel-managed PENDIN flag), alternate screen, cursor, and bracketed paste. Panic output followed screen restoration. Non-TTY help/version worked; usage errors returned 2, noninteractive launch returned 1, without terminal escape output.
- Idle observation: no terminal output for two seconds without events. No CPU/RSS/startup/latency benchmark claim is made; Phase 7 performance targets remain unmeasured.
- Local evidence: `target/smoke/results.json` and `.png`/`.txt` terminal captures, including `release-120x36-glaceon.png`, `100x30-light.png`, `70x24-indexed.png`, `60x18-ascii-mono-picker.png`, `60x18-long-unicode-input.png`, and `40x10-too-small.png`. These ignored build artifacts are local evidence, not checked-in product assets.
- Temporary `examples/terminal_probe.rs` was removed after the error/panic smoke; no test-only runtime flags or fake backend were added.

### Issues and blockers

- Fixed unnecessary Ratatui cursor-position querying at startup; documented it and macOS/Pyte automation caveats in `docs/TROUBLESHOOTING.md`.
- No blocker for Phase 2. `/opt/homebrew/bin/codex` exists, but its protocol version, permission mediation, and credentials have not been checked. Those remain Phase 4 prerequisites.
- Rust language-server operations were unavailable in this environment; compiler, tests, Clippy, and real execution provided verification.
- Completion boundary: only Phase 1 is done. Continue the first Phase 2 storage increment; do not infer later-role workflows are implemented from their visible forms.
