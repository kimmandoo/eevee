# H — Provider, product and distribution acceptance

Only authenticated provider rows are credential-blocked. Local visual/state checks, resource measurements and native distribution evidence remain independently actionable after the Pokémon milestone. Never substitute subscription OAuth for a permitted SDK route, inspect credential stores or manufacture live-provider evidence. Consult [agent compatibility](../docs/AGENT_COMPATIBILITY.md) for the current interfaces and scoped observations.

### A04 — Claude Agent SDK interoperability

- [ ] Complete authenticated task UX acceptance through the implemented SDK adapter.
- **Needs:** external permitted Claude API/cloud or Console credentials and explicit authorization for live calls. **Builds on:** A03.
- **Accept:** In a disposable workspace: preview/run, answer a question, deny/allow once, interrupt and resume by explicit session id. No queued follow-up after cancel, silently saved rule or double-counted cumulative usage. Verify hook/MCP boundaries; the Rust local path must still work without Node.
- **Status:** Complete bridge 11 (0.1.33+: structured TodoWrite progress, host command MCP transport, taskless model inspection), protocol-1 IPC, Node ≥24, host discovery, mandatory source/write policy, questions/permissions and explicit resume are implemented; bridge 10 and older mixes refuse before dispatch. Recorded inspection observed SDK 0.3.300 / embedded CLI 2.1.300; those are observations, not permanent exact-version admission rules. The 45 policy/production-IPC cases and login-start/cancel evidence are not authenticated acceptance. Recheck current route eligibility when authorized; never claim successful installation authenticated the user.

### A06 — Gemini ACP conformance

- [ ] Complete authenticated conformance through the shared ACP client.
- **Needs:** external completed Gemini authentication, current eligibility and explicit authorization for live calls. **Builds on:** A05.
- **Accept:** Real prompt/cancel/permission/context scenarios; capability mismatch must show a truthful reason without invoking the missing capability. Record actual CLI version, protocol, auth route and restrictions. Exercise mediated client filesystem consent separately from provider-owned external tools.
- **Status:** The current documented invocation is `--acp`. Historical Gemini 0.28.0 `--experimental-acp` negotiated ACP1/loadSession, omitted `session/list` and returned `session/new` 401; it is historical unauthenticated evidence, not a current invocation requirement. Shared ACP file callbacks and local protocol proof do not complete live acceptance.
- **Watch out:** A 401 does not establish that API keys are the only supported authentication route.

### A07 — Cross-agent handoff

- [ ] Complete one task across authenticated agents without portable sessions.
- **Needs:** A04, A06. **Builds on:** A05, S01.
- **Accept:** Codex → Claude → ACP peer through an accepted handoff; the old session remains available, grants reset, only reviewed context is sent, and stale approvals/newer schemas are rejected.
- **Status:** Schema-1 JSON/Markdown handoff, idle switching, grant reset and scoped Codex→OpenCode continuation already passed. Retain that evidence and finish the missing authenticated routes; do not rebuild handoff or pretend sessions are portable.

### Q01 — Complete journey and visual state matrix

- [ ] Close the composed product acceptance matrix with explicit per-row evidence.
- **Needs:** A07 for final composed-provider signoff; local rows can start independently. **Builds on:** UX01, UX02, PT07A, PT07B, PT08A, MV06.
- **Accept:** Empty/missing, ready/running, permission/question, cancellation/unknown, save failure, stale evidence, handoff and party states across sizes, themes and CJK content. Every controlled action names its enforcement boundary. Distinguish current-source local PTY, native-target and authenticated-provider evidence instead of treating one successful row as the whole matrix.
- **Status:** v0.1.29 passed exact-source Check/all-five-native profiles with100 selected journeys; public macOS runtime16 and rapid first-save8 additionally passed. Final Windows evidence retained234 native-cell snapshots with zero orphan wide halves. Public v0.1.34 added downloaded WS20/disconnect20/full-native24 journeys. These are scoped local/native rows, not the entire composed matrix.
- **Refresh for 0.1.35 surfaces:** the 0.1.35 changes alter visible states this matrix owns: `| ` key-hint footers, the 8-command everyday palette, `[Auto]`/`[Pinned]` mode and the `/forms` `a` toggle, `(not connected)`/`(setup)` route labels and the first-run provider list, the compact todo line, the `/todo` panel, auto-adopted todo and default-on prelude, choice-button highlighting and verbose-only disclaimers. Re-capture those rows (both sizes, mono/true-color, CJK) from the released binary; the source-time PTY captures in `target/ux-captures/` are not the final matrix. Historical Codex/OpenCode live rows do not establish missing Claude/Gemini conformance; reconcile remaining states explicitly.
- **Rights:** The [move-name distribution inventory](../docs/ASSET_LICENSES.md#2026-10-21--wave-5-move-name-distribution-review) already covers the fixed nine eight-name lists and no-artwork/no-endorsement boundary. It did not establish legal clearance or full Q01 completion. Review new asset/name changes before their distribution; do not label the existing inventory unperformed.

### Q02 — Measured native and token efficiency

- [ ] Measure current workloads against the performance contract.
- **Needs:** —. **Builds on:** PT07A, R02, T01A.
- **Accept:** Publish methods, raw samples and sample size for startup, input latency, idle, bounded output/diff and three concurrent partners. Report core, sidecar and agent resources separately and in aggregate. Live workload rows require their own permitted authentication/authorization; unknown provider usage remains unknown. Local measurements do not wait for all Q01 rows.
- **Status:** Historical M1 Pro observations were warm-start p95 89.55 ms, idle 0.826% CPU / 7.44 MiB, streaming-input p95 12.61 ms, a 10 MiB stream bounded to 1 MiB and a 10k-line diff in 137.66 ms. They are dated scoped measurements, not current aggregate-resource acceptance. See the current design contract before choosing workloads or thresholds.

### P03 — Native distribution, upgrade and support evidence

- [ ] Close supported-target installation and recovery evidence gaps.
- **Needs:** —. **Builds on:** P02, E02, E03.
- **Accept:** Clean-target installation without Rust; checksums, help/version and local use without account/art/network prerequisites; genuine schema upgrade/resume and incompatible-writer refusal without data loss; uninstall preserving data. Publish per-target evidence and unsigned-artifact disclosure. Identify unsupported/manual rows explicitly rather than inferring actual native TUI behavior from compilation or archive headers.
- **Status:** v0.1.29 source `48ffbb0` passed native run40069352365 and release run40071156552. All eight public assets and five qualified native package identities were verified. Downloaded macOS installation/update/uninstall passed24 composed checks including34 terminal scenarios; genuine public v0.1.28→v0.1.29 Schema18→19 continuity, byte-identical older-writer refusal and current resume passed seven cases. Public v0.1.33 and v0.1.34 repeated the downloaded 24-group/34-scenario macOS installation proof (`target/release-034-install/results.json`); other targets remain CI-package rows, not downloaded-install rows.
- **New upgrade row (0.1.35):** SQL20 is unchanged, but a task saved after `/todo-eager off` writes `progress.eager_off`, which public 0.1.34 strict readers refuse. Prove: 0.1.34 data opens and resumes in 0.1.35 unchanged; default 0.1.35 progress bytes still open in 0.1.34; an opted-out task is refused by 0.1.34 without mutating the database. Preserve that scoped proof; reconcile remaining clean-target/native TUI/manual rows instead of inferring them from archive headers. Packages remain unsigned/not notarized; GitHub reports `immutable:false`, distinct from the no-overwrite release policy.
- **Watch out:** Q02 and provider credentials are not blanket prerequisites for installation/recovery checks. Signing/notarization is not this task's acceptance requirement; unsigned/not-notarized disclosure is. Historical migration evidence must name its actual predecessor/current binaries, not claim a new schema migration in navigation-only v0.1.21.

