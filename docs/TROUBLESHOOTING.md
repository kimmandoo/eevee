# Troubleshooting

### [2026-10-15] UX smoke dependencies were unavailable in the system Python

- **Context / Symptom:** The real-PTY UX capture failed with `ModuleNotFoundError: No module named 'pyte'`; installing into the system interpreter was refused with `externally-managed-environment`.
- **Root Cause:** This host uses a distribution-managed Python environment; the smoke harness requires its own pinned `pyte` and `wcwidth` dependencies.
- **Solution:** Created `target/ux-venv` with `uv venv` and installed `pyte==0.8.2 wcwidth==0.2.13` using `uv pip install --python target/ux-venv/bin/python`. Ran terminal verification with that interpreter without changing system packages.
- **Prevention / Reference:** Use an isolated smoke environment; do not override the distribution's externally-managed protection. Rust LSP reference lookup also exited before serving requests on this host; source inspection and compiler/runtime checks were used instead.

### [2026-10-15] Reserved partner drafts could not preserve valid escaped text

- **Context / Symptom:** PT05B's existing maximum-text round-trip failed with `Stored JSON exceeds its bounded size`.
- **Root Cause:** Schema 7 reserved only 64 KiB for all partner drafts, but four valid 4096-byte drafts can JSON-escape to nearly 100 KiB before adding other partners. Reusing that bound would reject previously valid data.
- **Solution:** Schema 8 enlarged the same reserved column through a transactional task-table rebuild, retaining all fields and deleted AUTOINCREMENT identities. The bounded capacity accommodates six complete draft sets without custom encoding or a reduced text limit.
- **Prevention / Reference:** Keep the existing maximum-escaped-text test and migration/old-binary-refusal coverage when changing persisted aggregate bounds.


### [2026-10-15] schema_upgrade_smoke couldn't drive the slash palette

- **Context / Symptom:** `scripts/schema_upgrade_smoke.py` was written against a pre-slash palette and kept failing on the real v0.1.3 binary: `previous executable has no visible action: Acceptance criteria`.
- **Root cause:** Three drifted assumptions: chooser rows render as `> /criteria Acceptance criteria` (slash-prefixed, `║` border); `criteria-edit` is a contextual choice that only exists inside the checklist overlay; and Ctrl+K+Home sent in one write raced the chooser opening, so the retained cursor hid the target row.
- **Solution:** `previous_action` now opens/resets the chooser with Ctrl+K, types the command's slash name into the filter, and requires the selected row to contain `> /<name>` before Enter.
- **Prevention / Reference:** When driving the palette from scripts, filter by typed command name instead of arrowing to labels, and pump between chooser open and filter typing.
### [2026-10-15] Source-packet dedup produced dangling citations

- **Context / Symptom:** A new `same_bytes_as` dedup for overlapping basket captures passed compile and a first-run assembly, but the focused test failed: a covered capture cited another *covered* capture (`same_bytes_as` pointed at an entry with no `text` field), so the referenced bytes were absent from the packet.
- **Root cause:** The first pass assigned each covered capture to the first enclosing capture *seen in input order*, not to a capture guaranteed to carry text. A narrow range pinned before a wider one could become the "representative" of a third range while itself being covered — a citation chain ending in a stub.
- **Solution:** Compute the maximal set first (captures not covered by any other), then assign every covered capture to the earliest *maximal* cover. The merge key stays `(root, path, source_sha256)` plus a byte-containment check, so identical text from different files or revisions never merges and chains are structurally impossible.
- **Prevention / Reference:** `context::tests::overlapping_captures_of_one_revision_deduplicate_but_keep_every_citation` covers the narrow-then-wide ordering, exact-duplicate pinning, and the same-text-different-path case. When adding reference-style packet fields, assert that every citation target is itself a text-bearing entry.

### [2026-10-14] Documented Windows install never started

- **Observed failure:** Following the README on Windows 11 (PowerShell 5.1, every execution-policy scope `Undefined`, i.e. Restricted) failed at `.\install-ev.ps1` with "running scripts is disabled on this system". The offline example also failed: relative `-Archive`/`-Checksums` paths were rejected as non-absolute.
- **Root cause:** The README ran the saved script directly, which the default client policy blocks; CI only invokes it with `-File` under a runner policy. `Get-AbsolutePath` resolved command-line paths against nothing but an absolute root.
- **Correction/proof:** The README runs the installer as a script block (`& ([scriptblock]::Create((irm …)))`), which execution policy does not gate, and documents a process-only `-ExecutionPolicy Bypass` for saved scripts. Command-line paths now resolve against the PowerShell location after validating the user's own components; PATH entries stay strictly absolute. The installer prints the exact user-PATH command. On the reporting machine, a sandboxed `LOCALAPPDATA` online install via the published v0.1.1 script installed `ev 0.1.1` with all nine sprites; offline install with relative paths and the ambiguous `bin. ` rejection were verified with the patched script.

### [2026-10-14] Native source-context smoke failed after dialogue-frame styling

- **Observed failure:** Native run `38500866295` passed SDK policy and all five native builds, then every installed-runtime row stopped at source capture/revalidation. The local actual-TUI reproduction reached exact payload inspection and failed in `panel_text`: `max() iterable argument is empty`.
- **Root cause:** The capture helper recognized rounded/single/ASCII frame corners, but the new active dialogue used double corners. It could not locate the panel; no incorrect payload or dispatch was established.
- **Correction/proof:** Recognize double corners when extracting the actual panel cells, without asserting a border style or changing exact payload/byte-count/source-identity checks. The corrected actual-TUI journey passed all ten context cases, including CJK/CRLF, redaction, independent captures, purge, changed/missing source, link substitution and missing stored artifacts. Eight execution and six recovery cases also passed. Evidence: `target/pre-011-{context-fixed,execution,recovery}`. Fresh native qualification is still required after pushing this correction.

### [2026-10-14] Original sprites still looked like large broken pixels

- **Root cause:** Correct 96×96 source files still passed through the half-cell renderer. Replacing illustrations with sprites did not remove that lossy character-grid conversion, and the old native gate excluded WSL/Windows Terminal.
- **Correction:** Automatic mode negotiates Sixel/Kitty and actual cell-pixel dimensions through the sole input reader, then preserves source pixels at integer scale. No native support/metrics or insufficient space means text, not a silently substituted mosaic. `--images cells` remains an explicit low-resolution option. Use F4 to inspect the actual renderer.
- **Observed surface:** Windows Terminal 1.24.12741.0 on WSL2 reported Sixel and 10×20 cell pixels. Dedicated-window captures show Eevee, all nine evolutions, both light/dark palettes, image-free overlays and restored portraits; the owned windows exited normally. Captures are under `target/windows-terminal-sprites{,-light,-dark}`.
- **Transport/input evidence:** `scripts/graphics_smoke.py --binary target/release/ev --assets-dir target/sprite-ui-smoke/sprites --output target/original-pixel-release` passed native Sixel, native Kitty, unsupported auto, explicit cells and off. Decoded payloads preserved each opaque source pixel at 4×; Kitty RGB was exact and Sixel differed by at most one channel unit due to percentage encoding. Split/late replies did not change the saved draft. PTY encoding evidence is not another GUI terminal certification.

### [2026-10-14] Browser OAuth returns to localhost with connection refused

- **Confirmed integration behavior / limit:** Codex browser login was selected by default even in WSL/SSH. Its callback belongs to the machine/network namespace running the CLI, not necessarily the desktop browser. Eevee's existing native handoff waits for the provider process to exit; it does not stop it merely because a browser opened. Its owned process group is terminated on cancellation, the ten-minute deadline, or native return. A refused browser callback alone does **not** establish that Eevee killed the listener early.
- **Reproduced WSL host diagnosis:** On the observed Windows 10.0.26200 / WSL2 NAT host, a healthy Linux `127.0.0.1:1455` listener received no connections from Windows localhost. Windows socket binds to both IPv4 and IPv6 port 1455 failed with `AccessDenied` / Winsock `10013`. Read-only `netsh interface ipv4 show excludedportrange protocol=tcp` showed the reserved range **1446–1545**, covering Codex's 1455 and fallback 1457. No early provider termination was established. A loopback bridge cannot bind a Windows-reserved port either. The selected resolution leaves reservations, firewall and WSL settings unchanged and uses official device authorization; no bridge is installed.
- **Preferred Codex recovery:** In **`/connect` → Codex → Connect**, choose **device authorization** and review `["login", "--device-auth"]`. This defaults on detected WSL/SSH. The official flow needs no localhost callback, but personal ChatGPT security settings or workspace policy must permit device login. It does not fix account eligibility or unrelated network failures; unavailable device authorization never silently retries browser/API-key login.
- **Explicit browser recovery:** Leave the native terminal login running while completing browser consent. If the browser refuses localhost and the CLI is still waiting, cancel with **Ctrl+C**, then start a fresh reviewed login rather than reusing its callback URL. Use a browser that can reach the CLI's own loopback, restore appropriate Windows↔WSL localhost forwarding, or for SSH establish a loopback-only tunnel from the browser machine using the **actual port printed by the provider** (`ssh -L 1455:localhost:1455 user@remote` for Codex's default). Do not expose the callback listener publicly, disable firewalls indiscriminately, or paste callback URLs/tokens into Eevee or tickets. WSL forwarding is environment-dependent, not universally broken. See [Microsoft's WSL networking guidance](https://learn.microsoft.com/en-us/windows/wsl/networking) and [official Codex remote-login guidance](https://learn.chatgpt.com/docs/auth#login-on-headless-devices).
- **Provider boundary:** OpenCode and Gemini retain their official native chooser contracts; Eevee does not invent device-auth flags for them. Gemini's provider-owned manual-code option can avoid a loopback dependency where its chosen auth route supports it. Claude SDK credentials remain separate. Every subsequent model request still needs independent Run approval.
- **Devin:** `/connect` → Devin ACP → Connect offers browser OAuth (`auth login`) or explicit manual token paste (`auth login --force-manual-token-flow`) for inaccessible loopback. Manual token paste is not device authorization; use only the official native Devin UI. No fallback or task replay. CLI 3000.11.3 reports internal ACP agent version `0.0.0-dev`, validated separately.
- **Regression coverage / commands:** `python3 scripts/auth_smoke.py --fixture --binary target/release/ev --output target/auth-fixture` uses no provider account or internet: a real PTY and local TCP listener exercise default device selection, unavailable-device refusal without fallback, explicitly selected browser login, foreground stdin, a listener kept alive until the fixture callback, post-return status refresh, Ctrl+C process/listener cleanup, empty model/execution journals, and final terminal restoration. This fixture does not prove human OAuth or Windows↔WSL forwarding. The opt-in official initiation/cancellation journey remains `python3 scripts/auth_smoke.py --live --binary target/release/ev --provider-runtime /path/to/pinned/runtime --output target/auth-live`; it suppresses browser launch, isolates provider homes and never completes human consent or saves native auth transcripts. New coverage is supplied for integrated verification, not recorded here as already exercised.
- **Real WSL device-flow proof:** `python3 scripts/auth_smoke.py --wsl-device --binary target/release/ev --codex /path/to/codex --output target/auth-wsl-device` launches the actual pinned Codex 0.152.1 through Eevee with isolated credentials and guest callback port 1455 deliberately occupied. The official device-code prompt was observed, Ctrl+C returned to Connections and reaped the login, no browser fallback/model request/credential file was created, and terminal modes were restored. Native output and the one-time code are not retained. This proves initiation/cancellation without a usable callback port, not human consent or completed account authentication. Codex may report interrupted device polling as a failed operation rather than a cancelled exit; neither is treated as authenticated.
- **Contract evidence:** Codex [0.152.1 login dispatch](https://github.com/openai/codex/blob/rust-v0.152.1/codex-rs/cli/src/main.rs) maps `--device-auth` directly to the device flow, while bare `login` selects browser OAuth. Its [pinned login implementation](https://github.com/openai/codex/blob/rust-v0.152.1/codex-rs/cli/src/login.rs) waits on `server.block_until_done()` and exits with an error on device-flow failure. Eevee retains those literal arguments and does not substitute methods after approval.

### Model picker has no selectable models or reasoning values

- Opening `/model` or `/effort` uses local cached state only. If no local task exists, use `/task-new` or save the Goal first. Choose **Refresh / inspect** or `/models-refresh`, review executable/configuration/network effects, then explicitly Run. Inspection sends no task prompt.
- An unauthenticated Devin 3000.11.3 session can report a current model with an empty option list. That default is not evidence of selectable models or entitlement; connect through the native login and explicitly refresh again. No guessed model list is substituted.
- ACP reasoning options can belong only to the model active during inspection. After selecting another model, refresh the catalogue for that selected model before choosing effort. Unsupported controls show an explanation and reject requests instead of ignoring the selection.
- Claude SDK effort cannot be qualified before a prompt after organization caps, so `/effort` explains its unavailability. Claude model selection remains available through official model listing and effective-model control acknowledgment. Reinstall the optional sidecar for required bridge revision 2; an older bridge is refused before dispatch.

### [2026-10-14] Schema growth changed the quota smoke's payload budget

- **Context / Symptom:** The newly installed schema-6 binary passed installation and the first 20 workbench cases. The fixed 64-KiB quota scenario reclaimed tasks and saved its retained draft, but a subsequent navigation/draft save correctly reported `database or disk is full`.
- **Root Cause:** The old fixture treated total database capacity as payload capacity. New schema pages consumed that fixed total; even its post-reclamation state had insufficient headroom for the next saved state.
- **Solution:** Initialize the actual native schema, measure SQLite page count/size, then allocate a fixed 32-KiB payload budget above that cost. The test still fills the real quota, requires failure with a retained draft, explicitly deletes eligible tasks, retries, preserves an unrelated task and verifies the resumed draft. Production quota limits and error handling are unchanged.

### [2026-10-14] Replacing PATH hid the pinned provider launcher's Node runtime

- **Context / Symptom:** Live implementation acceptance stopped before any model call with `Version executable exited unsuccessfully`. Direct launch reproduced `env: node: No such file or directory`.
- **Root Cause:** The acceptance invocation replaced the host PATH with a minimal list. The isolated official npm Codex launcher requires the existing Node runtime, whose installation was outside those directories.
- **Solution:** Prepend the isolated provider runtime to the existing PATH instead of replacing it. Verified Codex 0.152.1 and Node 24.15.0; the real five-case controlled implementation/denial/journal/local-check/restart journey then passed. No preflight refusal was bypassed, and the standalone local core still requires no Node.

### [2026-10-14] Deleting the first draft before its insert acknowledgement resurrected text

- **Context / Symptom:** Real warm-start measurement typed `x`, deleted it immediately, exited and reopened. The visible editor had been empty, but the next launch restored `x` as a retained draft.
- **Root Cause:** Before the first insert acknowledgement, the task ID was still zero. Returning the editor to its original empty value no longer satisfied the dirty-task guard, so deletion did not advance the revision. The pending older snapshot remained authoritative.
- **Solution:** Treat queued/in-flight initial saves as durable task state for subsequent editor/navigation revisions. A late insert acknowledgement cannot supersede the newer empty editor.
- **Verification:** `clearing_first_draft_before_insert_ack_does_not_restore_deleted_text` failed before the correction and passed after. Actual native performance acceptance then completed all 30 type/delete/exit/reopen startup samples without restoring deleted text; the separate large-diff harness issue did not invalidate those recorded startup observations.

### [2026-10-14] A duplicated writer-lock descriptor outlived store shutdown

- **Context / Symptom:** Concurrent native process/storage tests observed a writer-lock refusal immediately after the storage worker had been dropped and joined.
- **Root Cause:** Closing the owning descriptor does not release an open-description file lock while a duplicate remains open. A concurrent fork can retain that description until exec, despite close-on-exec. A native descriptor-duplication experiment reproduced the lock surviving owner close and releasing only after explicit unlock.
- **Solution:** Close SQLite first, then explicitly unlock the stable sidecar lock during store-state teardown. Do not retry/steal a live writer's lock or weaken conflict detection.
- **Verification:** Added a real descriptor-duplication/store-reopen/save regression. The integrated suite passed 290 tests/two ignored after the shutdown correction. Quota acceptance now derives payload headroom from actual schema page cost rather than assuming schema 6 still fits the older schema's fixed empty-page budget.

### [2026-10-14] Native task-switch smoke raced its own navigation save

- **Context / Symptom:** Run `38425278423` passed all native builds, the three Windows ConPTY regressions, the full installed terminal suite and all four Unix smoke rows. Windows execution acceptance reached its eighth scenario, then remained on task cards with `Finish or retry task/context saves before switching; the current task was retained.`
- **Root Cause:** The harness sent palette/card navigation and the affirmative Enter without observing the intervening navigation-save acknowledgement. Windows correctly refused the task switch while that save was pending; the harness then waited for a switch it had not successfully requested.
- **Solution:** Wait for the actual task-card surface and `Storage: saved`, select the target, wait for its saved state, and only then press Enter. Applied the same prerequisite ordering to source-context task switching. No automatic retry, longer timeout, skipped case, or weakened task-ownership assertion was introduced.
- **Verification:** The corrected harness passed all eight local execution and ten context cases against the actual previous installed binary. The next native run must complete Windows execution, recovery and handoff acceptance; the previous run did not reach the final two suites there.

### [2026-10-14] ConPTY teardown raced asynchronous Windows Job termination

- **Context / Symptom:** Run `38419475447` passed the real ConPTY regressions, all 34 Windows terminal scenarios and installation/workbench/repository/context/execution suites. The abrupt-death recovery case then failed `TerminateProcess` with `WinError 5` after `Session.close()` closed the owning Job.
- **Root Cause:** Job teardown had already begun terminating the child, but its process handle had not yet signalled completion. A second termination request was denied during that interval; an immediate liveness check mistook termination-in-progress for failure.
- **Solution:** Accept the raced access-denied result only after `WaitForSingleObject` observes process completion within five seconds. Other errors and an unsignalled process still fail. Added a real native Job-close/ConPTY-close regression.
- **Verification:** Local six-case execution-recovery smoke passed, including abrupt death/no replay. Run `38425278423` passed all three native Windows ConPTY regressions, including Job-close teardown. Its later task-switch harness race is recorded separately; complete Windows recovery acceptance remains required.

### [2026-10-14] Raw ConPTY child inherited redirected CI standard handles

- **Context / Symptom:** Run `38417411635` failed both native transport regressions before any console output: `GetConsoleMode` raised `OSError: [WinError 6] The handle is invalid`. The child's traceback went directly to the CI log while the ConPTY reader received nothing.
- **Root Cause:** The new `STARTUPINFOEXW` omitted `STARTF_USESTDHANDLES`. With redirected parent streams, child standard handles could retain the runner's pipes instead of being initialized to the attached pseudoconsole. `bInheritHandles=False` alone does not select console standard handles.
- **Solution:** Set `STARTF_USESTDHANDLES` with all three standard-handle slots null, allowing ConPTY to populate console handles without changing or closing the parent's handles. Moved native transport regressions before compilation so this prerequisite fails immediately rather than after packaging.
- **Reference / Verification:** [Microsoft GetStdHandle attachment rules](https://learn.microsoft.com/en-us/windows/console/getstdhandle), [Alacritty's equivalent ConPTY startup isolation](https://github.com/alacritty/alacritty/blob/master/alacritty_terminal/src/tty/windows/conpty.rs). Run `38419475447` passed both real Windows transport regressions and the complete 34-scenario installed terminal suite; local skips were not treated as native proof.

### [2026-10-13] Windows help return capture corrupted UTF-8 border characters

- **Context / Symptom:** Run `38412094582` failed `help back did not restore the invoking surface` at 100×30. `help-back.txt` contained `╰─────────���──────────╯`, wrapped the adjacent border and scrolled the screen; the help overlay itself had closed. The 120×36 journey and initial idle check had passed.
- **Root Cause:** Pinned pywinpty 2.0.15 used winpty-rs 0.4.0, whose native pipe reader called `MultiByteToWideChar` independently for each `ReadFile` chunk. A UTF-8 border split between reads became replacement characters before the harness received a Python string. Splitting `─` (`e2 94 80`) after its first byte reproduces the observed `���` signature. Later waits cannot restore discarded bytes.
- **Solution:** Replaced the harness dependency with `scripts/windows_conpty.py`, a native ConPTY raw-byte pipe transport. The existing incremental UTF-8 decoder now owns decoding on both platforms. Retained exact help-return surface equality, terminal restoration, input and idle assertions. Added real Windows split-byte Unicode, process/resize and cleanup regressions.
- **Verification:** Local macOS terminal smoke passed all 34 scenarios with the integrated reader, and actionlint passed. The two native Windows regressions are explicitly skipped on macOS; hosted Windows verification is required, not inferred from that skip.
- **Reference:** [Pinned lossy decoding implementation](https://github.com/andfoy/winpty-rs/blob/v0.4.0/src/pty/base.rs), [Microsoft ConPTY lifecycle](https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session).

### [2026-10-13] Windows startup frames were mistaken for idle output

- **Context / Symptom:** Native run `38374725065` passed four Unix rows and SDK policy, but Windows installation reached the nested terminal suite and failed with `app emitted output while idle`.
- **Root Cause:** The captured initial ConPTY surface had the saved-storage header but not its footer; a fixed startup delay accepted a partially delivered frame. Separately, passive provider discovery unconditionally invalidated the UI, causing a full Windows repaint even while the connections drawer was hidden.
- **Solution:** Apply discovery results without invalidating hidden UI, wait for the complete saved home surface, and drain the bounded in-flight frame before starting the uninterrupted idle observation. Keep zero-output assertions and capture before/after screens plus the exact unexpected output on failure.
- **Prevention / Reference:** `passive_discovery_only_redraws_the_connections_drawer` covers visible versus hidden discovery. Local macOS installed-package acceptance passed 20 lifecycle cases and all 34 terminal scenarios; fresh hosted Windows qualification is required before release.

### [2026-10-13] Embedded Windows installer read uninitialized script-scoped state

- **Context / Symptom:** Native bootstrap succeeded, but `ev uninstall` failed under StrictMode because `$script:animation` had not been initialized.
- **Root Cause:** `-File` execution and invocation through a created ScriptBlock have different local/script scope boundaries. Initializers used local scope while helper functions explicitly read and wrote script scope.
- **Solution:** Initialize all shared animation and running-owner fields explicitly in script scope. Keep StrictMode and ownership checks enabled.
- **Verification:** Actual Windows PowerShell **5.1.26100.9444**, invoked from WSL, executed the full installer through an embedded ScriptBlock against a hash-owned removal fixture: executable/metadata removed, unrelated data retained, no transaction residue. This isolates the scope correction; native CI still proves real running-binary self-update/uninstall.

### [2026-10-13] macOS installer smoke lost its PTY before restoration inspection

- **Context / Symptom:** The animated bootstrap completed its process lifetime, but post-exit `tcgetattr(slave)` failed with `ENOTTY` on macOS.
- **Cause / Correction:** A directly session-leading shell can acquire and revoke the PTY at exit. The POSIX bootstrap smoke now uses a waiting Python session leader with the real installer shell as its child, matching ordinary shell invocation ownership. It retains the exact post-exit termios, cursor, animation, executable-hash and uninstall assertions; no terminal error is suppressed. Fresh macOS CI must verify the correction.

### [2026-10-13] Windows nested source failed Git inventory membership

- **Context / Symptom:** `native_source_identity_survives_json_without_path_normalization` failed capture of `src\資料.rs` with “Source is now ignored or absent”.
- **Root Cause:** Source visibility compared Git's slash-separated inventory bytes directly with the native Windows path.
- **Solution:** Translate Windows separators only at the literal Git query/membership boundary. Preserve native paths in source identities and JSON; Unix backslashes remain literal filename bytes. Borrow unchanged paths and allocate only when conversion is required.
- **Prevention / Reference:** The existing real-Git capture/JSON regression exercises nested Unicode paths and exact CRLF content; native Windows CI is required to qualify this correction.

### [2026-10-13] Cached smoke installation retained orphaned metadata

- **Context / Symptom:** Run `38367147072` failed the first Linux installation with `orphaned metadata exists ... target/native-smoke/installation/installed 空 格/bin/.ev-install`.
- **Root Cause:** Runtime installation state lived inside the cached Cargo target tree. The pinned rust-cache action recursively removes non-build files while retaining directory structure, allowing an empty ownership directory to survive without its executable.
- **Solution:** Keep all native smoke state and evidence in `$RUNNER_TEMP/ev-native-smoke`, outside Cargo's cache. Updated every installed-binary consumer and artifact-upload path together. Retain the installer's orphaned-metadata refusal; do not erase or adopt unknown installations.
- **Prevention / Reference:** [Pinned cache cleanup implementation](https://github.com/Swatinem/rust-cache/blob/6323deb102c322ba6fcbdcafc7e3dddab59af2b6/src/cleanup.ts); build caching must not restore mutable installation/test state.

### [2026-10-13] Execution warnings and cancellation disappeared in compact terminals

- **Context / Symptom:** Burst output could scroll a transient truncation notice away; at 60x18 a long follow-output header hid the warning. At 39x10 the general resize screen hid owned-work cancellation.
- **Root Cause:** Capture state was represented as ordinary output, and execution confirmations did not participate in the below-minimum layout.
- **Solution:** Added a typed capture-state event and a pinned warning row, rendered compact neutral cancellation, and refused approval below the review minimum. Actual recovery smoke exercised 10 MiB output with a 1 KiB spool, 60x18 warnings, and 39x10 cancellation/approval refusal.
- **Prevention / Reference:** `scripts/execution_recovery_smoke.py`; output truncation and cancellation are safety state, not disposable transcript lines.

### [2026-10-13] Search and nested command overlays lost input ownership

- **Context / Symptom:** Ctrl+K opened the palette while command-output search owned input. Returning through nested form/command overlays could restore an invisible command surface.
- **Root Cause:** The parent global shortcut ran before the search guard, and restoring an overlay frame did not reopen its execution state. Deferred form selection also left its picker open.
- **Solution:** Suppressed palette handling during search/confirmation, restored execution visibility with its frame, and closed the guarded form picker after the transition. Reproduced both focus failures in the real TUI and added actual smoke regressions.
- **Prevention / Reference:** `scripts/execution_smoke.py`; retain the invoking overlay rather than discarding the navigation stack.

### [2026-10-13] Post-commit artifact cleanup reported a fictional rollback

- **Context / Symptom:** Review found that physical spool cleanup failure after committed metadata deletion returned a generic error, leaving callers with obsolete evidence state.
- **Root Cause:** Transactional metadata mutation and fallible filesystem cleanup shared one error result.
- **Solution:** Returned committed metadata plus a typed cleanup warning. A regression exercises committed output purge and task deletion with failing artifact cleanup; UI now refreshes committed state without claiming physical erasure.
- **Prevention / Reference:** `committed_purge_and_deletion_return_metadata_when_artifact_cleanup_fails` in `src/store/tests.rs`.

### [2026-10-13] Smoke key bursts exceeded terminal typeahead capacity

- **Context / Symptom:** Sending 646 Delete events as one burst left 136 command bytes unchanged, causing the next edited command to retain an old suffix.
- **Root Cause:** Treating a long native key-event burst like atomic bracketed paste exceeded the observed input path's capacity.
- **Solution:** Sent Delete events in batches of 32 and waited for the exact remaining byte counter before continuing; retained full command and execution assertions.
- **Prevention / Reference:** `scripts/execution_smoke.py`; use acknowledged key events for editing and bracketed paste only for text.

### [2026-10-13] Credential-prefix matching redacted ordinary task names

- **Context / Symptom:** A direct redactor probe turned `task-card task-navigation mask-value` into `ta[REDACTED] ta[REDACTED] ma[REDACTED]`.
- **Root Cause:** The recognizable `sk-` credential prefix was matched inside ordinary words, risking unwanted task-text changes and rejection of non-secret project paths.
- **Solution:** Required a token-name boundary before prefix detection. The same direct probe preserved all three words; a regression covers ordinary task/project names alongside an actual recognizable credential.
- **Prevention / Reference:** Redaction remains best-effort. Exercise both secret removal and preservation of ordinary text; do not interpret masking as forensic erasure.

### [2026-10-13] Native CI was blocked before runner startup by account billing

- **Context / Symptom:** After successful five-row installation run `38282091481`, run `38285280853` failed every job with zero steps. The check annotation said: `The job was not started because recent account payments have failed or your spending limit needs to be increased. Please check the 'Billing & plans' section in your settings`.
- **Root Cause:** GitHub account billing/spending admission, not compilation, installer behavior, or a test failure.
- **Resolution status:** Historical admission blocker. Run [38354348498](https://github.com/kimmandoo/eevee/actions/runs/38354348498) subsequently started all five native rows; Linux and SDK policy passed, while macOS/Windows exposed real code/fixture failures. Diagnose those step logs rather than attributing current failures to billing. This does not identify which account-setting change resolved admission.
- **Prevention / Reference:** Check-run annotation `114932046529`; [blocked run](https://github.com/kimmandoo/eevee/actions/runs/38285280853). Earlier native installation evidence remains valid only for its own source revision and exercised capabilities.

### [2026-10-13] SQLite no-follow rejected macOS temporary-directory aliases

- **Context / Symptom:** Durable-store integration tests failed to initialize or export with `unable to open database file: /var/folders/.../workbench.sqlite3`.
- **Root Cause:** SQLite's `SQLITE_OPEN_NOFOLLOW` rejects symlinks in ancestor components too; macOS `/var` resolves through `/private/var`. Checking only the final database file did not make that explicit directory alias usable.
- **Solution:** Canonicalized the selected data directory (or read-only database parent) without lossy string conversion, then retained final-file regularity/reparse checks and SQLite no-follow. Alias paths now share the same writer lock. The corrected suite passed 68 tests, including directory-alias writer exclusion.
- **Prevention / Reference:** Keep project identity separate from sanitized display text, and qualify native filesystem behavior rather than applying POSIX pathname assumptions to every platform.

### [2026-10-13] Windows replacement failure can move the old executable

- **Context / Symptom:** Installer review found that the failure path only restored the executable after a successful `File.Replace` return.
- **Root Cause:** Windows documents `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177): replacement can fail after moving the original to the backup name. Treating all exceptions as no-effect could then delete the only old executable with staging cleanup.
- **Solution:** Checked for and restored the backup regardless of the success flag; used a move when the destination was absent and retained staging if rollback failed.
- **Prevention / Reference:** [ReplaceFileW failure states](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew). The regular native locked-file scenario exercises rollback; this rare partial OS failure was identified by static review, not reproduced locally.

### [2026-10-13] Pinned license generator required its CLI feature

- **Context / Symptom:** Installing `cargo-about 0.9.2` without explicit features reported no available binaries.
- **Root Cause:** This release gates its executable behind the `cli` feature.
- **Solution:** Used `cargo +1.90.0 install cargo-about --version 0.9.2 --locked --features cli` locally and in native packaging CI; generated target-qualified license data and complete notices successfully on the local host.
- **Prevention / Reference:** Keep the tool version/features pinned together; notice generation fails packaging on uncovered dependency licenses.

### [2026-10-13] Windows console reflow retained stale editor cells

- **Context / Symptom:** Native runs `38261999485` and `38265625588` failed strict help/back surface equality after 120×36 -> 39×10 -> 120×36 resizing; captures retained editor borders/footer characters and displaced the space after a decomposed combining glyph.
- **Root Cause:** Windows' legacy console cell output and reflow did not preserve the sparse renderer's assumed cell positions. Ordered ConPTY resize notifications and bounded output settling corrected the capture model but were insufficient to fix the actual stale cells.
- **Solution:** Composed displayed cell graphemes on Windows without rewriting draft text, and invalidated the terminal diff buffers for every native resize. Native run `38268855355` then passed the complete first interactive scenario: Unicode/literal paste, draft preservation, strict overlay/back equality, nine-form selection, confirmations, idle output, and clean process/console-mode exit.
- **Prevention / Reference:** Keep exact overlay restoration comparisons and actual native resize captures. Never infer a rendering fix from a text-only parser adjustment or silently normalize stored user text.

- **Indexed recurrence:** User-reported `CJK/combining paste not visible` also occurred in run `38271337325` after eight dark/light Windows cases passed. The indexed draft retained all 29 UTF-8 bytes; captured spaces contained letters from the old underlay. This was a distinct output failure with the same generic assertion, not proof of a repeated old input bug. Fresh state-changed Windows frames fixed the projection while idle stayed event-driven. Native run [38274013013](https://github.com/kimmandoo/eevee/actions/runs/38274013013), revision `9d49447`, passed all 34 Windows scenarios, including indexed, mono/ASCII, linear, and NO_COLOR modes.

### [2026-10-13] ConPTY did not relay alternate-screen escape markers

- **Context / Symptom:** After the corrected Windows interactive scenario exited with status 0 and identical console-mode bits, the generic VT assertion failed with `alternate screen was not left`; its transcript contained cursor/paste restoration but no CSI 1049 entry or exit.
- **Root Cause:** ConPTY projected native console buffers into VT output without relaying the alternate-screen marker expected by the POSIX harness. Absence of an external marker did not prove which native buffer was active.
- **Solution:** Added a native primary-buffer sentinel in the isolated ConPTY console, observed it hidden during the child, and required the exact sentinel to be restored through a fresh `CONOUT$` handle after normal/error/panic exit. Kept mode, cursor, and paste assertions; POSIX still requires the VT alternate-screen exit marker. Native run `38274013013` verified all three exit paths with original `(503, 7)` console modes and preserved primary contents.
- **Prevention / Reference:** Verify native state at the same console boundary used by the program; do not replace state restoration with a missing/not-throw assertion.

### [2026-10-12] Native Windows input lost combining text and paste boundaries

- **Context / Symptom:** Native CI run `38255345859` passed four POSIX hosts but failed Windows ConPTY with `CJK/combining paste not visible`: the editor showed `cafe` rather than `café`. The original Unicode payload was present in the recorded input events.
- **Root Cause:** Crossterm 0.29 represented genuine Alt-code text on VK_MENU release as a release event, which the app correctly ignored for ordinary releases. Its Windows console source also did not emit bracketed `Event::Paste`, so accepting all releases would neither prevent duplicate keys nor preserve pasted controls/newlines.
- **Solution:** Added one Windows console-record reader, enabled VT input, decoded bounded bracketed-paste envelopes before navigation, accepted only provenance-qualified Alt-code release text, and joined UTF-16 surrogate pairs. Repeated records retained repeat semantics; original input-mode bits were captured and restored after raw-mode cleanup. Native run `38274013013` verified literal Unicode paste and restoration in all 34 Windows scenarios; cross-target compilation alone was not used as runtime proof.
- **Prevention / Reference:** Kept literal-paste, Unicode CSI, Escape/modifier, and UTF-8-boundary/drain regressions. The real ConPTY harness retained the original combining-text assertion and console-mode before/after checks.

### [2026-10-12] macOS could not compile Windows ring without an MSVC SDK

- **Context / Symptom:** Full `cargo check --target x86_64-pc-windows-msvc` on macOS failed in ring's C compilation because `assert.h` was unavailable.
- **Root Cause:** Installing Rust's Windows standard library did not supply Microsoft's native C headers/toolchain.
- **Solution:** Type-checked and linted the actual Windows input module in a temporary dependency-minimal package; retained full native Windows CI for the real build and ConPTY acceptance. Did not remove TLS, weaken dependencies, or claim a full Windows cross-build.
- **Prevention / Reference:** Use the native `windows-2022` workflow row for full Windows evidence; isolated checks only validate Rust/API types.

### [2026-10-12] A named-pipe asset blocked startup before the first frame

- **Context / Symptom:** A real PTY launch with `133.png` as a FIFO stayed alive with zero startup output while no writer existed.
- **Root Cause:** Opening the path with a blocking `File::open` happened before checking the opened handle's regular-file metadata. Decode/read byte limits cannot bound a wait inside `open`.
- **Solution:** On Unix, open assets/manifests with `O_NONBLOCK`, then reject every nonregular handle before reading. The opened-handle check also covers path replacement races; ordinary PNG reads retain their byte limits. Added an isolated FIFO regression with no writer.
- **Prevention / Reference:** File-type checks must not themselves require an unbounded open; validate both the I/O boundary and the decoder boundary.

### [2026-10-12] Native image payload width skipped the remaining TUI

- **Context / Symptom:** In real Ghostty 1.3.1, the initial portrait and nine-image picker drew only the rows preceding the first image; later controls were missing and the old workspace remained visible behind the picker.
- **Root Cause:** `ratatui-image` 11.0.0 stored an entire escaped Kitty row, including base64 transmission, in one buffer cell. `ratatui-core` 0.1.2 measured that symbol as ordinary visible text and advanced its diff iterator past subsequent controls. An idle process sample showed the app waiting for input, ruling out a blocked image writer.
- **Solution:** After rendering each native image, set `CellDiffOption::ForcedWidth` on each encoded row's first cell to its physical image width. Keep the library's trailing-cell skip directives. Verified all nine native portraits and the complete picker in the real terminal; added a regression covering adjacent/later text and image removal.
- **Prevention / Reference:** Escape payload byte/display width is not terminal-cell geometry. Recheck the adapter when either pinned Ratatui or image dependency changes; text-only PTYs cannot qualify pixel rendering.

### [2026-10-12] Native VT Backspace arrived as Ctrl+H

- **Context / Symptom:** The real PTY editor smoke failed with `backspace split a combining grapheme`: sending the VT BS byte left the `café` draft unchanged.
- **Root Cause:** Crossterm represented BS as `Char('h')` with Control rather than `KeyCode::Backspace`; the modal editor intentionally ignored control shortcuts. Other terminals send DEL for Backspace.
- **Solution:** Normalize Ctrl+H to the existing grapheme-aware Backspace handler inside the editor only. Kept Control shortcuts suppressed elsewhere; verified the PTY scenario and added a combining-grapheme regression. Added Ctrl+N as a portable newline key because some terminals cannot distinguish Shift+Enter from Enter.
- **Prevention / Reference:** Exercise actual VT/ConPTY byte input in addition to reducer key-event tests.

### [2026-10-12] GUI verification required native Ghostty scripting

- **Context / Symptom:** System Events automation returned `osascript is not allowed assistive access. (-25211)`. `ghostty +new-window` was unsupported on macOS, and tab changes invalidated earlier CoreGraphics window IDs.
- **Root Cause:** GUI accessibility automation was not authorized, and the CLI action was not the macOS window-creation interface. Capturing an old window ID produced stale surfaces rather than the running app.
- **Solution:** Used Ghostty's own scripting dictionary to create isolated verification windows, activate their exact terminal IDs, and send terminal actions without changing accessibility permissions. Looked up the current CoreGraphics window ID by the dedicated verification title before each capture.
- **Prevention / Reference:** Capture only the dedicated app surface and qualify actual terminal/version behavior; do not infer a rendering failure from an inactive sibling tab or stale window snapshot.

### [2026-10-12] Fullscreen startup waited for a cursor-position reply

- **Context / Symptom:** The first real PTY launch exited with `eevee: The cursor position could not be read within a normal duration` before drawing the workspace.
- **Root Cause:** Ratatui 0.30.2 resolved to ratatui-core 0.1.2, whose `Terminal::clear()` first queries and then preserves the existing cursor position. A plain PTY automation driver does not answer the device-status request. Preserving that cursor is unnecessary for a newly entered fullscreen alternate buffer, and the query also introduces an avoidable terminal round trip.
- **Solution:** In `src/platform/terminal.rs`, clear the new backend directly and explicitly set the cursor to `(0, 0)` before the first draw. Keep failures visible and restore terminal modes; do not catch and ignore a cursor timeout. Rebuilt and launched the actual TUI successfully without a cursor-query responder.
- **Prevention / Reference:** For fullscreen initialization, use `Backend::clear()` plus explicit cursor positioning rather than cursor-preserving `Terminal::clear()`. The real PTY lifecycle probes verified no `ESC[6n` request and verified normal/error/panic restoration. This choice is specific to a new fullscreen buffer, not an inline viewport with existing content.

### [2026-10-12] macOS PTY cleanup checks needed a supervising session

- **Context / Symptom:** Reading the slave terminal attributes after its controlling session exited raised `termios.error: (25, 'Inappropriate ioctl for device')`. Fast-exit lifecycle probes also differed only in the terminal's `PENDIN` flag; a probe that waited without draining output timed out. Pyte's display conversion raised `IndexError: string index out of range` after overwriting wide-character continuation cells during a resize/edit sequence.
- **Root Cause:** macOS revokes the controlling PTY when its session leader exits. `PENDIN` is a kernel-managed pending-input flag rather than a user-facing raw/canonical mode. A PTY has bounded output capacity. The installed Pyte display helper did not tolerate retained empty continuation cells after a wide glyph was overwritten.
- **Solution:** The throwaway smoke driver kept a supervising process as the session leader, captured terminal attributes immediately after the application returned, and compared all attributes except `PENDIN`. It drained output while waiting and treated orphan empty continuation cells as blank when exporting the emulated display; valid wide-character continuations stayed zero-width. Normal exit, returned I/O error, and panic restored the terminal, including cursor visibility, bracketed paste, and alternate-screen state. Panic diagnostics appeared after leaving the alternate screen.
- **Prevention / Reference:** Keep application behavior separate from terminal-emulator behavior. Exercise real subprocesses, consume PTY output concurrently, and capture restoration before closing the controlling session. Screen images under `target/smoke/` are rasterized captures of real PTY output, not screenshots of a separate GUI terminal. The temporary lifecycle probe source was removed after verification.

### [2026-10-12] Themed SVG preview was not visually inspectable

- **Context / Symptom:** The static image preview of `docs/TUI_MOCKUP.svg` appeared entirely black/invisible, so it could not be used to assess the updated composition.
- **Root Cause:** The SVG uses CSS theme variables and `currentColor`; the standalone preview did not produce usable contrast. The exact preview-renderer cause was not established, and this was not an application TUI failure.
- **Solution:** Rendered the same SVG inside Chromium with explicit inherited theme variables, then overlaid the actual-source Eevee PNG and displayed all nine species images in a separate picker study. Inspected the resulting composition, confirmed all ten displayed images loaded, and checked SVG text bounds. Reported the static preview mismatch to the tool issue reporter.
- **Prevention / Reference:** Validate theme-dependent design assets in a theme-aware renderer rather than treating an unusable converter image as evidence of an application bug. `target/design-artwork/sanctuary-preview.png` is a local concept preview, not native terminal graphics verification or a checked-in franchise asset.

### [2026-10-15] Runner self-spawning tests failed after moving modules into folders

- **Context / Symptom:** After moving `runner.rs` to `src/workspace/runner.rs`, nine `workspace::runner::tests::*` cases failed at their first assertion while every other test passed.
- **Root Cause:** Those tests re-launch the test binary with a hard-coded test filter string `"runner::tests::native_fixture"`. The module path became `workspace::runner::tests::native_fixture`, so the child ran zero tests and produced no fixture output.
- **Solution:** Updated the filter strings to the new path. `store::tests::interrupted_migration_process_fixture` kept its path because store tests stayed at `store::tests`.
- **Prevention / Reference:** Before moving a module, `grep -rn '"[a-z_:]*tests::' src` to find self-referencing test filters. When extracting an inline `mod tests` into `tests.rs`, keep the original indentation and run `cargo fmt` instead of dedenting by hand, so multi-line string literals stay byte-identical.

### [2026-10-15] Installer always chose the ASCII ball despite a UTF-8 terminal

- **Context / Symptom:** With `LANG=C.UTF-8`, the new pixel-art installer animation still drew the ASCII fallback in a real PTY.
- **Root Cause:** `install.sh` sets `LC_ALL=C` on line 4 for predictable parsing, so the later UTF-8 check always saw `C`.
- **Solution:** Capture `user_locale=${LC_ALL:-${LC_CTYPE:-${LANG:-}}}` before forcing `LC_ALL=C` and decide the ball style from it. UTF-8 bytes still pass through `printf` unchanged under the C locale.
- **Prevention / Reference:** Any terminal-capability decision in the installers must use the user's environment captured before the script normalizes it. Windows PowerShell has the same class of issue with the console code page; `Test-BlockGlyphs` round-trips the glyphs through `[Console]::OutputEncoding` before drawing them.
