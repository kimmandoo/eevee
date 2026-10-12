# Troubleshooting

### [2026-10-13] Windows console reflow retained stale editor cells

- **Context / Symptom:** Native runs `38261999485` and `38265625588` failed strict help/back surface equality after 120×36 -> 39×10 -> 120×36 resizing; captures retained editor borders/footer characters and displaced the space after a decomposed combining glyph.
- **Root Cause:** Windows' legacy console cell output and reflow did not preserve the sparse renderer's assumed cell positions. Ordered ConPTY resize notifications and bounded output settling corrected the capture model but were insufficient to fix the actual stale cells.
- **Solution:** Composed displayed cell graphemes on Windows without rewriting draft text, and invalidated the terminal diff buffers for every native resize. Native run `38268855355` then passed the complete first interactive scenario: Unicode/literal paste, draft preservation, strict overlay/back equality, nine-form selection, confirmations, idle output, and clean process/console-mode exit.
- **Prevention / Reference:** Keep exact overlay restoration comparisons and actual native resize captures. Never infer a rendering fix from a text-only parser adjustment or silently normalize stored user text.

- **Indexed recurrence:** User-reported `CJK/combining paste not visible` also occurred in run `38271337325` after eight dark/light Windows cases passed. The indexed draft retained all 29 UTF-8 bytes; captured spaces contained letters from the old underlay. This was a distinct output failure with the same generic assertion, not proof of a repeated old input bug. The Windows renderer now repaints every state-changed frame instead of trusting sparse blank-cell projection; idle remains event-driven. Full native verification is required before claiming the recurrence resolved.

### [2026-10-13] ConPTY did not relay alternate-screen escape markers

- **Context / Symptom:** After the corrected Windows interactive scenario exited with status 0 and identical console-mode bits, the generic VT assertion failed with `alternate screen was not left`; its transcript contained cursor/paste restoration but no CSI 1049 entry or exit.
- **Root Cause:** ConPTY projected native console buffers into VT output without relaying the alternate-screen marker expected by the POSIX harness. Absence of an external marker did not prove which native buffer was active.
- **Solution:** Added a native primary-buffer sentinel in the isolated ConPTY console, observed it hidden during the child, and required the exact sentinel to be restored through a fresh `CONOUT$` handle after normal/error/panic exit. Kept mode, cursor, and paste assertions; POSIX still requires the VT alternate-screen exit marker. Full native verification remains required.
- **Prevention / Reference:** Verify native state at the same console boundary used by the program; do not replace state restoration with a missing/not-throw assertion.

### [2026-10-12] Native Windows input lost combining text and paste boundaries

- **Context / Symptom:** Native CI run `38255345859` passed four POSIX hosts but failed Windows ConPTY with `CJK/combining paste not visible`: the editor showed `cafe` rather than `café`. The original Unicode payload was present in the recorded input events.
- **Root Cause:** Crossterm 0.29 represented genuine Alt-code text on VK_MENU release as a release event, which the app correctly ignored for ordinary releases. Its Windows console source also did not emit bracketed `Event::Paste`, so accepting all releases would neither prevent duplicate keys nor preserve pasted controls/newlines.
- **Solution:** Added one Windows console-record reader, enabled VT input, decoded bounded bracketed-paste envelopes before navigation, accepted only provenance-qualified Alt-code release text, and joined UTF-16 surrogate pairs. Repeated records retained repeat semantics; original input-mode bits were captured and restored after raw-mode cleanup. Native CI must verify the correction; cross-target compilation is not runtime proof.
- **Prevention / Reference:** Kept literal-paste, Unicode CSI, Escape/modifier, and UTF-8-boundary/drain regressions. The real ConPTY harness retained the original combining-text assertion and console-mode before/after checks.

### [2026-10-12] macOS could not compile Windows ring without an MSVC SDK

- **Context / Symptom:** Full `cargo check --target x86_64-pc-windows-msvc` on macOS failed in ring's C compilation because `assert.h` was unavailable.
- **Root Cause:** Installing Rust's Windows standard library did not supply Microsoft's native C headers/toolchain.
- **Solution:** Type-checked and linted the actual Windows input module in a temporary dependency-minimal package; retained full native Windows CI for the real build and ConPTY acceptance. Did not remove TLS, weaken dependencies, or claim a full Windows cross-build.
- **Prevention / Reference:** Use the native `windows-2022` workflow row for full Windows evidence; isolated checks only validate Rust/API types.

### [2026-10-12] A named-pipe asset blocked startup before the first frame

- **Context / Symptom:** A real PTY launch with `133.png` as a FIFO stayed alive with zero startup output while no writer existed.
- **Root Cause:** Opening the path with a blocking `File::open` happened before checking the opened handle's regular-file metadata. Decode/read byte limits cannot bound a wait inside `open`.
- **Solution:** On Unix, open assets/manifests with `O_NONBLOCK`, then reject every nonregular handle before reading. The opened-handle check also covers path replacement races; ordinary PNG reads retain their byte limits. Added an isolated FIFO regression with no writer.
- **Prevention / Reference:** File-type checks must not themselves require an unbounded open; validate both the I/O boundary and the decoder boundary.

### [2026-10-12] Native image payload width skipped the remaining TUI

- **Context / Symptom:** In real Ghostty 1.3.1, the initial portrait and nine-image picker drew only the rows preceding the first image; later controls were missing and the old workspace remained visible behind the picker.
- **Root Cause:** `ratatui-image` 11.0.0 stored an entire escaped Kitty row, including base64 transmission, in one buffer cell. `ratatui-core` 0.1.2 measured that symbol as ordinary visible text and advanced its diff iterator past subsequent controls. An idle process sample showed the app waiting for input, ruling out a blocked image writer.
- **Solution:** After rendering each native image, set `CellDiffOption::ForcedWidth` on each encoded row's first cell to its physical image width. Keep the library's trailing-cell skip directives. Verified all nine native portraits and the complete picker in the real terminal; added a regression covering adjacent/later text and image removal.
- **Prevention / Reference:** Escape payload byte/display width is not terminal-cell geometry. Recheck the adapter when either pinned Ratatui or image dependency changes; text-only PTYs cannot qualify pixel rendering.

### [2026-10-12] Native VT Backspace arrived as Ctrl+H

- **Context / Symptom:** The real PTY editor smoke failed with `backspace split a combining grapheme`: sending the VT BS byte left the `café` draft unchanged.
- **Root Cause:** Crossterm represented BS as `Char('h')` with Control rather than `KeyCode::Backspace`; the modal editor intentionally ignored control shortcuts. Other terminals send DEL for Backspace.
- **Solution:** Normalize Ctrl+H to the existing grapheme-aware Backspace handler inside the editor only. Kept Control shortcuts suppressed elsewhere; verified the PTY scenario and added a combining-grapheme regression. Added Ctrl+N as a portable newline key because some terminals cannot distinguish Shift+Enter from Enter.
- **Prevention / Reference:** Exercise actual VT/ConPTY byte input in addition to reducer key-event tests.

### [2026-10-12] GUI verification required native Ghostty scripting

- **Context / Symptom:** System Events automation returned `osascript is not allowed assistive access. (-25211)`. `ghostty +new-window` was unsupported on macOS, and tab changes invalidated earlier CoreGraphics window IDs.
- **Root Cause:** GUI accessibility automation was not authorized, and the CLI action was not the macOS window-creation interface. Capturing an old window ID produced stale surfaces rather than the running app.
- **Solution:** Used Ghostty's own scripting dictionary to create isolated verification windows, activate their exact terminal IDs, and send terminal actions without changing accessibility permissions. Looked up the current CoreGraphics window ID by the dedicated verification title before each capture.
- **Prevention / Reference:** Capture only the dedicated app surface and qualify actual terminal/version behavior; do not infer a rendering failure from an inactive sibling tab or stale window snapshot.

### [2026-10-12] Fullscreen startup waited for a cursor-position reply

- **Context / Symptom:** The first real PTY launch exited with `eevee: The cursor position could not be read within a normal duration` before drawing the workspace.
- **Root Cause:** Ratatui 0.30.2 resolved to ratatui-core 0.1.2, whose `Terminal::clear()` first queries and then preserves the existing cursor position. A plain PTY automation driver does not answer the device-status request. Preserving that cursor is unnecessary for a newly entered fullscreen alternate buffer, and the query also introduces an avoidable terminal round trip.
- **Solution:** In `src/terminal.rs`, clear the new backend directly and explicitly set the cursor to `(0, 0)` before the first draw. Keep failures visible and restore terminal modes; do not catch and ignore a cursor timeout. Rebuilt and launched the actual TUI successfully without a cursor-query responder.
- **Prevention / Reference:** For fullscreen initialization, use `Backend::clear()` plus explicit cursor positioning rather than cursor-preserving `Terminal::clear()`. The real PTY lifecycle probes verified no `ESC[6n` request and verified normal/error/panic restoration. This choice is specific to a new fullscreen buffer, not an inline viewport with existing content.

### [2026-10-12] macOS PTY cleanup checks needed a supervising session

- **Context / Symptom:** Reading the slave terminal attributes after its controlling session exited raised `termios.error: (25, 'Inappropriate ioctl for device')`. Fast-exit lifecycle probes also differed only in the terminal's `PENDIN` flag; a probe that waited without draining output timed out. Pyte's display conversion raised `IndexError: string index out of range` after overwriting wide-character continuation cells during a resize/edit sequence.
- **Root Cause:** macOS revokes the controlling PTY when its session leader exits. `PENDIN` is a kernel-managed pending-input flag rather than a user-facing raw/canonical mode. A PTY has bounded output capacity. The installed Pyte display helper did not tolerate retained empty continuation cells after a wide glyph was overwritten.
- **Solution:** The throwaway smoke driver kept a supervising process as the session leader, captured terminal attributes immediately after the application returned, and compared all attributes except `PENDIN`. It drained output while waiting and treated orphan empty continuation cells as blank when exporting the emulated display; valid wide-character continuations stayed zero-width. Normal exit, returned I/O error, and panic restored the terminal, including cursor visibility, bracketed paste, and alternate-screen state. Panic diagnostics appeared after leaving the alternate screen.
- **Prevention / Reference:** Keep application behavior separate from terminal-emulator behavior. Exercise real subprocesses, consume PTY output concurrently, and capture restoration before closing the controlling session. Screen images under `target/smoke/` are rasterized captures of real PTY output, not screenshots of a separate GUI terminal. The temporary lifecycle probe source was removed after verification.

### [2026-10-12] Themed SVG preview was not visually inspectable

- **Context / Symptom:** The static image preview of `docs/TUI_MOCKUP.svg` appeared entirely black/invisible, so it could not be used to assess the updated composition.
- **Root Cause:** The SVG uses CSS theme variables and `currentColor`; the standalone preview did not produce usable contrast. The exact preview-renderer cause was not established, and this was not an application TUI failure.
- **Solution:** Rendered the same SVG inside Chromium with explicit inherited theme variables, then overlaid the actual-source Eevee PNG and displayed all nine species images in a separate picker study. Inspected the resulting composition, confirmed all ten displayed images loaded, and checked SVG text bounds. Reported the static preview mismatch to the tool issue reporter.
- **Prevention / Reference:** Validate theme-dependent design assets in a theme-aware renderer rather than treating an unusable converter image as evidence of an application bug. `target/design-artwork/sanctuary-preview.png` is a local concept preview, not native terminal graphics verification or a checked-in franchise asset.
