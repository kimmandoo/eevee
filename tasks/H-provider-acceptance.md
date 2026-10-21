# H — Provider acceptance (blocked)

Blocked until the user prepares permitted authentication. Never substitute claude.ai subscription OAuth, read credential stores, or fake provider evidence. Order: A04, A06 → A07 → Q01 → Q02 → P03.

### A04 — Claude Agent SDK interoperability

- [ ] Run the same task UX through the Claude SDK adapter.
- **Needs:** permitted Claude API/cloud or Console credentials. **Builds on:** A03.
- **Accept:** In a disposable workspace: preview/run, answer a question, deny/allow once, interrupt, and resume by explicit session id. No queued follow-up after cancel, no silently saved rule, no double-counted cumulative usage. Hook/MCP boundaries are verified, and the Rust local path works without Node.
- **Status:** The sidecar is implemented (SDK 0.3.296 / CLI 2.1.296 / Node ≥24, strict policy, questions/permissions, explicit resume, Bedrock/Vertex/Console `ant` routes). `--inspect` reports `credential_route: unavailable`; Console login start/cancel smoke passed without sign-in. Evidence: `target/claude-acceptance-inspect.json`, `target/auth-smoke-claude-console`.

### A06 — Gemini ACP conformance

- [ ] Prove the ACP client is not OpenCode-specific.
- **Needs:** completed Gemini authentication and current eligibility. **Builds on:** A05.
- **Accept:** The same prompt/cancel/permission/context scenarios on real Gemini ACP. A capability present in one peer and absent in the other gets a correct reason and is never invoked. Record version, protocol, auth route and restrictions.
- **Status:** Gemini 0.28.0 `--experimental-acp` negotiates ACP1/loadSession, omits `session/list` (not invoked), and returns a real `session/new` 401 on Linux and Windows. Evidence: `target/gemini-acceptance-inspect.json`.
- **Watch out:** A 401 does not mean API keys are the only route.

### A07 — Cross-agent handoff

- [ ] Continue one task across agents without portable sessions.
- **Needs:** A04, A06. **Builds on:** A05, S01.
- **Accept:** Codex → Claude → ACP peer via an accepted handoff; the old session stays available; grants reset; only reviewed context is sent; stale approvals and newer schema are rejected.
- **Status:** Schema-1 JSON/Markdown handoff, idle switching, grant reset and Codex→OpenCode continuation passed; the four provider-free manual cases pass on the current binary.

### Q01 — Complete journey and visual state matrix

- [ ] Prove all nine workflows and the composed UX.
- **Needs:** A07.
- **Accept:** Empty/missing, ready/running, permission/question, cancellation/unknown, save failure, stale evidence, handoff and party states (concurrent partners, approval queue, write lease) across sizes, themes and CJK content. Every controlled action names its enforcement boundary. A naming/art-rights review (including MV06 move names) happens before distribution.
- **Status:** Five native rows passed at `d198fa5`; specialist journeys pass in all themes/layouts; the Codex and Codex→OpenCode live rows pass. Claude/Gemini rows are missing.
- **Move-name inventory:** the 2026-10-21 v0.1.13 distribution review recorded all nine fixed eight-name lists and the no-artwork/no-endorsement boundary in [asset licensing](../docs/ASSET_LICENSES.md#2026-10-21--wave-5-move-name-distribution-review). It did not establish legal clearance or complete Q01's live-provider/whole-product acceptance.

### Q02 — Measured native and token efficiency

- [ ] Measure the performance contract.
- **Needs:** Q01.
- **Accept:** Observations against targets with method and sample size; core, sidecar and agent resources reported separately and in aggregate, including three concurrent partners. Unknown provider usage is shown as unknown.
- **Status:** Local slice measured on an M1 Pro: warm start p95 89.55 ms, idle 0.826% CPU / 7.44 MiB, streaming input p95 12.61 ms, 10 MiB stream bounded to 1 MiB, 10k-line diff 137.66 ms.

### P03 — Native distribution, upgrade and support evidence

- [ ] Reproducible install/upgrade on the supported matrix.
- **Needs:** Q02. **Builds on:** P02.
- **Accept:** Clean-target install without Rust; checksums, help/version, the local flow, and no account/art/network prerequisites. A schema upgrade with resume works; an incompatible downgrade is refused without data loss; uninstall keeps data; per-target evidence and unsigned-artifact disclosure are published.
- **Status:** 0.1.0–0.1.2 ship five native packages. Remaining: integrated/schema/provider acceptance and signing/notarization.
