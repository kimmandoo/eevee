# A — Party: free form switching and cooperating partners

Remaining party feature. Shipped partner storage, switching, roster, per-partner drafts, concurrency, approvals, write leases and relays are in [COMPLETED.md](COMPLETED.md). Contract: [Party](../docs/DESIGN_RULES.md): at most six partners, three working per task and one writer per task.

### PT11 — Formations

- [ ] Save and reuse a party arrangement.
- **Priority:** Pokémon milestone 1. **Needs:** —. **Builds on:** S03, PT05B, PT07A, PT08A, EV18.
- **Existing foundation:** Strict versioned recipes already preview and stage task plans; they do not save role slots, assign partners or prepare multiple owned drafts.
- **Change:** Save a reusable arrangement with optional role slots, then preview explicit local partner mappings and draft proposals before applying it. Extend the existing recipe model/versioning and owned store path; reuse lead/form and relay guards rather than assigning form fields directly.
- **Accept:** Save, reopen and reapply a three-partner arrangement across restart. Show missing/unmapped slots and refuse duplicate or newly ineligible mappings. Preserve nonempty drafts/carets unless their exact replacement is explicitly confirmed; cancellation or failure leaves no unexplained partial application. Forms and automatic/manual mode remain coherent. No request, command, context selection or permission starts.
- **Watch out:** Revalidate task/partner identity, busy state, write ownership and draft revisions at commit. A formation carries neither provider sessions nor grants; unmapped slots do not silently create partners.
