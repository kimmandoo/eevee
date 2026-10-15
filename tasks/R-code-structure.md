# R — Code structure

Done on 2026-10-15 (`refactor(src)`): modules grouped into `store/`, `workspace/`, `providers/`, `platform/`; `store.rs` split by role; inline tests of large files moved into sibling `tests.rs`. Remaining goal: no non-test source file above ~1,200 lines, split along responsibilities rather than line counts.

Rules for every RF task:
- **Behavior-free:** a pure move or extraction, with no logic or visibility widening beyond `pub(super)`. The test count (375 passed, 2 ignored at `d93402a`) must be unchanged.
- **Self-referencing test filters:** grep `'"[a-z_:]*tests::'` before moving a module; runner tests launch themselves by test path.
- **Script paths:** `scripts/terminal_smoke.py` copies `platform/terminal.rs` + `platform/input.rs` standalone, so they must keep using `super::input`. `scripts/installation_smoke.py` reads `platform/distribution.rs` and `ui/artwork.rs`.
- **One file per commit:** `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, `cargo test`. Update README's source-layout table when a folder changes.
- **Order:** do RF tasks right before the feature task that touches the same file (e.g. RF06 before PT06B), not as a big-bang pass.

### RF01 — Split `providers/codex.rs` (2,592 lines)

- [ ] Separate edit tooling, model listing and transport.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** `codex/edit.rs` (EditBaseline, digest, read/apply replacement), `codex/models.rs` (ModelListing), `codex/transport.rs` (`run_transport`, about 1,400 lines: extract its per-message handlers first), keeping `codex.rs` as the worker/payload entry point.
- **Accept:** Codex tests and `scripts/planning_smoke.py`/`implementation_smoke.py` (offline cases) pass unchanged.

### RF02 — Split `providers/acp.rs` (2,188 lines)

- [ ] Separate framing, negotiation/permissions, models and the transport loop.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** `acp/rpc.rs` (Framer, rpc id validation, bounds), `acp/permissions.rs` (permission options, selections), `acp/models.rs`, `acp/transport.rs`.
- **Accept:** The ACP tests pass unchanged.

### RF03 — Split `app.rs` (2,328 lines)

- [ ] Split the single `impl App` by concern.
- **Priority:** 1. **Needs:** —. **Builds on:** —.
- **Change:** Keep the state types in `app.rs`; move `impl App` blocks into `app/keys.rs` (key dispatch), `app/overlays.rs` (confirmation/overlay stack) and `app/navigation.rs` (panes/focus/forms). This is the place PT02B's lead-partner accessor will live.
- **Accept:** App tests and `scripts/workbench_smoke.py` pass unchanged.
- **Watch out:** Do this before PT02B.

### RF04 — Split `ui.rs` (1,974 lines)

- [ ] Separate layout helpers from surfaces.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** `ui/layout.rs` (shell, centered, inner, grid_cell, occlusion), `ui/workspace.rs` (workspace/inspect/repository), `ui/chrome.rs` (header, footer, selection line, confirmation), with `Paint` staying in `ui.rs`.
- **Accept:** UI tests and the workbench smoke pass unchanged.

### RF05 — Split `app/planning.rs` (1,866 lines)

- [ ] Separate the planning picker, review and journal state.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** Split along the existing picker / review / journal sections.
- **Accept:** Planning tests pass unchanged.
- **Watch out:** Do this before PT06B, which replaces this state with per-partner slots.

### RF06 — Split `workspace/runner.rs` (1,775 lines)

- [ ] Separate spec validation, process ownership, capture and the native transport.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** `runner/spec.rs` (CommandSpec, validate, executable identity), `runner/capture.rs` (Sanitizer, output streams), `runner/native.rs` (the existing inline `native` module), keeping the worker in `runner.rs`.
- **Accept:** Runner tests (including self-spawning fixtures) and `execution_smoke.py` pass unchanged.

### RF07 — Split `providers/claude.rs`, `providers/auth.rs`, `providers/agent.rs`

- [ ] Bring each provider file below the limit.
- **Priority:** 3. **Needs:** —. **Builds on:** —.
- **Change:** One commit per file: claude (sidecar process vs credential routes), auth (per-provider login methods), agent (preflight vs catalogue).
- **Accept:** Tests and `auth_smoke.py` offline cases pass unchanged.

### RF08 — Split `app/execution.rs`, `app/workflows.rs`, `workspace/repo.rs`

- [ ] Bring the remaining files below the limit.
- **Priority:** 3. **Needs:** —. **Builds on:** —.
- **Change:** One commit per file along existing section boundaries.
- **Accept:** Tests and the related smoke scripts pass unchanged.

### RF09 — Split `store/tests.rs` (3,239 lines)

- [ ] Mirror the store submodules in tests.
- **Priority:** 3. **Needs:** —. **Builds on:** —.
- **Change:** Turn `store/tests.rs` into `store/tests/{tasks,runs,schema,recovery,memories}.rs` with shared fixtures in `store/tests.rs`.
- **Accept:** The same test count passes; `store::tests::interrupted_migration_process_fixture` keeps a path the self-spawning test can find (update its filter string if it moves).
