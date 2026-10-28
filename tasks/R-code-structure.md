# R — Code structure

The initial store/workspace/provider/platform grouping, RF03 app split and RF05 planning split are complete; see [COMPLETED.md](COMPLETED.md). Remaining work separates actual responsibilities. File size is a diagnostic, not a reason to invent abstraction or duplicate already-shared tooling.

Rules for every RF task:
- **Behavior-free:** preserve the immediately preceding source's behavior, tests and visibility boundaries; do not pin the historical 375-test baseline or remove meaningful coverage to keep a count.
- **Self-referencing fixtures:** find and update exact self-spawn test filters together with module moves. Preserve their real process behavior.
- **Script paths:** check actual source dependencies in terminal/installation drivers before moving modules; keep standalone `platform/terminal.rs` and `platform/input.rs` linkage working.
- **Coherent commits:** one responsibility extraction at a time, with pinned Rust 1.90.0 formatting, locked all-target/all-feature tests, strict Clippy and the affected actual smoke journey. Update README's source-layout table when ownership changes.
- **Order:** just before the feature that genuinely needs the extraction, not a cleanup wave ahead of Pokémon completion. Completed PT06B is not a future prerequisite.

### RF01 — Separate remaining Codex transport responsibilities

- [ ] Separate model listing and transport handlers without duplicating shared file tooling.
- **Priority:** 2. **Needs:** —. **Builds on:** A02, A03.
- **Change:** Extract remaining `ModelListing` and `run_transport` responsibilities into focused modules, keeping the worker/payload entry point clear. Codex already uses `providers::writes` for shared proposals, validation and file tools; do not recreate the obsolete private `codex/edit.rs` plan.
- **Accept:** Codex tests and `scripts/planning_smoke.py`/`implementation_smoke.py` (offline cases) pass unchanged.

### RF02 — Separate remaining ACP protocol responsibilities

- [ ] Separate framing, negotiation/permissions, models and the transport loop.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** Separate framing/RPC identity and bounds, negotiation/permission selections, model handling and transport along existing ownership. Preserve the already-extracted `acp/files.rs` shared filesystem route rather than making another file-policy implementation.
- **Accept:** ACP behavior tests and actual controlled prompt/permission/cancel/filesystem journeys pass unchanged.

### RF04 — Separate remaining UI layout and chrome

- [ ] Separate layout helpers from surfaces.
- **Priority:** 1 (triggered). **Needs:** —. **Builds on:** —. **Needed by:** GL02, GL05.
- **Trigger (2026-10-28):** `src/ui.rs` grew to ~3,000 lines with the 0.1.35 header/status/todo/choice helpers, and the bag and item-notice surfaces would add more chrome into the same file. Do this extraction first, as its own behavior-free commit, before GL02.
- **Change:** `ui/layout.rs` (shell, centered, inner, grid_cell, occlusion), `ui/workspace.rs` (workspace/inspect/repository), `ui/chrome.rs` (header, footer, selection line, confirmation, `choice_text`/`choices`, `todo_line`/`compact_todo`/`composer_todo`), with `Paint` staying in `ui.rs`. Keep the message lint test's `src/ui.rs` path list in sync with the moved files.
- **Accept:** UI tests and the workbench smoke pass unchanged.

### RF06 — Separate runner validation, capture and native ownership

- [ ] Separate spec validation, process ownership, capture and the native transport.
- **Priority:** 2. **Needs:** —. **Builds on:** —.
- **Change:** `runner/spec.rs` (CommandSpec, validate, executable identity), `runner/capture.rs` (Sanitizer, output streams), `runner/native.rs` (the existing inline `native` module), keeping the worker in `runner.rs`.
- **Accept:** Runner tests (including self-spawning fixtures) and `execution_smoke.py` pass unchanged.

### RF07 — Split `providers/claude.rs`, `providers/auth.rs`, `providers/agent.rs`

- [ ] Separate sidecar transport, credential routes, login methods and catalogue responsibilities.
- **Priority:** 3. **Needs:** —. **Builds on:** A01.
- **Change:** Make sequential coherent extractions: Claude sidecar process versus credential routes, provider-specific auth methods, and preflight versus catalogue. Preserve the existing shared policy and credential-ownership boundaries.
- **Accept:** Tests and `auth_smoke.py` offline cases pass unchanged.

### RF08 — Split `app/execution.rs`, `app/workflows.rs`, `workspace/repo.rs`

- [ ] Separate remaining execution, workflow and repository responsibilities.
- **Priority:** 3. **Needs:** —. **Builds on:** R01, S03, C01.
- **Change:** Extract one existing responsibility at a time while preserving its owner and callers; do not split merely to satisfy a line-count target.
- **Accept:** Tests and the related smoke scripts pass unchanged.

### RF09 — Split remaining legacy store tests by responsibility

- [ ] Mirror the store submodules in tests.
- **Priority:** 3. **Needs:** —. **Builds on:** —.
- **Change:** Group remaining task/run/schema/recovery/memory tests with shared fixtures, reusing the already-dedicated move/record/held/ribbon test modules.
- **Accept:** Preserve the immediately preceding meaningful cases and outcomes; `store::tests::interrupted_migration_process_fixture` retains a discoverable exact self-spawn filter, updated together if its path moves. Exercise the actual interrupted-migration scenario.
