# G — UX, platform and release

UX03 action discovery shipped in v0.1.21 and is recorded in [COMPLETED.md](COMPLETED.md). Prioritize the Pokémon-enabling reduced-motion work; general UX/platform extensions follow that milestone. Existing setup and updater paths are foundations, not features to rebuild.

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

### UX04B — Reduced motion

- [ ] Turn off animation everywhere, persistently.
- **Priority:** Pokémon milestone 3. **Needs:** —. **Builds on:** V01, UX01.
- **Existing foundation:** `--no-animation` and unsuitable-terminal fallbacks already suppress TUI activity frames; installers already have static redirected/unsuitable-output routes.
- **Change:** Add a persistent reduced-motion preference and an explicit installer/self-update animation opt-out. Reuse existing frame gates; the preference governs current activity indicators and future PK03 transitions. Keep CLI overrides unambiguous without treating color selection as motion intent.
- **Accept:** Preference survives restart and applies to real activity at both sizes/languages. Install and self-update opt-out show truthful phase output with no animated frames; redirected output remains escape-free. Verify CLI/preference precedence and unchanged consent/input behavior. Complete both TUI and installer slices before marking UX04B done.
- **Watch out:** NO_COLOR is not a reduced-motion request. Test a real assistive setup before claiming screen-reader support; reduced motion alone is not that proof.

### S04 — Recipes from reviewed work

- [ ] Save a verified workflow without its authority or secrets.
- **Priority:** 2. **Needs:** —. **Builds on:** S03, R02, C02.
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
- **Priority:** 2. **Needs:** P04A. **Builds on:** —.
- **Change:** Rollback is offered only when the retained package and schema compatibility are established.
- **Accept:** A retained package with established compatibility rolls back successfully. An actual older writer incompatible with the current Schema 18 store is refused with byte-identical data; unknown compatibility also refuses. Repeat against the current schema when it changes, rather than treating the historical Schema 7 case as current coverage.
- **Watch out:** Existing install replacement recovery is not a schema-aware rollback gate. Never rewrite user data to let an older binary start; restoring a complete backup is a separate explicit recovery operation. No elevation or profile edits.
