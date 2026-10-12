# Tasks

## Active task

Implement the Eevee terminal workbench according to `docs/DESIGN_RULES.md` and `docs/IMPLEMENTATION_PLAN.md`.

## Progress

- [x] Write the English plan covering Eevee and all eight canonical evolutions.
- [x] Define role boundaries, evidence, permissions, and curated memory.
- [x] Select Rust/Ratatui and define responsive visual/performance contracts.
- [x] Draw the conceptual TUI in `docs/TUI_MOCKUP.svg` (not a runtime screenshot).
- [x] Phase 1: native shell, terminal lifecycle, all form identities, responsive navigation.
- [ ] Phase 2: durable tasks, repository evidence, Eevee and Vaporeon workflows.
- [ ] Phase 3: controlled execution, approvals, cancellation, Jolteon workflow.
- [ ] Phase 4: real Codex integration, scoped requests, Flareon and Espeon workflows.
- [ ] Phase 5: Umbreon diagnosis, Leafeon refinement, Glaceon review.
- [ ] Phase 6: Sylveon handoff, curated memory, Evolution Stone recipes.
- [ ] Phase 7: all-form end-to-end verification, visual acceptance, performance measurements.

Next action: Phase 2, first increment — durable task creation/resumption and goal storage through a SQLite worker; prove create/edit/exit/reopen in a temporary data directory before implementing repository evidence.

Phase 1 verification: `cargo test --all-targets` (12 passed), `cargo clippy --all-targets -- -D warnings`, `cargo build --release`, and actual PTY interaction across sizes/themes, all nine forms, Unicode editing, and normal/error/panic terminal restoration. This is a complete local-shell increment, not a finished agent workbench.
