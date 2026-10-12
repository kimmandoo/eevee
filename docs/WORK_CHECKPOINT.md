# Work Checkpoint

## 2026-10-12 — Concept refinement and actual species artwork plan

- Active work: implement the Eevee workbench incrementally. The current request refined the product/design/backlog; no application implementation task was completed in this documentation session.
- Exact next action: read this checkpoint, `AGENTS.md`, `task.md`, and the incremental delivery section of `docs/IMPLEMENTATION_PLAN.md`; implement **V01 — Sanctuary layout and visual hierarchy** in the existing TUI, reserving a bounded actual-artwork area. Verify actual terminal layouts and themes, not only the SVG.
- Priority change: V01 -> V02A actual assets/cell rendering -> V02B native terminal graphics -> V03 image-based form selection now precede E01 durable storage. Phase 1 stays complete; all 21 new backlog tasks remain unchecked. The old Phase 2 next action below is historical, not the active instruction.
- User clarification: use actual Eevee and all eight evolution images, not original ASCII characters. Default planned collection is PokéAPI's transparent official-artwork PNGs; character-cell fallback is derived from the same image. Preserve original image colors, transparency, aspect ratio, and species identity.
- Product decisions: one task/one partner, a sanctuary task card, explicit evolution handoffs, evidence journal, and approved habits rather than XP/affection scores. Each form has a signature interaction and observable output; no mandatory nine-agent tour or cosmetic model calls.
- Technical plan: keep Rust/Ratatui; add a compatible pinned ratatui-image version only in V02B. Native protocol support requires actual-terminal checks; bounded probing, off-render decode/encoding, cached assets, image cleanup, and working cells/ASCII/mono/off modes are acceptance requirements.
- Source/rights: inspected PokéAPI's Eevee metadata, sprite repository, image rights notice, and ratatui-image protocol guidance. The source notice retains The Pokémon Company image copyright; opt-in download/caching is not a legal license. Do not commit/bundle franchise bytes by default; review public distribution rights.
- Changed files: `README.md`, `CHANGELOG.md`, `task.md`, `docs/DESIGN_RULES.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/TUI_MOCKUP.svg`, `docs/TROUBLESHOOTING.md`, and this checkpoint. No Rust sources, Cargo dependencies, or runtime behavior changed.
- Starting state: `main`, latest commit `633598d feat(tui): add native evolution workbench shell`; pre-existing untracked `AGENTS.md` remained untouched and outside the commit.

### Verification in this planning session

- Validated 21 unique implementation task IDs, existing/earlier dependency references, an acyclic delivery order, unchecked status, and a concrete change/demo criterion for every task.
- Validated local Markdown links and well-formed static SVG with a 960x560 viewBox and no embedded franchise image bytes.
- Retrieved and fully decoded all nine official-artwork PNGs: IDs 133, 134, 135, 136, 196, 197, 470, 471, and 700, each 475x475. Local hashes/source records are in ignored `target/design-artwork/sources.json`.
- Rendered the sanctuary SVG with inherited theme tokens and actual Eevee art in Chromium, plus a nine-species picker study. Inspected the result; all ten displayed image elements loaded and SVG text bounds stayed inside the viewBox. Closed the managed preview tab.
- Local visual artifact: `target/design-artwork/sanctuary-preview.png`. This is a concept preview using real source artwork, **not a running TUI or native terminal-image verification**. Downloaded preview PNGs are local ignored assets, not checked-in product files.
- No application build/test/smoke was rerun: only planning/design assets changed. The Phase 1 runtime evidence below remains the last application verification.
- Resolved an unusable standalone themed-SVG preview by using a theme-aware browser; documented the limitation and reported it to the tool issue reporter.
- Blockers: none for V01. Later V02B requires access to a graphics-capable terminal for real protocol/cleanup validation; no compatibility claim is made from the browser preview. Later provider prerequisites remain unchanged.

## Previous checkpoint — Phase 1 native terminal shell

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
