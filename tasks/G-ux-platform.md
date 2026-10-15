# G — UX, platform and release

### UX03 — Contextual, searchable actions

- [ ] Make the next useful action discoverable.
- **Priority:** 1. **Needs:** —. **Builds on:** UX01, UX02.
- **Change:** Reuse the `/` and Ctrl+K registry with aliases, current-surface actions and visible disabled reasons (EV03B's owning-form reasons plug in here).
- **Accept:** At both sizes, partial queries find attachment, provider, cancellation and evidence actions; empty/disabled results explain the next step; Escape restores the draft and focus; mutating actions only open a review.
- **Watch out:** No second dispatcher, and no execution on search.

### A08 — Actionable connection diagnostics

- [ ] Turn provider setup failures into one next step.
- **Priority:** 1. **Needs:** —. **Builds on:** A01.
- **Change:** Separate executable/version, permitted authentication, eligibility and transport failures; offer a native setup action and redacted details.
- **Accept:** Missing executable, incompatible version, a real auth-required response and a cancelled login each show the blocker and the next action; no tokens or codes appear; a successful install is never shown as authenticated.
- **Watch out:** Never read or copy credential stores, and never switch method/billing silently.

### UX04A — Text density

- [ ] Choose compact or comfortable text density.
- **Priority:** 2. **Needs:** —. **Builds on:** UX01, V01.
- **Change:** A persistent density preference.
- **Accept:** Both densities keep permission/error facts, keyboard focus and CJK editing at both sizes.

### UX04B — Reduced motion

- [ ] Turn off animation everywhere, persistently.
- **Priority:** 2. **Needs:** —. **Builds on:** V01.
- **Change:** A persistent reduced-motion preference covering the TUI (PK03) and an explicit installer animation opt-out.
- **Accept:** No animated frames during install/update or in the TUI, with truthful phase output; redirected output stays escape-free.
- **Watch out:** NO_COLOR is not a reduced-motion request. Test a real assistive setup before claiming screen-reader support.

### S04 — Recipes from reviewed work

- [ ] Save a verified workflow without its authority or secrets.
- **Priority:** 2. **Needs:** —. **Builds on:** S03, R02, C02.
- **Change:** Preview a recipe from user-authored steps and real validation commands; edit name, scope and criteria; export create-only TOML via S03's schema. MV08B reuses this export path.
- **Accept:** Export, reopen and apply in another project; fresh review is required; cancel leaves no file; nothing is overwritten or auto-executed.
- **Watch out:** Exclude grants, sessions, credentials and absolute paths unless explicitly reviewed.

### P04A — Update review screen

- [ ] Explain an update before replacing anything.
- **Priority:** 2. **Needs:** —. **Builds on:** P02, E01.
- **Change:** A TUI entry to the checksum-verified updater showing current/target version, source, sprite-cache effects and retained-data policy.
- **Accept:** Cancel changes no files; an approved update resumes the task; a checksum failure keeps the last usable install.

### P04B — Rollback eligibility

- [ ] Offer rollback only when it is safe.
- **Priority:** 2. **Needs:** P04A. **Builds on:** —.
- **Change:** Rollback is offered only when the retained package and schema compatibility are established.
- **Accept:** A compatible rollback works; rollback across a newer schema (Schema 7 → ≤0.1.2) is refused with byte-identical data.
- **Watch out:** Never rewrite user data to let an older binary start. No elevation or profile edits.
