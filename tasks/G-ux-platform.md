# G — UX, platform and release

UX03, the four-task Pokémon milestone and A08/UX04A/S04 are recorded in [COMPLETED.md](COMPLETED.md). Source 0.1.27 completed diagnostics, density and reviewed workflow export; public v0.1.26 remains unchanged. The two unfinished extensions below reuse the shipped updater and existing persistence/installation protections.

### P04A — Update review screen

- [ ] Explain an update before replacing anything.
- **Priority:** 2. **Needs:** —. **Builds on:** P02, E01.
- **Change:** Add a TUI entry to the existing checksum-verified command-line updater, showing current/target version, source, sprite-cache effects and retained-data policy. Preserve its ownership and failed-replacement recovery checks.
- **Accept:** Cancel changes no files; an approved update resumes the task; a checksum failure keeps the last usable install.

### P04B — Rollback eligibility

- [ ] Offer rollback only when it is safe.
- **Priority:** 2. **Needs:** P04A. **Builds on:** UX04B, UX04A.
- **Change:** Rollback is offered only when the retained package, SQL schema and strict serialized-format compatibility are established; matching SQL schema numbers alone are insufficient.
- **Accept:** A retained package with established compatibility rolls back successfully. An actual incompatible older writer refuses with byte-identical data, including v0.1.21 against Schema 18's true reduced-motion JSON field and v0.1.26 against Comfortable text density. Unknown compatibility also refuses. Repeat against current persistence when it changes.
- **Watch out:** Existing install replacement recovery is not a schema-aware rollback gate. Never rewrite user data to let an older binary start; restoring a complete backup is a separate explicit recovery operation. No elevation or profile edits.
