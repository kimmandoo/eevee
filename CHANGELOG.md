# Changelog

## 2026-10-13

- test(platform): added native ConPTY alternate-buffer restoration checks using preserved primary-screen contents instead of requiring an unrelayed external VT marker.
- fix(windows): composed displayed console graphemes without changing editable text and invalidated reflowed console buffers on resize before repainting.

## 2026-10-12

- test(platform): synchronized bounded ConPTY captures with queued output and ordered resize notifications, retained strict overlay restoration checks, and preserved exact transcript newlines.
- fix(windows): preserved combining/astral text and literal bracketed paste through a bounded single-owner console-input decoder, and restored the exact original input mode on every terminal exit path.
- build(rust): raised the minimum toolchain to Rust 1.90 to match the pinned image-rendering dependency graph.
- perf(ui): reused borrowed goal lines and untruncated header text during redraws while preserving pinned actions and bounded scrolling.
- ci(platform): added pinned native macOS/Linux/Windows build, test, Clippy, and real PTY/ConPTY lifecycle jobs with host-qualified evidence artifacts and owned-process cleanup.
- feat(companion): delivered the sanctuary, pinned nine-species artwork cache, image-derived cells and qualified native Kitty rendering, deliberate grid/list handoffs, linear keyboard navigation, bounded scrolling, and safe Unicode draft/exit interactions; fixed escaped-image row width and nonregular-asset startup hangs.
- docs(platform): planned native macOS, Linux, and Windows build/runtime/distribution gates; expanded the backlog to thirty-one tasks; decoupled local UX and storage from unavailable graphics/platform runners; tightened persistence, task lifecycle, retention, cancellation, and request-preview acceptance.
- docs(ux): specified focus, editing, accessibility, approval, cancellation, and recovery contracts; separated evolution roles from execution agents; planned capability-qualified Codex, Claude Agent SDK, and ACP integration with reviewed cross-agent handoffs and twenty-seven incremental tasks.
- docs(design): refined the companion journey, sanctuary composition, and twenty-one dependency-ordered tasks, including actual species artwork, native terminal images, and source-derived cell fallbacks.
- feat(tui): added the Rust evolution workbench shell with nine forms, responsive elemental themes, keyboard navigation, and Unicode-safe session-local goal editing.
- fix(terminal): avoided cursor-position queries during fullscreen initialization and restored terminal modes on normal, error, and panic exits.
