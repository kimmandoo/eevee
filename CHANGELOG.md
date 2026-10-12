# Changelog

## 2026-10-13

- docs(distribution): recorded five successful native installation rows, each with ownership/upgrade/failure-preservation checks and 34 installed-binary terminal scenarios; separated the subsequent account-billing CI blocker from code failures.
- feat(storage): persisted project goals and selected forms through a bounded SQLite worker, correlated committed and failed revisions, preserved unsaved edits on failure, and added single-writer exclusion, transactional migration/crash recovery, and read-only export.
- fix(installer): restored the prior Windows executable from its backup even when ReplaceFile reported failure after moving it, and retained recovery files when rollback failed.
- feat(distribution): added Apache-2.0 native verification packages, checksum-first offline POSIX/PowerShell installation, ownership-protected replacement, and installed-binary lifecycle coverage; withheld public Release and remote bootstrap.
- feat(cli): changed the executable and command-line contract to `ev` while preserving project/cache identities.
- docs(platform): recorded completed section-A acceptance and five verified native core baselines, including 34 Windows ConPTY scenarios and qualified graphics/provider/distribution limits.
- fix(windows): repainted state-changed console frames to prevent indexed-color blank cells from retaining prior editor/overlay text, while keeping idle rendering event-driven.
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
