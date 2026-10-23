# G — UX, platform and release

UX03 action discovery shipped in v0.1.21; UX04B reduced motion and the four-task Pokémon milestone are implemented for v0.1.22 and recorded in [COMPLETED.md](COMPLETED.md). The five unfinished extensions below reuse the shipped setup, updater and portable-file foundations.

### A08 — Actionable connection diagnostics

- [ ] Turn provider setup failures into one next step.
- **Priority:** 1. **Needs:** —. **Builds on:** A01.
- **Change:** Finish the actionable failure matrix on the existing preflight/setup paths, which already distinguish executable/version, authentication, eligibility and transport states. Reuse their provider-specific next actions and redacted details; do not build another connection system.
- **Accept:** Missing executable, incompatible version, a real auth-required response and a cancelled login each show the blocker and the next action; no tokens or codes appear; a successful install is never shown as authenticated.
- **Watch out:** Never read or copy credential stores, and never switch method/billing silently.

### UX04A — Text density

- [ ] Choose compact or comfortable text density.
- **Priority:** 2. **Needs:** —. **Builds on:** UX01, V01.
- **Change:** A persistent density preference, distinct from the already-shipped responsive layouts and themes.
- **Accept:** Both densities keep permission/error facts, keyboard focus and CJK editing at both sizes.

### S04 — Recipes from reviewed work

- [ ] Save a verified workflow without its authority or secrets.
- **Priority:** 2. **Needs:** —. **Builds on:** S03, R02, C02, MV08B.
- **Change:** Preview a workflow from user-authored steps and real validation commands; edit name, scope and criteria, then export create-only TOML through the existing versioned recipe model. Reuse safe publication and the reviewed portable-export flow delivered with the earlier MV08B milestone rather than inventing another writer.
- **Accept:** Export, reopen and apply in another project; each effect still requires fresh review. Cancel leaves no file, existing files are never overwritten and nothing auto-executes. A saved workflow describes its actual provenance without asserting that every future run is verified.
- **Watch out:** Exclude grants, sessions, credentials, foreign local IDs and machine-specific absolute paths from portable definitions. Do not weaken S03's path/privacy validation for export convenience.

### P04A — Update review screen

- [ ] Explain an update before replacing anything.
- **Priority:** 2. **Needs:** —. **Builds on:** P02, E01.
- **Change:** Add a TUI entry to the existing checksum-verified command-line updater, showing current/target version, source, sprite-cache effects and retained-data policy. Preserve its ownership and failed-replacement recovery checks.
- **Accept:** Cancel changes no files; an approved update resumes the task; a checksum failure keeps the last usable install.

### P04B — Rollback eligibility

- [ ] Offer rollback only when it is safe.
- **Priority:** 2. **Needs:** P04A. **Builds on:** UX04B.
- **Change:** Rollback is offered only when the retained package, SQL schema and strict serialized-format compatibility are established; matching SQL schema numbers alone are insufficient.
- **Accept:** A retained package with established compatibility rolls back successfully. An actual incompatible older writer refuses with byte-identical data, including v0.1.21 against Schema 18's new true reduced-motion JSON field. Unknown compatibility also refuses. Repeat against current persistence when it changes.
- **Watch out:** Existing install replacement recovery is not a schema-aware rollback gate. Never rewrite user data to let an older binary start; restoring a complete backup is a separate explicit recovery operation. No elevation or profile edits.
