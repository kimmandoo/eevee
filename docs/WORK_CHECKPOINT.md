# Work Checkpoint

## 2026-10-12 — Section A companion implementation and native acceptance

- Active work: user requested all of section A, scoped commits, and push. The five companion tasks V01/V02A/V02B/V03/UX01 are implemented and locally verified; **P01** remains active while native CI evidence is collected. Do not start section B during this request.
- Exact next action: commit the native workflow and portable real-terminal harness separately, push the section-A implementation, and observe the five native CI rows. Correct any actual failures, then record per-row outcomes and push the final checkpoint. Do not mark P01 complete from YAML or compilation alone.
- Starting state: `main...origin/main [ahead 5]`; latest commit `9c922f9 docs(platform): plan native support and close UX gaps`. Existing `Cargo.lock`, `Cargo.toml`, `src/app.rs`, `src/config.rs`, `src/domain.rs`, and `src/ui.rs` changes were preserved and integrated into the runnable companion rather than discarded. Pre-existing untracked `.DS_Store` and `AGENTS.md` remain excluded.
- Runtime scope: source-tracked opt-in artwork, bounded static PNG loading/cell cache, qualified native Kitty resources, sanctuary/action hierarchy, all nine deliberate form previews, local-only handoff preview, grapheme cursor editing, portable Ctrl+N/VT Backspace, safe dirty-draft/exit confirmations, stable focus/selection/scroll, and linear navigation. No storage worker, repository reads, grants, commands, or model requests were added.
- Native route: macOS arm64/Ghostty 1.3.1 only, with usable kernel-reported pixel dimensions and no SSH/tmux/screen layer. Native eligibility never reads stdin; a missing report immediately falls back to cells. Other native protocols/OS/version tuples remain disabled until real acceptance.
- Changed runtime files: `Cargo.toml`, `Cargo.lock`, `src/app.rs`, `src/config.rs`, `src/domain.rs`, `src/main.rs`, `src/ui.rs`, `src/terminal.rs`, new `src/artwork.rs`, and new `src/native.rs`. Runtime/contract docs and troubleshooting were aligned; platform CI/harness are a separate commit scope.

### Observed verification

- Rust 1.90.0 minimum toolchain: `cargo +1.90.0 test --offline --all-targets` passed **42** tests; `cargo +1.90.0 clippy --locked --all-targets -- -D warnings` and `cargo +1.90.0 build --release --locked` passed. The lockfile-selected quantette dependency required raising the manifest minimum from 1.88 to 1.90. Formatting used the CI-pinned toolchain.
- Earlier integrated local native harness: **34** real PTY scenarios passed across requested sizes and seven display modes, including normal/error/panic terminal restoration, non-TTY exit codes, CJK paste/controls, selection, modal neutral focus, resize, and idle output. The current release also passed the dedicated artwork/native scenarios below; the workflow will exercise the final revision's complete matrix.
- Explicit CLI download obtained all nine PNGs at PokeAPI sprites revision `0dc105d5b12416932f29b594078282ab4be045f5`; a second invocation verified the complete cache without downloading. Product code/repository contains no franchise image bytes.
- Artwork smoke: **10** scenarios passed, including **63** offline form switches across dark/light/indexed/mono/NO_COLOR/no-pixels/tmux modes, wide/compact portraits, missing/corrupt/oversized asset states, and no terminal graphics queries. `target/artwork-smoke/results.json` records outcomes; actual cell captures were inspected.
- Real Ghostty 1.3.1 macOS arm64 surface: inspected all nine native portraits and dark/light grids, image-free preview/inspector/editor overlays, compact/font resize and restoration, CJK/combining text plus portable multiline input, and exits with no remaining portraits. `target/a-verify/final-native-*.png` are ignored local captures of the actual GUI terminal.
- Measured actual ANSI nonblank text across all nine forms: minimum **7.14:1 dark / 5.19:1 light**, 6,978 text cells sampled per theme, excluding image pixels and decorative borders. `target/a-verify/contrast.json` records the method's sampled counts; theme regression tests also enforce 4.5:1 text and 3:1 active focus.
- Reproduced a FIFO asset blocking before the first frame, then observed visible first-frame output, normal quit, and termios restoration after the nonblocking-open fix. Kept a permanent isolated FIFO regression. Also kept regression coverage for native payload row width and VT Ctrl+H combining deletion.
- Resolved non-trivial native rendering, asset startup, and GUI automation issues are recorded in `docs/TROUBLESHOOTING.md`. No assistive-terminal/screen-reader claim or general performance benchmark claim is made.
- Remaining evidence: other native rows are not yet claimed supported; P01 completion awaits observed CI results. Local screenshots/PNGs stay ignored, and throwaway smoke helpers are removed after final verification.

## 2026-10-12 — Native platform backlog and UX gap closure

- Active work: implement the Eevee workbench incrementally. This request added multiplatform tasks and tightened existing UX/safety acceptance; it changed planning documents only, not application behavior.
- Exact next action: read this checkpoint, `AGENTS.md`, `task.md`, and the incremental delivery section of `docs/IMPLEMENTATION_PLAN.md`; continue **V01 — Sanctuary layout and visual hierarchy** against the existing user-owned working-tree changes. Inspect those changes before editing, preserve them, and verify the actual TUI rather than assuming the historical Phase 1 result covers them.
- Starting state: `main...origin/main [ahead 4]`; latest commit `34ed74e docs(ux): specify agent interoperability and recovery flows`. Pre-existing modified `Cargo.lock`, `Cargo.toml`, `src/app.rs`, `src/config.rs`, `src/domain.rs`, and `src/ui.rs`, plus untracked `.DS_Store` and `AGENTS.md`, were not edited or included in this documentation commit.
- Backlog: **31** unchecked implementation tasks. Added P01 native core/terminal baseline, P02 native filesystem/process parity, P03 installation/upgrade/distribution, and E03 archive/purge/privacy/quotas. Next remains V01, then P01 and the first dependency-ready unblocked task. No implementation task was completed by this planning request.
- Planned native targets: macOS arm64/x86_64, Linux x86_64/arm64, Windows x86_64. Native Windows Terminal/ConPTY is required; WSL and cross-compilation are not native runtime proof. Windows ARM64/other variants remain unclaimed. Record tested minimum OS/toolchain/terminal versions and keep core, graphics, provider, and package evidence separate.
- Dependency correction: UX01 requires V01 rather than V03; V03 requires V02A rather than V02B; E01 requires UX01 without waiting for unavailable P01 native runners. P02 still requires completed P01 and full native storage/process acceptance. Missing hosts, native graphics, or optional agents leave their gates pending without preventing independent local progress; complete visual/integrated/distribution gates are not waived.
- UX/safety additions: dirty-draft discard and below-minimum-size escape paths; transactional/versioned storage and writer conflicts; revision-safe save acknowledgments; task-switch/quit ownership and bounded cancellation; archive versus confirmed purge and retention limits; no undrained-output deadlock at quota exhaustion; native argv/shell/tree semantics; per-OS agent readiness and dispatch-time context revalidation. Later-feature checks are assigned to C02/R01/R02/S02/A02/Q01, not impossible prerequisites for initial local storage.
- Changed files: `task.md`, `README.md`, `docs/DESIGN_RULES.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/AGENT_COMPATIBILITY.md`, `CHANGELOG.md`, and this checkpoint.

### Verification in this planning session

- Ran an in-memory Python validator through Eval against the actual Markdown: 31 unique unchecked tasks, 76 dependency edges, all references to earlier existing tasks, acyclic graph, and Change/Accept/Depends sections for every task.
- Matched the implementation-plan delivery map to all 31 task IDs exactly; validated 23 local Markdown links and anchors across the five planning/contract documents.
- Executed a dependency-readiness simulation: the normal sequence reaches all 31 tasks from V01. With P01, V02B, A04, A05, and A06 blocked, local input, cell picker, storage, retention, source context, execution, and recovery still become reachable, while P02/P03/Q01/Q02 remain pending. This validates planning progression, not application runtime.
- No application build/test/TUI smoke was run: this session changed only Markdown and preserved pre-existing source/Cargo work. Last recorded runtime verification remains Phase 1 on macOS arm64; other OS targets, current user changes, artwork, and providers are not newly verified.
- No non-trivial implementation/environment issue was encountered or resolved; `docs/TROUBLESHOOTING.md` was intentionally unchanged. No temporary scripts or fixtures were created.
- Blockers: none for the documentation request or continuing V01. Future platform gates need native hosts/runners and terminal evidence; graphics and provider gates retain their real-terminal/installation/credential prerequisites.

## 2026-10-12 — Detailed UX and multi-agent compatibility plan

- Active work: implement the Eevee workbench incrementally. This request refined the English plan, interaction contract, compatibility boundaries, and acceptance backlog; it did not implement an adapter or complete a runtime task.
- Exact next action: read this checkpoint, `AGENTS.md`, `task.md`, and the incremental delivery section of `docs/IMPLEMENTATION_PLAN.md`; implement **V01 — Sanctuary layout and visual hierarchy** in the existing TUI. Preserve its honest Phase 1 state, reserve the actual-artwork area, and verify real terminal layouts/themes. Do not jump to agent integration because its plan is detailed.
- Current order: V01 -> V02A -> V02B -> V03 -> UX01 -> local persistence/context -> controlled execution and UX02 recovery -> A01-A07 interoperability -> specialist workflows -> integrated acceptance. All **27** implementation tasks remain unchecked; Phase 1 remains the only completed application phase. Previous checkpoint next actions are historical.
- UX contract: independent task/request/storage/evidence states; keyboard-first focus restoration and CJK editing/paste; separate draft/preview/run; neutral initial approval focus; correlated permission/questions; no silent retries; explicit cancelling/unknown outcomes; save-failure recovery; linear text layout and measured contrast. Real assistive-terminal verification is required before a screen-reader claim.
- Concept: actual Eevee/evolution images remain the companion identity; role and agent are independent. Keep one task agent by default, no nine-process swarm, no model calls for form selection, no fake progress or completion. The inspector/request preview exposes model and actual control/configuration limits.
- Interoperability decisions: Rust-owned Codex app-server first; optional pinned TypeScript Claude Agent SDK bridge for its documented interactive API; one negotiated ACP v1 client exercised against OpenCode and Gemini; manual reviewed handoff for unsupported CLIs. Keep the native local shell independent of Node. A07's handoff schema is reused by Sylveon, not duplicated.
- Safety/ownership limits: approval callbacks are not universal interception or OS isolation. Selected excerpts describe Eevee-sent context, not every provider-added instruction/file. Provider session IDs/grants do not migrate between agents; a handoff starts a fresh reviewed session. Keep auth/billing/config trust explicit, including provider-owned persistent effects. Claude SDK subscription-login restrictions and Codex app-server distribution terms require review; no credential copying or implied entitlement.
- New reference: `docs/AGENT_COMPATIBILITY.md` records primary sources, documented versus observed behavior, per-provider caveats, request/event/accounting contracts, and the shared real-agent conformance gate.
- Changed files: `README.md`, `CHANGELOG.md`, `task.md`, `docs/DESIGN_RULES.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/AGENT_COMPATIBILITY.md`, `docs/TUI_MOCKUP.svg`, `docs/TUI_REQUEST_MOCKUP.svg`, and this checkpoint. No Rust source, dependency, or application behavior changed.
- Starting state: `main...origin/main [ahead 3]`; latest commit `326e637 docs(design): plan actual evolution artwork and incremental UX tasks`. Pre-existing untracked `.DS_Store` and `AGENTS.md` were left untouched and excluded from the session commit.

### Verification in this planning session

- `codex --version`: `codex-cli 0.147.0`. `codex app-server --help`: experimental app-server, stdio default, schema-generation commands, and strict-config support. These are CLI help observations, not a successful protocol handshake.
- `command -v claude`, `command -v gemini`, and `command -v opencode`: none found on the current PATH. No install/login/configuration change or paid/model request was attempted.
- Inspected release-matched Codex protocol/source, official Claude SDK/CLI/auth/permission documentation, ACP v1, OpenCode ACP, Gemini ACP, and MCP architecture. Documentation research does not establish runtime support; exact sources and version caveats are linked in the compatibility contract.
- In-memory JavaScript validation checked 27 unique pending task IDs, dependency existence/order/acyclicity, per-task change/acceptance sections, and 22 local Markdown links including the compatibility anchor. Final dependency alignment reused A07 in S01.
- Browser-rendered both static SVGs in dark and light previews, using the cached actual-source 475x475 Eevee PNG in the home portrait. No SVG parse errors or text outside the viewBox; approval initial focus was Details. Inspected both screenshots and closed the managed browser tab.
- Preview-only contrast check: minimum text contrast 7.89:1 dark / 5.29:1 light; focus accent contrast 11.37:1 / 6.75:1 against the preview canvas. These theme-aware browser values do not prove current Rust theme or arbitrary terminal palette contrast.
- Local ignored visual evidence: `target/design-artwork/ux-compatibility-dark.png` and `target/design-artwork/ux-compatibility-light.png`. Committed SVGs remain static/theme-variable-only and embed no franchise image bytes. The browser study is not a real TUI, native graphics, accessibility, or interoperability smoke.
- No application build/test/smoke was rerun: only plans and SVG assets changed. The prior Phase 1 evidence below remains the last runtime verification. No new non-trivial implementation/environment issue was resolved; the existing troubleshooting record is unchanged.
- Blockers: none for V01. Later native-image acceptance requires a graphics-capable terminal. Later adapter gates need real compatible installations and permitted credentials; auth, approvals, cancellation, and cross-agent handoff remain unverified until those tasks run.

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
