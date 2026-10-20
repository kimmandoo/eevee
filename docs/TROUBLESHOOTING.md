# Troubleshooting

### [2026-10-21] Native response identities collided after transport recreation

- **Context / Symptom:** Exact follow-up HTTP payloads were correct, but the current partial/complete answer appeared under the first request's label and replaced its retained display item.
- **Root Cause:** Tool-free transports were recreated between turns, resetting their local generation. Display turn/item IDs used only that generation, so separate owned requests reused the same identity.
- **Solution:** Correlated native display turn/items with the immutable durable work request ID; streaming updates within that request still replace the same item. Provider generation remains transport-local, and no provider session or remote turn is invented.

### [2026-10-21] Automatic refusal disposal hid native scope recovery

- **Context / Symptom:** External credential/endpoint drift correctly sent no HTTP request, but the worker's subsequent “Native preview discarded” message replaced the actionable refusal and `/chat-new` recovery detail.
- **Root Cause:** Both foreground and parked ordinary-send admission canceled a blocked preview automatically, discarding its inspectable blockers.
- **Solution:** Retained the exact blocked preview without dispatch/retry, exposed blocked status and `/approvals` details, and allowed explicit local reset/cancel to dispose of it. Compact unknown-outcome status now retains the complete uncertainty phrase rather than clipping “unknown”.
- **Prevention / Reference:** Keep ordinary-send blocker regressions, exact frozen input, zero inference requests and explicit reset/cancel checks. Navigation metadata revisions are not credential effects; probes compare unchanged task facts, exact drafts/carets, claims and evidence instead of incidental timestamps.

### [2026-10-21] Direct memory selection reopened its own loading gate

- **Context / Symptom:** After explicit memory approval, `/select-memory` left zero selected notes and displayed “Loading approved memories; wait before editing.” The loaded record remained visible.
- **Root Cause:** Global direct actions called `open_overlay(Memory)` even when already in Memory; its load side effect ran before the existing same-overlay return, and the intended action was then rejected as loading.
- **Solution:** Started memory loading only when entering from another surface. Repeated selection/editing acts on the visible loaded record without new reads; ordinary entry still reloads and first-use authoring still waits for owned loading completion.
- **Prevention / Reference:** Regression covers actual selection/unselection and editing at both sizes, exact outgoing selected records, no implicit approval and no new load request.

### [2026-10-21] Terminal native uncertainty incorrectly retained live ownership

- **Context / Symptom:** Every native protocol closed its actual response socket on cancellation and displayed saved partial/unverified output, but explicit follow-ups remained blocked by the earlier unknown outcome.
- **Root Cause:** One sticky flag represented both unknown remote completion/billing and unresolved tool-capable effects. Ownership treated both as ongoing work even after the tool-free local transport and durable claim had settled. The runtime probe also incorrectly demanded the idle label instead of the truthful unknown-outcome status.
- **Solution:** Classified effect uncertainty from the immutable submitted request at terminal handling. Preserved remote-unknown disclosure, informed quit and canonical exclusion while allowing later explicit native sends/task changes after normal settlement. Kept CLI/tool effects blocking across route changes; fixed the probe to require actual socket closure, released claims, saved storage and partial/unknown status.
- **Prevention / Reference:** Never infer uncertainty kind from the currently selected provider, clear it through login, manufacture tool-journal rows, or treat local reader cancellation as proof of remote cancellation.

### [2026-10-21] Authentication fixture lost terminal restoration evidence after exit

- **Context / Symptom:** Both-size foreground login/callback/cancel/no-replay groups passed, but final `Session.wait_exit()` raised `termios.error: (25, 'Inappropriate ioctl for device')` after the application exited.
- **Root Cause:** The fixture exec'd Eevee as the controlling-session leader; macOS revoked the slave before the harness could sample restored termios. Fixed-count Escapes had also returned through a retained-Request review instead of reliably requesting exit.
- **Solution:** Requested structural safe exit with Ctrl+C and added an opt-in POSIX controlling-TTY supervisor for auth fixtures. It execs/reaps the actual application child, retains its real PID/status and keeps the controlling session alive until terminal attributes are sampled and acknowledged. Resize reaches the application group; cleanup reaps the supervisor.
- **Prevention / Reference:** Do not suppress ioctl errors or infer restoration from exit alone. Keep termios, cursor, bracketed-paste, alternate-screen and exit-status assertions alongside real foreground child/socket cleanup.

### [2026-10-21] Stable command cutover left contradictory surface guidance

- **Context / Symptom:** Actual surface audit found Help advertising ordinary sends as advanced reviews, execution recommending source-owned `/search`, Jolteon advertising inspection as approval and compact footers hiding Escape. An execution probe entered Sources instead of output search.
- **Root Cause:** Context-dependent commands had been replaced centrally, but some shared first-action text, footer branches and one smoke consumer still described the old workspace semantics.
- **Solution:** Made Jolteon's primary action direct command authoring, distinguished `/run` inspection from `/execute` permission review and `/search` source filtering from `/output-search`. Updated advanced session guidance and the affected consumer; bounded compact footer variants retained navigation and exact draft/caret return.
- **Prevention / Reference:** Inspect actual wide/compact surfaces as well as command dispatch. Label-copy assertions do not prove that the visible action reaches the right behavior.

### [2026-10-21] Native follow-ups lost conversational roles and ordinary intent

- **Context / Symptom:** Native setup exposed instructions rather than a private credential editor, and ordinary follow-ups were serialized as isolated planning input instead of the preceding user/assistant conversation.
- **Root Cause:** Configuration discovery was the only connection path; request construction reused display-oriented planning text without a canonical successful-turn history.
- **Solution:** Added explicit masked setup and route/model selection. Captured authoritative completed public responses in bounded process-local role-bearing history, froze it with each input and serialized it separately from system/task/source context. Kept ordinary answers distinct from requested plans and managed reviews; excluded partial, reasoning and display text.
- **Prevention / Reference:** Verify exact roles and raw content on real HTTP requests, including cancellation, incomplete output, scope drift, local chat reset and task/partner isolation. Never rebuild conversations from provider-tool journals or UI items.

### [2026-10-21] Private connection navigation lost keys during background saves

- **Context / Symptom:** Actual native setup occasionally skipped private fields after Tab/Ctrl+U; the endpoint contained the model value and the model was blank. The TUI reported that saves or active work had to finish.
- **Root Cause:** The application applied global idle admission to every private-form key, including harmless edits and navigation, while task autosaves briefly remained pending.
- **Solution:** Kept all private typing, paste, navigation and neutral cancellation inside the private editor. Applied saved/idle admission only to deliberate Save or reviewed affirmative Forget; repeats/releases still cannot approve effects.
- **Prevention / Reference:** Retain pending-save input regressions and actual field-focus assertions at both supported terminal sizes. Do not hide lost input with sleeps or retries.

### [2026-10-21] Neutral authentication cancellation returned to the wrong page

- **Context / Symptom:** Reopening the same sign-in method after neutral Back appeared stuck at “Preparing connection review…”, despite a functioning preparation worker.
- **Root Cause:** Cancellation reset Methods to Providers. The next Enter reopened Methods instead of preparing the selected method, while the previous preparing notice remained visible.
- **Solution:** Retained the originating page and selected method, invalidated the canceled generation and cleared its preparing notice. Late worker completion remains unable to authorize or reopen canceled work.
- **Prevention / Reference:** Exercise prepare, cancel, stale completion, fresh preparation and neutral Back before any explicit authorization; never compensate with an extra hidden Enter.

### [2026-10-21] Credential privacy required anchored paths and native ACL checks

- **Context / Symptom:** Review found that a checked credential path could be redirected through a replaced parent link, and POSIX mode checks alone did not establish privacy on macOS ACL-bearing objects.
- **Root Cause:** Path validation and opening were separate operations, and Darwin ACL permissions were not included in the handle-bound privacy snapshot.
- **Solution:** Anchored no-follow reads to the verified destination, checked owner/mode/native ACL through the opened handle and failed closed on unsupported inspection. Removed inherited ACLs only from newly created app-owned objects; never repaired HOME or provider-owned files. Kept failed-create cleanup from deleting existing Windows targets.
- **Prevention / Reference:** Keep credential/link/ACL fixtures isolated from real accounts. Plaintext persistence is explicit, not encryption or keychain storage; cleanup failures must disclose retained bytes.

### [2026-10-21] Saving connection defaults discarded configured reasoning effort

- **Context / Symptom:** Editing native connection settings could drop a previously configured reasoning effort or carry it onto a different model without explaining that association.
- **Root Cause:** The new saved profile was reconstructed from visible fields without preserving the original model/effort binding.
- **Solution:** Preserved the configured binding when the model stayed unchanged, visibly reset it on deliberate model changes and refused mismatched associations at dispatch.

### [2026-10-21] Direct actions lost asynchronous intent and editor return ownership

- **Context / Symptom:** First source actions disappeared while catalogues loaded, specialist launch failures immediately closed, restored tasks returned to another composer's context and model selection disappeared behind required provider-switch review.
- **Root Cause:** Direct navigation reused synchronous workspace assumptions, interpreted the initiating Enter as editor completion and did not bind deferred return/picker intent to its exact task/partner/route.
- **Solution:** Retained owned source actions until catalogue/context readiness, canceled stale intent, captured pre-key specialist editing state, rebound restored-task returns and carried route-bound picker intent only through an accepted matching switch review.
- **Prevention / Reference:** Keep exact Request draft/caret and unchanged-effects assertions; direct entry removes optional navigation, not required approvals.

### [2026-10-21] Compact sheets obscured live context and question content

- **Context / Symptom:** Modal entry covered storage/refusal feedback; long technical correlation IDs appeared before the actual question/options at the minimum terminal size.
- **Root Cause:** Independent sheets did not consistently reserve live header/footer rows, and metadata was ordered ahead of the user's decision.
- **Solution:** Used shared two-header/two-footer sheet bounds, retained actual storage state rather than a stale stamp and moved technical IDs after question/options/answer content. Full details remain keyboard-inspectable without weakening visible-summary approval gates.

### [2026-10-21] Bounded fixture version probe expired during runtime qualification

- **Context / Symptom:** The first E execution-relay run stopped after 10 of 14 groups: the deterministic peer's version probe exceeded the application's three-second deadline at 60×18 (`target/wave4-accepted-execution-relays-e`).
- **Root Cause:** Not established. The fixture's version branch only printed its fixed version; the observation did not establish a production protocol defect.
- **Solution:** Preserved the failure and manually ran the unchanged complete suite in fresh isolated disposable processes; all 14 groups passed on identical E bytes (`target/wave4-accepted-execution-relays-e-isolated`). No provider request retry, harness workaround, admission bypass or timeout change was added.
- **Prevention / Reference:** Keep the bounded fail-closed probe and original evidence. A successful isolated run is not a claim that the timeout's cause was proven.

### [2026-10-21] Compact history hid acknowledged provider coverage loss

- **Context / Symptom:** Three explicit resumed native turns reached the real 256-record journal limit. Terminal-only discard correctly kept committed rows and released ownership, but 60×18 history showed only the positive acknowledgement; the unverified gap was invisible (`target/wave4-cooperation-c-reachable/60-journal-limit/initial/final.txt`).
- **Root Cause:** The bounded history footer applied `brief` to a long storage acknowledgement while the coverage-loss warning was appended afterward.
- **Solution:** Prioritized the existing gap as the compact notice, made its first sentence explicitly say write attempts were not saved and were not verification, and appended its full memory-only owner/request/error detail to the scrollable history presentation. No stored observation was rewritten or invented.
- **Prevention / Reference:** Keep the post-discard visible-warning assertion alongside exact terminal-only recovery, 256 unchanged committed rows, deliberate neutral consent and no provider replay.

### [2026-10-21] Compact command editor hid a correct capacity refusal

- **Context / Symptom:** With three real owned local commands at 60×18, applying a fourth correctly refused execution and retained its draft, but no reason appeared. `target/wave4-accepted-cooperation/60-local-slots/initial/final.txt` preserved the failure.
- **Root Cause:** The centered command editor covered screen rows 1–16, including the global notice row. The next key cleared the old notice before closing the editor, so Escape could not reveal it afterward.
- **Solution:** Used the existing specialist-desk layout boundary: reserve two header and two footer rows before centering the editor. Task/storage/refusal/exit text remains outside the modal; the existing caret-driven input viewport retains the complete draft without changing the capacity guard.
- **Prevention / Reference:** Keep the actual fourth-command visibility assertion at the minimum terminal size; absence of a process alone does not prove an actionable refusal.

### [2026-10-21] Native graphics input fixture reopened the wrong editor

- **Context / Symptom:** The native graphics smoke failed `typed text was lost, duplicated or polluted by terminal replies` after successfully decoding a real sprite.
- **Root Cause:** Its pre-first-use-UX flow typed into the Request composer, then reopened Goal after Help and expected that different field to contain the same text. Terminal replies had not corrupted the input.
- **Solution:** Opened Goal explicitly before typing and resumed that same retained field after Help. All five actual PTY routes then passed input persistence and terminal recovery: Sixel, Kitty, unsupported-native cell fallback, explicit cells and images-off.
- **Prevention / Reference:** `scripts/graphics_smoke.py`; decoded native subject pixels remained at integer scale 3, with maximum RGB error 1 for Sixel and 0 for Kitty. Capability responders are not GUI-terminal certification.

### [2026-10-21] Editing-owner banner painted over local command consent

- **Context / Symptom:** The actual mixed reader/writer/command journey displayed `Ember (Flareon) is editingpermission` where the command permission title belonged. The full review locator failed even though the neutral choices and exact command remained below.
- **Root Cause:** A queue wrapper drew the lease banner after rendering the confirmation, directly on its first content row.
- **Solution:** Removed the post-render wrapper. Queue identity became the panel title; the editing owner used an existing consent-summary row without hiding command identity, directory, executable, effects or invalidated-request errors. The compact six-row approval budget and exact-detail scrolling were preserved.
- **Prevention / Reference:** The actual mixed-work journey reached approval and started its exact owned command after the correction; both-size full cooperation qualification remained required. Deferred command smokes deliberately reopened `/approvals`; native-provider denial used the visible neutral Deny choice, not a nonexistent `/deny` palette command.

### [2026-10-21] Parallel context fixtures collided on a timestamp-only directory name

- **Context / Symptom:** The native integrated suite failed a context fixture at `fs::create_dir` with `AlreadyExists (code 17)` while creating a PID-and-nanosecond-named directory.
- **Root Cause:** Wall-clock timestamps did not guarantee unique names for parallel fixture creation in one process.
- **Solution:** Added a process-local atomic counter alongside the PID and timestamp, preserving canonicalization, isolation and the original parallel test execution.
- **Prevention / Reference:** `workspace::context::tests::Fixture::new`; no production path, guard or timeout was relaxed.

### [2026-10-21] Idle transport retirement lost resumable ownership

- **Context / Symptom:** Cooperative runtime review found that a completed request retained its thread/session reference while the idle provider worker—and its backend ownership registry—was dropped. The next Resume could therefore refuse an otherwise owned session. Parked states could also retain old authentication catalogues or future-launch credentials.
- **Root Cause:** Retirement used only transport activity, and authentication updates reached only the front state.
- **Solution:** Retained bounded owner workers while current or cached resumable sessions existed; propagated account invalidation and future credential replacement/clearing to parked states without mutating submitted immutable input. Revoked sessions no longer kept obsolete workers alive.
- **Prevention / Reference:** Consecutive same-owner readonly turns and parked-authentication regressions; no automatic resend or session adoption.

### [2026-10-21] Full provider journal prevented ownership settlement

- **Context / Symptom:** A definitive provider completion with a failed 257th journal record retained the exact work claim. Storage-only retry could not overcome the 256-record limit, while active ownership correctly refused purge, leaving ordinary settlement impossible.
- **Root Cause:** Failed unsaved observations had no explicit terminal recovery distinct from retry or deletion of committed history.
- **Solution:** Added neutral `/journal-discard` review bound to owner/task/request and every exact failed FIFO write attempt, including repeated IDs/partial/final versions. Only provably unsaved failures after definite termination could be acknowledged; committed rows remained unchanged, an unverified memory-only coverage-gap/error notice remained, and ordinary exact-claim release completed settlement. Running/unknown work and ambiguous commits/reads/purges refused recovery.
- **Prevention / Reference:** Real 257th-record store regression and paced actual protocol fixture; recovery never invented successful persistence, purged history or replayed a provider.

### [2026-10-21] Releasing the current idle lead conflicted with live autosave state

- **Context / Symptom:** Review found that a committed release and subsequent front switch could share one revision with differing draft values. Removed recipient intents also remained in live workflows and could reappear on the next save.
- **Root Cause:** Release completion copied roster/task metadata without reconciling live specialist intents or separating committed cleanup from additional front/navigation changes.
- **Solution:** Paused autosave during the release transaction, applied committed recipient-intent pruning, cleared only the removed lead's specialist proposals, and saved additional live changes strictly above the committed revision while retaining peers, transports, unrelated drafts and carets.
- **Prevention / Reference:** Current-lead release with three working peers, recipient-intent resurrection and preserved-evidence regressions.

### [2026-10-20] Empty cooperation recovery rewrote an idle current-schema database

- **Context / Symptom:** The first integrated wave 4 suite failed `current_schema_reopens_without_rewriting_or_inventing_activity`; reopening and saving the identical task changed the SQLite change counter despite no reserved execution or active work.
- **Root Cause:** Startup recovery unconditionally executed `DELETE FROM active_work`, including an empty table. SQLite still performed a write.
- **Solution:** Checked for either reserved executions or persisted ownership before opening the recovery transaction. A quiescent store now returns through the read-only path; real recovery still interrupts evidence and clears stale claims atomically under the exclusive writer lock.
- **Prevention / Reference:** Retained the byte-preserving reopen regression and real previous-binary refusal qualification. Attribution fixtures were separately aligned with actual roster identities rather than weakening work-owner validation.

### [2026-10-20] Cooperative consent and receipts used mutable or previous-request metadata

- **Context / Symptom:** Read-only integration review found that a deferred request A could display a later saved draft B while dispatching A, a new claim could expose its predecessor's receipt/model, and modified affirmative keys could confirm partner release.
- **Root Cause:** Legacy rendering read the current task draft; receipt fields lacked current-request ownership; release consent tested the key code without the modifier/Press boundary.
- **Solution:** Rendered the immutable submitted snapshot, bound current-request receipt/model disclosure to its identity, and required unmodified affirmative Press events after visible release review. Added regressions for A versus B, independent request receipts and Ctrl/Alt/Shift/repeat/release-key refusal.
- **Prevention / Reference:** Future drafts remain editable, but never replace reviewed input or its attribution. Unknown provider usage/cost must stay explicit, not become an earlier request's measurements.

### [2026-10-20] Compact specialist pagination skipped saved detail lines

- **Context / Symptom:** Final 60×18 inspection exposed only the beginning of the comparison warning. A new keyboard-only regression confirmed that paging skipped several user-authored hypothesis lines entirely.
- **Root Cause:** Page size came from terminal height, not the actual detail viewport. Narrow layouts reserved only three detail rows while PageDown advanced six rows.
- **Solution:** Reported the actual rendered viewport height as ephemeral `scroll_page` state and used it for normal specialist and relay paging. The saved-hypothesis regression failed before and passed after at both sizes; the integrated source passed 713 tests / 2 ignored, strict Clippy and release build.
- **Prevention / Reference:** The actual workflow smoke now inspected both linked run IDs, both factual durations and the no-speedup warning across reachable pages rather than stopping at the first `Single-run` substring. Old selected source captures were deliberately removed through normal reviewed UI before the independent source-free design request; freshness guards were not bypassed.

### [2026-10-20] Smoke isolation invalidated toolchain and overlay assumptions

- **Context / Symptom:** After isolating provider HOME, the Cargo smoke produced no source diagnostic. Fixed-count Escape batches in relay fixtures also left a different overlay or reopened a retained Request editor, so later command text could enter that editor instead of the chooser.
- **Root Cause:** Rustup's default data location followed the disposable HOME, while overlay depth depended on the actual navigation history. A fixed number of Escape presses did not establish a workspace boundary.
- **Solution:** Preserved explicit `RUSTUP_HOME` and `CARGO_HOME` for the real compiler fixture while keeping provider configuration isolated. Unwound observed overlay frames to the workspace footer before deliberate Request editing; checked committed ownership and empty recipient text before relaying. Both-size Cargo/location and owning-form journeys passed without changing production guards or skipping effects checks.
- **Prevention / Reference:** `execution_relay_smoke.py`, `owner_redirect_smoke.py` and the workflow fixture's `workspace`/`clear_request` helpers. Chooser fixtures also reset to the first row before selecting an evidence identity instead of inheriting an old highlight.

### [2026-10-20] Decision criteria preview reset scrolling before confirmation

- **Context / Symptom:** A 40-criterion preview regression could not reach the final criterion at 120×36. Repeated PageDown input left the first items visible.
- **Root Cause:** The compact recipe confirmation layout also applied to decision criteria and reset the shared scroll position on every render.
- **Solution:** Kept the complete criteria preview on the ordinary scrollable detail path while preserving compact recipe consent behavior. The same regression passed at 120×36 and 60×18, with every last item inspectable and no confirmation or provider dispatch.
- **Prevention / Reference:** `every_decision_criterion_remains_inspectable_before_confirmation`; full integrated verification passed 712 tests / 2 ignored plus strict Clippy and release build.

### [2026-10-20] Controlled-provider fixture stalled on coalesced JSONL requests

- **Context / Symptom:** The actual reviewed-edit smoke timed out before a provider turn started. A direct probe wrote initialize, initialized and account/read together but observed only the initialize response.
- **Root Cause:** `select` watched the OS pipe while buffered `stdin.readline()` had already consumed later lines into Python's private buffer. The fixture also used its launch directory rather than the separately requested thread directory for policy acknowledgement.
- **Solution:** Read available pipe bytes with `os.read` and drained complete newline-delimited packets from an explicit buffer before waiting again. The direct probe then received response IDs 1 and 2. Thread acknowledgement used the exact requested cwd and reasoning effort; production qualification and timeouts remained unchanged.
- **Prevention / Reference:** `scripts/controlled_peer_fixture.py` is a disposable protocol peer, not live-provider conformance or a replacement for the actual Eevee file-write path.

### [2026-10-20] Specialist desks covered the storage acknowledgement row

- **Context / Symptom:** The actual relay journey reached a supported hypothesis but timed out with `missing state: Storage: saved`; its specialist overlay visibly covered the entire storage/activity header row.
- **Root Cause:** The desk centered a nearly full-height panel against the entire terminal, starting at row 1. Header status was painted first and then cleared by the overlay, so local persistence acknowledgements and failures were not inspectable there.
- **Solution:** Reserved the two header rows and two global footer rows before sizing the desk. Compact confirmations retained their visible-summary/consent gates and scroll handling. Removed the obsolete fixed 16–20-row geometry assertion from the local-summary fixture rather than re-pinning it; semantic full-summary, no-scroll and reviewed-state checks remained.
- **Prevention / Reference:** `workflow_relay_smoke.py` waits for actual storage status while the desk is open at both sizes; do not replace this with a sleep or assume a hidden save succeeded.

### [2026-10-20] Rust panic thread IDs were parsed as part of a source path

- **Context / Symptom:** The actual Cargo/TUI location list displayed `5894500) panicked at src/lib.rs:3:5`. Pin range correctly refused it with `Source is absent from the current nonignored catalogue` instead of capturing `src/lib.rs`.
- **Root Cause:** Newer Rust panic output included a parenthesized thread ID before `panicked at`. The Node stack-frame heuristic treated any last opening parenthesis as an unclosed filename wrapper and included the already-closed thread annotation.
- **Solution:** Used the parenthesized-path branch only while the parenthesis remained unclosed before the coordinates; otherwise normal path tokenization applied. Added the exact observed thread-ID form and a Node path containing spaces to the parser regressions. All three isolated parser tests passed after the correction; actual integrated Pin range verification remained required.
- **Prevention / Reference:** `target/wave3-execution-relay-runtime/120/cargo-locations.txt` retained the failing-before surface. Source containment/exclusion checks were not weakened to accommodate a bad parse.

### [2026-10-20] Finding relays bypassed the source credential boundary

- **Context / Symptom:** Static review of the new finding relay found that its exact range-comparison cache copied the whole raw file into task metadata, including recognizable credentials outside a sanitized pinned range.
- **Root Cause:** Normal source capture applied secret masking, but the separate raw baseline reader and workflow validation did not inherit that boundary. Redacting only the persisted cache would also make later exact range comparisons incorrect.
- **Solution:** Refused credential-bearing raw baselines before staging or mutating the relay and enforced the same refusal in workflow storage validation. Added a regression covering actual Finding → Fix refusal, unchanged source/evidence/workflow state and no credential in the outgoing snapshot.
- **Prevention / Reference:** `relay_paths`, `workflows::State::validate` and the credential-bearing baseline regression. Use synthetic credential fixtures only; a source hash is not permission to persist all its raw bytes.

### [2026-10-20] Deferred decision drafts and edited criteria lost their original provenance

- **Context / Symptom:** Static review found that relaying decision A, choosing B and then fronting A's recipient could attach B's identity to A's text. Ordinary criterion renames/deletions could also leave stale origin rows and eventually exhaust origin capacity despite an empty checklist.
- **Root Cause:** Recipient hydration substituted the latest chosen decision instead of the original relayed snapshot; the normal Criteria editor updated only the checklist, not its provenance lifecycle.
- **Solution:** Persisted the original decision snapshot with the draft relay and refused stale proposal confirmation. Reconciled criterion origins atomically with normal editing: exact identities survived reordering, only an unambiguous single rename inherited its origin, deletions released capacity and ambiguous duplicates refused without mutation.
- **Prevention / Reference:** Regressions cover A→B recipient restoration, normal rename/full task restoration, unrelated reorder, duplicate refusal and 64 criteria → clear → new criterion.

### [2026-10-20] Finding range addressing misclassified appends and omitted untracked deletions

- **Context / Symptom:** Static review found that appending after a finding's final line marked it addressed, while deleting a previously relayed untracked file could leave all its findings unaddressed.
- **Root Cause:** The edit-alignment insertion transition reused the deletion-line intersection flag. Separately, removed untracked files vanished from the repository inventory instead of receiving the tracked-file `Missing` state; failed reads were treated uniformly as unavailable evidence.
- **Solution:** Computed insertion boundaries separately from deleted/replaced lines. Required a complete stable repository observation and confirmed contained no-follow disappearance before treating an omitted file as deleted; links, traversal, replaced roots and arbitrary read failures remained unknown. Neither path automatically resolved a finding.
- **Prevention / Reference:** Boundary/insertion/replacement/deletion fixtures plus no-follow disappearance and app-level deleted-untracked-source regressions.

### [2026-10-20] Ordinary repository refresh did not update stale-check admission

- **Context / Symptom:** Static review found that home `/refresh` could make specialist evidence stale while `/rerun` still compared against the completed run's old execution snapshot and refused the advertised action.
- **Root Cause:** `execution::State::repo_observed` returned before updating freshness unless an execution-specific before/after observation was pending.
- **Solution:** Updated the latest observed source fingerprint for ordinary observations too, while preserving generation gates that alone settle command preparation and final journal snapshots. Added a regression proving an ordinary refresh updates staleness without completing an unstarted execution capture.
- **Prevention / Reference:** The actual execution-relay journey uses home `/refresh`, not an execution-only refresh. Retained command arguments containing irreversible journal masks are separately refused rather than replayed as substituted `[REDACTED]` bytes.

### [2026-10-16] Carry-over evidence fixtures failed native canonical-root validation

- **Context / Symptom:** v0.1.10 qualification run `38866630609` failed tests on macOS and Windows. Native Windows diagnosis reproduced three carry-over assertions reporting one open hypothesis instead of zero for otherwise supported evidence.
- **Root Cause:** The new fixture stored the raw temporary directory as its source root. `repo::read_source` correctly requires the already-canonical identity used by real captures; macOS system aliases and Windows canonical/verbatim path prefixes made the raw fixture root invalid. Existing workflow/recommendation fixtures already canonicalized their roots.
- **Solution:** Canonicalized the carry-over fixture root after creating it, reusing the existing fixture convention. Did not weaken source identity checks, change expected hypothesis status, skip platform tests or reuse packages from the failed run. Qualification must restart for the corrected commit across Check and all five native rows.
- **Prevention / Reference:** Keep diagnostic build output on local Windows storage and clear inherited WSL `TERM`. Public Actions log download required authentication unavailable on this host, so diagnosis used the installed native Windows Rust 1.90.0 toolchain instead. Separate local parallel store timeouts were investigated with serial diagnostic execution; CI's normal parallel test command and assertions remained unchanged.

### [2026-10-16] Carry-over snapshots treated pending context as empty evidence

- **Context / Symptom:** A regression switched forms while the stored pin load was pending; the initial implementation immediately published a zero-pin record. Hiding that pending record with the next key also discarded the switch before its counts settled.
- **Root Cause:** Empty in-memory source/history collections were indistinguishable from completed empty loads at the snapshot boundary, and dismissal erased the pending transition instead of only its future visibility.
- **Solution:** Deferred snapshot settlement through pending source/run-history loads, retained the transition independently of visibility, and recorded late settlement without redisplaying it after input. Reopening then used the settled record. Behavioral regressions covered queued/in-flight run history, delayed pin acknowledgement, the first Unicode insertion/caret and later reopening.

### [2026-10-16] Explicit specialist refresh left carry-over hypothesis counts cached

- **Context / Symptom:** After a supporting source changed with otherwise identical stored metadata, deliberate specialist refresh showed unresolved proof while the next carry-over still counted zero open hypotheses. The failing regression observed 0 instead of 1.
- **Root Cause:** The carry-over metadata cache did not observe the deliberate source-revalidation boundary; unchanged serialized references did not prove the underlying bytes were unchanged.
- **Solution:** Invalidated carry-over hypothesis counts alongside recommendations on explicit workflow refresh. Source restoration was covered by the same regression. Recommendation and carry-over consumers shared one repository fingerprint, while ordinary form/draft/nickname changes remained cache hits.

### [2026-10-16] Compact picker hid the identity of an off-screen recommendation

- **Context / Symptom:** EV01B's actual 60×18 fixture offered `d Dismiss suggestion`, but Umbreon was outside the five visible rows while Jolteon remained selected, so the user could not identify the recommendation.
- **Root Cause:** The picker only marked visible form cells; its heading named the current form but not the off-screen recommendation. The home recommendation could also remain as an unintended background status.
- **Solution:** Included the recommended form in the compact heading without moving selection or portrait rectangles, and suppressed the background home recommendation under Forms. Both actual sizes passed dismissal/restart/new-run journeys; `target/ev01b-tui/60-dismiss/marked-with-current-selection.txt` shows current Jolteon and suggested Umbreon together.

### [2026-10-16] Equivalent review metadata kept a stale recommendation cache result

- **Context / Symptom:** The EV01A source-review fixture restored source bytes and explicitly recorded the same accepted-review metadata again, but the cached recommendation remained absent.
- **Root Cause:** Comparing only stored input values could not distinguish a deliberate evidence refresh with identical serialized values from an unchanged frame.
- **Solution:** Invalidated the suggestion verdict cache at the existing deliberate workflow-refresh boundary, keeping source revalidation outside rendering. Unchanged draft/animation frames still did not reread source files. Replaced copied capture/run cache contents with borrowed views and a bounded metadata digest.


### [2026-10-16] Empty-project startup skipped personal partner identity restoration

- **Context / Symptom:** A PK01 regression loaded a committed partner nickname with an empty task list, but the lead still had the default name. Task-backed restarts already restored the nickname.
- **Root Cause:** The Loaded handler refreshed the lead partner only inside its selected-task branch; the no-task path copied the roster without updating the lead.
- **Solution:** Restored the lead from the committed roster even without a selected task, preserving an unsaved live form when local edits existed. The runtime proof also renames before creating any task, exits, and restarts.


### [2026-10-16] Rejected nickname paste could turn Enter into an unintended clear

- **Context / Symptom:** The PK01 actual-TUI boundary case deleted the old candidate, pasted 13 CJK graphemes, and pressed Enter. Paste was visibly refused, but Enter saved the remaining empty editor and cleared the committed nickname.
- **Root Cause:** Insertion rejection retained the previous editor text but its error did not participate in save admission. Empty text was independently valid as the clear action.
- **Solution:** Retained an insertion-rejected flag until an explicit valid edit/correction. Enter after refused input queued no mutation; store-failure errors remained separately retryable. Added a regression starting from a committed nonempty nickname.
- **Prevention / Reference:** The actual terminal proof covers 13-grapheme refusal, the 12-CJK boundary, duplicate rejection and explicit clearing; display clipping never becomes a different saved value.


### [2026-10-16] Neutral intake-return review hid a retained provider draft

- **Context / Symptom:** The EV07 actual-TUI provider cancellation journey kept the stored follow-up draft but showed the empty composer placeholder after Stay or an unknown cancellation outcome.
- **Root Cause:** Generic slash dispatch retained and closed the composer before opening the local Return to Eevee confirmation. The review did not open an overlay whose closure would restore the editor.
- **Solution:** Handled the busy `/eevee` and `/normal` review before generic editor retention, so neutral Stay and cancelled/unknown outcomes kept the original input, caret and editing state. Regression coverage checks both known and unknown cancellation outcomes.
- **Prevention / Reference:** The native HTTP fixture correctly reported a locally closed reader with remote outcome unknown; intake return remained blocked rather than relabeling it safe cancellation. The owned-command fixture separately verified actual process-tree exit before returning.


### [2026-10-16] Partner attribution displaced compact run evidence and smoke routes

- **Context / Symptom:** The resumed EV03B recovery evidence showed `Failed` in the 60×18 history but lost the `Failed command` freshness line. Local workflow acceptance also waited for the obsolete `Flareon / Codex implementation` title while the actual native route displayed `Flareon / Codex · native (no tools) implementation`.
- **Root Cause:** Partner attribution lengthened the first history line beyond the fixed three-row slot. Smoke drivers also assumed any form could prepare commands or source edits, and pinned older capture/provider display copy.
- **Solution:** Kept state/task/end time on the first history row and moved partner attribution beside the run id on the second. Migrated command-preparation fixtures to Jolteon and implementation fixtures to Flareon. Used the current capture review boundary and provider-independent Flareon title prefix while retaining stored-evidence, no-send and unchanged-source assertions.
- **Prevention / Reference:** `scripts/execution_smoke.py`, `scripts/execution_recovery_smoke.py` and `scripts/workflow_smoke.py`; owning-form switches never replay the blocked action.

### [2026-10-16] Rust language server exited during resumed EV03B inspection

- **Context / Symptom:** The configured rust-analyzer failed the command-gate reference request with `LSP server exited unexpectedly (code 0)`.
- **Root Cause:** The configured server was not usable in this session; no symbol response was available.
- **Solution:** Used scoped source/caller inspection and Rust 1.90.0 compiler, test and Clippy verification without changing global tooling. No exported symbol was changed.


### [2026-10-16] Update progress differed from installation before download

- **Context / Symptom:** Self-update performed its initial release lookup before the installer's visible phases and did not preserve the user's UTF-8 locale when handing over to it.
- **Root Cause:** Release discovery was owned by the updater while animation was owned by the embedded installer; the sanitized helper environment omitted `LANG`, `LC_ALL` and `LC_CTYPE`.
- **Solution:** Reused the embedded installer's animated release lookup, retained locale variables alongside existing terminal settings, and kept read-only checks separate. Windows lookup used a private result file so both console handles remained attached; redirected/CI/dumb-terminal output stayed static.
- **Prevention / Reference:** The actual POSIX install/update lifecycle passed lookup motion, resize/restoration, monochrome shape parity, checksum refusal and read-only checks; four visible drawing frames were shared between install and update. Native Windows syntax/type checks were separate from terminal-animation qualification.

### [2026-10-16] Compact headers hid source failures and model selection

- **Context / Symptom:** At 60×18, purged-context dispatch displayed the detailed refusal but hid its status line. A long project/partner prefix also displaced the selected model and reasoning level from the home header.
- **Root Cause:** The source ownership sentence wrapped into the row reserved for status, while the home header concatenated context before selection without reserving selection width.
- **Solution:** Used separate fixed source ownership/status rows and a bounded compact home header with explicit model/effort space. Complete source failures remained scrollable; long contextual labels retained visible truncation.
- **Prevention / Reference:** The real source-context matrix exercised compact purge, missing/stale source and storage failures. A viewport regression retained model/effort with a long project name; targeted actual-TUI captures checked the selected values beside the draft.

### [2026-10-16] Directory separators escaped the attachment query

- **Context / Symptom:** Typing a scoped `@` query opened Commands at its first slash instead of filtering files, despite the query working in isolated matching tests.
- **Root Cause:** Global slash dispatch ran before the composer submenu and did not classify the active attachment query as literal input. Repeated catalogue requests while typing also temporarily blocked selection of otherwise visible rows.
- **Solution:** Gave the active file query character ownership, fetched one eligible catalogue per opening, and filtered its original indices locally. Case-insensitive scoped abbreviations and spaces never broadened source eligibility or bypassed capture review.
- **Prevention / Reference:** The regression drove real slash/space key events with a retained suffix/caret. Actual terminal journeys inspected the intended scoped source and excluded the same basename in a different directory.

### [2026-10-16] Local selection reviews kept an extra drawer or lost their visible owner

- **Context / Symptom:** Keeping the current provider after its review returned to Connections instead of the invoking editor. A direct unlisted-model review could also disappear during ordinary planning settlement while its pending selection remained.
- **Root Cause:** The connection launch frame was removed only after synchronous selection. The new direct-model review had no picker, but busy classification and automatic drawer settlement still relied on picker presence.
- **Solution:** Removed the launch-only connection frame before waiting for local review completion, counted pending model changes as busy, and excluded them from automatic drawer closure. Neutral cancellation preserved the original provider/session and exact draft.
- **Prevention / Reference:** App-level regressions exercised settlement and cancellation, not only the isolated picker. The direct-model regression failed before the ownership fix; actual wide/compact journeys exercised the provider review and retained editor.

### [2026-10-16] Cancelling metadata connection review left an invisible picker owning input

- **Context / Symptom:** The real-TUI project-executable trust fixture correctly stopped before execution, but Escape returned home and `/quit` was subsequently typed as literal draft text. The interface reported an unfinished request although no modal was visible.
- **Root Cause:** Metadata cancellation completed after the review overlay closed. Its normal completion path reopened the model picker inside planning state without a visible owner, leaving global commands blocked.
- **Solution:** Explicit catalogue dismissal now survives asynchronous cancellation and prevents both completion/error paths from reopening a picker. A new inspection resets this dismissal; blocked automatic inspections still retain their error/retry view.
- **Prevention / Reference:** Regression coverage checks cancellation reaches idle without a hidden picker or queued run. The actual terminal fixture also requires normal command-based exit and verifies the project executable remains untouched.

### [2026-10-16] First direct request cancelled during pending local context restoration

- **Context / Symptom:** The initial Enter-only native request returned to its draft before any network output with a misleading request-changed notice.
- **Root Cause:** The first durable task save could trigger context restoration before its prepared preview arrived. Temporary local readiness loss was incorrectly treated as a changed request.
- **Solution:** Retained the immutable prepared owner while the matching task/revision waited for local readiness, then dispatched exactly once. Real revision changes and invalid terminal sizes still cancel; failure notices preserve the actual reason.
- **Prevention / Reference:** The regression exercises pending save/readiness before dispatch. The actual first-send fixture completed one response with zero model-discovery requests and committed its first-used default route before exit.

### [2026-10-16] Compact Codex method selection browsed native settings without switching the task

- **Context / Symptom:** The 60×18 real-TUI journey chose native responses, but the task header remained CLI and the next action opened native setup instead of returning home. The wide layout's persistent provider column had hidden that incomplete transition.
- **Root Cause:** Removing a method shortcut from visible command choices also removed it from the shared availability validator, so the actual provider-switch command was refused after the drawer changed its browsed row. The intermediate provider frame also remained in overlay ancestry.
- **Solution:** Kept explicit native/CLI shortcuts as connection-method actions under the one Codex provider, preserving their capability validation. A completed local method choice removes the intermediate provider frame and returns to its invoking editor; sessions still use the real handoff boundary.
- **Prevention / Reference:** Assert actual task route and retained editor at both supported widths, not merely presence of a Codex label. The runtime journey checks that method selection closes the extra drawer without launching authentication.

### [2026-10-16] Starting in the user home blocked native provider configuration

- **Context / Symptom:** The actual release TUI launched with its working directory equal to HOME refused a private user-owned provider config with `Cannot read user provider config safely`; no metadata HTTP request was sent.
- **Root Cause:** Every current working directory was treated as an untrusted project boundary, so launching from the home directory incorrectly classified its own user-scoped provider stores as project files.
- **Solution:** Canonically verified home-directory startup is no longer a project-file boundary. Other workspaces, including projects beneath home, retain the project-source restriction. Ownership, permissions/DACL, no-follow identity and endpoint restrictions remain unchanged.
- **Prevention / Reference:** The focused real-TUI journey records failing-before and successful-after discovery without inference or config changes. Regression coverage also refuses the same files without a verified home and refuses project-local configuration in home descendants.

### [2026-10-16] SDK and ACP still rejected models omitted by discovery

- **Context / Symptom:** Exact custom IDs were locally refused even when the selected integration exposed a real model setter. The SDK also blocked every explicit effort because it lacked an applied-effort acknowledgment, although official request Options supported the value.
- **Root Cause:** Catalogue membership had been coupled to request eligibility in adapter-specific preflight and setter acknowledgment checks. Informational effort capability was confused with an assertion of observed application.
- **Solution:** Removed membership gates, retained official selector/control availability and exact correlated model acknowledgments, and kept SDK authentication/tool policy independent. SDK discovery failures remained inspection errors but no longer blocked acknowledged explicit models. Exposed advertised SDK effort request values with applied effort explicitly unknown. Required bridge revision 3 so old installed bridge code could not silently retain the rejected behavior.
- **Prevention / Reference:** Exact-ID and stale-option acknowledgment regressions cover SDK/ACP state transitions. Provider account qualification remains separate from parser/policy fixtures; no unconfigured live account is claimed.

### [2026-10-16] Native platform qualification exposed HTTP and ACL fixture assumptions

- **Context / Symptom:** Native macOS CI failed the loopback SSE reader fixture with `WouldBlock` / `Resource temporarily unavailable`; native Windows tests failed before production validation at fixture `SetSecurityInfo`, returning `ERROR_ACCESS_DENIED` (5). Linux rows and the local serialized Rust suite had passed.
- **Root Cause:** Darwin accepted sockets inherited the listener's nonblocking mode, while the fixture assumed bounded blocking reads. Windows fixtures rewrote inherited ownership/DACL state after file creation; the reported API transition failed on the runner account. The exact internal Windows denial cause was not established, so it was not attributed to guessed handle rights.
- **Solution:** Explicitly selected blocking mode on accepted fixture sockets while retaining read/write deadlines. Created Windows fixtures atomically with current-user owner and protected DACL in `SECURITY_ATTRIBUTES`; NULL-DACL cases use the same creation path. Production owner/DACL/reparse checks were not weakened, and no failing platform test was skipped.
- **Prevention / Reference:** Run the ordinary parallel all-target suite, not only `--test-threads=1`; local parallel acceptance passed 563 tests with 2 ignored after the socket fix, and native Windows owner/DACL fixtures passed qualification run `38762603406`. Each final source candidate still requires its own five-platform qualification. Runtime cancellation checks use actual client socket closure, no-resend and preserved draft/response ownership rather than incidental transient notice wording.

### [2026-10-16] Native response was rejected despite valid streamed provider output

- **Context / Symptom:** Native ChatGPT model discovery returned seven visible text models, including GPT-6.1/GPT-6, with an empty executable PATH. An explicitly reviewed GPT-6.1 request then ended with `Provider did not return an SSE stream; remote outcome unknown`.
- **Root Cause:** The successful HTTP response omitted `Content-Type` even though its body contained semantic SSE frames. A MIME-header prerequisite rejected valid provider output before the bounded protocol parser could inspect it. The request had already been sent, so the unknown-outcome label was retained rather than claiming a harmless preflight failure.
- **Solution:** Removed the MIME-header admission gate and used actual bounded SSE framing plus authoritative protocol completion to validate the response. JSON/HTML, truncated output and absent terminal events still cannot become successful responses. Removed the throwaway response-shape diagnostic without logging response values or credentials.
- **Prevention / Reference:** The real-provider journey is recorded under `target/native-017-live`; the extended workbench fixture deliberately omits the SSE MIME header for actual approved HTTP requests. Its second-turn ownership, final refusal text, refresh failure and cancellation checks exercise the full native path rather than merely echoing parser inputs.

### [2026-10-16] Native connection review found credential-mode and response ownership defects

- **Context / Symptom:** Static review found that explicit keyless mode could still attach an inherited API key, Windows file sources lacked the Unix-equivalent owner/permission boundary, and repeated native responses reused one consumer item identity. Responses refusal events were also ignored.
- **Root Cause:** Credential resolution consulted the environment before honoring keyless mode; file guards were Unix-only; native item IDs were constant across generations; text extraction handled only ordinary output text.
- **Solution:** Resolved keyless mode before any key lookup, added handle-bound Windows owner/DACL/reparse/resolved-path checks, qualified native item identity by request generation, and reconciled final text/refusals without duplicate output. Native consent now shows a typed immutable HTTP destination rather than subprocess-only effects.
- **Prevention / Reference:** Added key-audience/keyless, platform trust, cross-request identity, refusal/final-tail and actual two-request workbench regressions. Existing exact payload/configuration revalidation, no-redirect policy and authoritative cancellation/completion boundaries remain enforced.

### [2026-10-16] Native follow-up was incorrectly routed to unavailable session resume

- **Context / Symptom:** The actual TUI native-service fixture completed its first request, but Enter on the saved second request showed `Direct API requests are fresh and stateless here` instead of a Run review.
- **Root Cause:** Observed-model metadata reused `Event::Thread` with an empty thread ID. The consumer retained `Some("")`, so the composer attempted session resume. After separating that event, identical empty turn IDs still made the consumer associate the second response with the first request.
- **Solution:** Introduced a separate correlated `ModelObserved` event and unique `native-request-{generation}` local turn identities. Native streams report their actual model without manufacturing a provider session; fresh follow-ups and response ownership remain independent.
- **Prevention / Reference:** The model-acknowledgment regression asserts no resumable thread and a fresh follow-up. Producer tests assert distinct thread/turn correlation pairs; the workbench approves two actual HTTP/SSE requests and inspects each request owner before testing metadata failure and client cancellation.

### [2026-10-15] macOS self-update smoke lost its PTY before restoration inspection

- **Context / Symptom:** Published v0.1.6 installation acceptance passed the first 15 checks, then `live-owned-binary-self-update` failed at `termios.tcgetattr(self.slave)` with `(25, 'Inappropriate ioctl for device')`.
- **Root Cause:** The harness launched the update executable as a session leader. Unix self-update intentionally execs `/bin/sh`; that session-leading shell could acquire and revoke the macOS PTY on exit. The existing bootstrap-only wrapper prevented this for direct installation but not the same shell reached through update.
- **Solution:** Moved the existing POSIX parent-process wrapper into shared `animated_install`, covering bootstrap and self-update without changing product binaries or weakening termios/animation assertions. The actual installer remains a child, like a command launched from a normal user shell.
- **Prevention / Reference:** Re-ran complete acceptance against the downloaded v0.1.6 package: all 23 lifecycle checks and 34 installed terminal scenarios passed (`target/release-016-installed-verified`). Python release-script tests ran 17 with 3 platform skips. Keep both shell-entry paths under the same ownership guard.

### [2026-10-15] Codex showed only GPT-5.6 instead of GPT-6 and GPT-6.1

- **Context / Symptom:** `/model` showed only `gpt-5.6-sol`, `gpt-5.6-terra` and `gpt-5.6-luna`. Native inspection returned those exact three entries with no next page, so this was not an Eevee pagination or generation-name filter.
- **Root Cause:** PATH selected Homebrew Codex 0.147.0. With the same ChatGPT account and model configuration, isolated official 0.162.1 returned seven entries including `gpt-6.1-sol`, `gpt-6-astra`, `gpt-6-sol` and `gpt-6-luna`. Native lists can be bundled/cache-backed; they are not a guarantee of live global model availability. Separately, Eevee had treated the catalogue's default as the provider configuration default, ignoring the configured `gpt-5.6-luna`/`high` selection.
- **Solution:** Exposed actual executable/version and catalogue provenance, model count and compact scroll controls; added explicit same-installation update/refresh guidance. Read only the effective nonsecret model/provider/effort through official `config/read` and preserved that configured default through pre-dispatch catalogue revalidation. Kept complete pagination and exact advertised-model/effort validation; did not hardcode newer names, inject arbitrary models, silently substitute defaults or upgrade the host CLI.
- **Prevention / Reference:** Update the installation shown in the picker (`brew upgrade --cask codex` for this Homebrew path), then `/models-refresh`; changing Eevee's version alone cannot update its external CLI. The isolated [official 0.162.1 archive](https://github.com/openai/codex/releases/tag/rust-v0.162.1) matched SHA-256 `b293a0fcdd871900a44c0a4f40a1aa401b31e02854f790ebdb8bd5d1177fd1fe`. Native release-candidate journeys observed seven models at 120×36 and 60×18, an explicit GPT-6.1-Sol response, and a separate Default response correctly retaining GPT-5.6-Luna (`target/release-016-live/results.json`). The user did not authorize a global Codex upgrade.

### [2026-10-15] Fresh-home model loading stalled or completed into a hidden picker

- **Context / Symptom:** Published 0.1.5 stayed on `Loading models…` after `/model` on a fresh home. An integrated real-TUI smoke then found that the request-first settlement path closed the loading overlay, leaving a completed picker hidden behind the home screen and blocking another model action.
- **Root Cause:** The automatic catalogue driver and preparation path required a saved task even though metadata inspection carries no task content. The generic response-to-home transition treated catalogue loading like a model response. ACP also validated committed task/review input before its metadata-only branch; Codex required `inputModalities` even though the installed 0.147.0 schema declares it optional with a text/image default.
- **Solution:** Used an empty project-scoped snapshot before task creation, stripped task/context/write/handoff data from metadata inspection, kept metadata dialogs visible through completion, and retained provider-specific errors with retry actions. Moved ACP metadata admission before task-content checks while retaining provider/project/selection checks. Accepted only an absent Codex optional field using its documented default; explicit invalid modalities still fail. Preserved separate consent for manual refresh and all content requests.
- **Prevention / Reference:** Fresh-home and save-during-load regressions cover ownership and draft restoration. Actual Codex 0.147.0 catalogues loaded at 120×36 and 60×18; missing-provider/retry and fresh reviewed-refresh paths ran in the native macOS PTY (`target/home-model-after`). These metadata inspections sent no task prompt.

### [2026-10-15] Response progress appeared twice and the home partner disappeared

- **Context / Symptom:** The user observed duplicate loading in the response UI and no Eevee card on the main screen.
- **Root Cause:** Both the shell header and response metadata rendered `activity_label()` with a spinner, while the response also displayed a progress notice. The request-first shell gated the partner rail on companion focus rather than screen capacity.
- **Solution:** Made the header the sole live-progress owner, kept response metadata about its selected output only, and reserved footer rows only for available controls/notices. Earlier partial output no longer inherits the current stream or observed model. Restored the partner card at ≥90 columns, using the same native-image placement geometry; compact/linear layouts retain the drawer.
- **Prevention / Reference:** Two actual Codex turns at 120×36 and 60×18 exercised connecting, waiting, streaming, completion, previous-request attribution, paused follow and return-to-latest. Captured 110 wide and 123 compact sampled frames without duplicate stage labels. Cell-rendered cached portraits were exercised; this is not a new Kitty/Sixel GUI compatibility claim.

### [2026-10-15] Host Clippy version differed from the repository qualification toolchain

- **Context / Symptom:** Host Rust 1.97.1 passed 497 tests, but strict Clippy rejected an existing `src/ui/handoff.rs` optional-candidate access with `clippy::unnecessary_unwrap`.
- **Root Cause:** The host default toolchain was newer than the repository's Rust 1.90.0 qualification toolchain and enforced a newer lint on unchanged code.
- **Solution:** Ran final formatting, all-target tests, strict Clippy and the runtime build with the already-installed `+1.90.0` toolchain. No lint was suppressed and no unrelated handoff code was changed.
- **Prevention / Reference:** Use `cargo +1.90.0 clippy --locked --all-targets -- -D warnings` to reproduce the supported qualification check. Host 1.97.1 strict lint remains a known unmodified-code limitation. Language-server startup also exited on this host; scoped source inspection was used instead.

### [2026-10-15] Workbench smoke assumed unwrapped setup text and full task text on home

- **Context / Symptom:** The 60×18 journey failed waiting for contiguous `npm install`, although setup visibly wrapped those words onto adjacent rows. The locked-store journey also expected an appended multi-line Goal tail on the summarized home card.
- **Root Cause:** The assertions depended on incidental text wrapping and the older full-task home presentation rather than the actual setup and retained editor state.
- **Solution:** Removed the installation-wording assertion and reopened `/goal` before checking the retained failed-save edit. The smoke still checks missing-provider state, absence of an authentication review, safe navigation back, failed-save retention/retry and byte-preserved persisted drafts.

### [2026-10-15] Microsoft Store Python escaped the native smoke's tracked process tree

- **Context / Symptom:** Windows command cancellation saved `OutcomeUnknown / OwnershipUnknown`: the launched root had exited and Job accounting was empty, but both output pipes remained open. A direct process-handle check found the fixture root stopped while its child and grandchild remained alive.
- **Root Cause:** `uv run --python 3.13` selected Microsoft Store/App Execution Alias Python 3.13.14. Its fixture descendants were not contained in the tracked Job on this host; the exact package/broker mechanism was not traced. Ordinary native Rust children and a separate unpackaged Python tree did not reproduce it. This was not resolved by closing observer handles or changing console-detachment flags.
- **Solution:** Installed unpackaged Python 3.13.14 into an isolated temporary directory with `uv python install 3.13 --install-dir <directory> --no-bin --no-registry`, then set `UV_PYTHON_INSTALL_DIR` to that directory and `UV_MANAGED_PYTHON=1` for native verification. The unchanged cancellation implementation then reported `Cancelled / OwnedTreeExited`, no truncation/error, and all three fixture processes stopped. No global Python, PATH or registry changes were made.
- **Prevention / Reference:** Keep the unknown-outcome guard and original cancellation deadlines; do not count terminated launcher processes as proof that descendants stopped. Use unpackaged Python for ConPTY/owned-tree qualification. Native fixtures record interpreter provenance, and execution timeout diagnostics now include persisted state/tree/cancellation/reservation values. All temporary runner traces and console-flag experiments were removed.

### [2026-10-15] Deferred clean exit blocked waiting for an unnecessary key

- **Context / Symptom:** The actual command lifecycle cancelled its owned process tree and committed the final journal, but cancel-and-exit then remained on idle output instead of exiting or offering an exit review.
- **Root Cause:** Diagnostic tracing confirmed `Navigate(Quit)` reached the app with equal saved/current revisions and no dirty active draft. This clean exit was set near the start of the main loop, which entered blocking `input.read()` before its bottom-of-loop exit check. Request-first focus exposed the clean path that previously often required another save-confirmation key.
- **Solution:** Honored the settled quit immediately after deferred execution actions, before rendering or input waiting. Kept the existing bottom check for input/worker-driven outcomes. Removed the temporary diagnostic prints; no extra polling timer or forced process termination was added.
- **Prevention / Reference:** The existing actual execution smoke's cancel-and-wait/clean-exit/draft-restart scenario reproduced the failure without extra input; it remains the regression acceptance path on Linux and native Windows.

### [2026-10-15] Context smoke still expected the old repeated-root packet

- **Context / Symptom:** The source smoke expected 326 outgoing bytes while the actual reviewed envelope contained 267. Its expectation serialized the full stored capture DTO instead of the already-shipped compact outgoing packet.
- **Root Cause:** The fixture predated root hoisting, hexadecimal source identities and removal of storage-only fields from outbound captures. Execution fixtures also expected duplicate Back-history entries, and treated absent task cards as proof of cancel-and-switch acknowledgment even though the cancellation guard had already hidden those cards.
- **Solution:** Checked the current reviewed envelope against independent committed capture facts and its exact UTF-8 count, comparing parsed JSON rather than incidental key order. Removed the obsolete navigation-cycle expectation and waited for the selected task's actual home content before sending further input. Privacy masking, source identities, stale/missing-source refusals and execution-denial assertions remain.
- **Prevention / Reference:** All 10 actual source/context journeys passed (`target/015-context-final`); keep storage DTO and outgoing review contracts distinct.

### [2026-10-15] Request-first startup displaced a restored specialist overlay

- **Context / Symptom:** The actual specialist workflow resumed a saved task into its Goal editor instead of the saved Memory overlay. Closing Help could also resurrect a previously completed Goal edit instead of returning to navigation.
- **Root Cause:** Request-first editor initialization ran after restoring overlay navigation and overwrote it; a later explicit editor change retained the old composer-return owner.
- **Solution:** Restored overlays take precedence over the underlying editor, with its return owner preserved separately. Opening a different editor replaces that owner, and closing an overlay preserves its actual invoking focus.
- **Prevention / Reference:** A restart/focus regression and all 15 actual Linux specialist workflow cases passed (`target/015-workflows-complete`).

### [2026-10-15] Parallel capacity verification exceeded its worker-event deadline

- **Context / Symptom:** One integrated Rust run passed 480 tests but the capacity test timed out after save 239 while waiting five seconds for a Store worker event. Native verification was also running; this overlap alone does not establish the cause.
- **Root Cause:** Not established. Subsequent host sampling showed no sustained CPU or I/O saturation; no product storage failure was reproduced independently.
- **Solution:** Isolated final verification with `cargo test --locked --all-targets -- --test-threads=1`; all 481 tests passed with 2 ignored. No timeout, capacity, durability assertion or Store implementation was weakened.
- **Prevention / Reference:** Retain the original deadline and investigate with worker/thread diagnostics if the timeout recurs; a passing serial run is not a claimed concurrency fix.

### [2026-10-15] Clearing a submitted request raced committed-input admission

- **Context / Symptom:** A real first-screen request reached its review, but `y` immediately produced `Task changed since preview; save and review a new request` instead of a response.
- **Root Cause:** Returning home cleared and saved the Request draft before the worker's read-only committed-snapshot check. Providers also repeat that check during dispatch and permission handling, so merely delaying until thread creation was insufficient. Retained previous answers were additionally labelled with the newly submitted request.
- **Solution:** Held task saves through the content request's terminal acknowledgment while allowing visibly pending follow-up editing. Ctrl+S cannot bypass this ownership hold; cancellation waits before flushing. Pre-output failure restores the original input when no follow-up exists. Shared bounded request ownership now follows each retained response item.
- **Prevention / Reference:** The regression exercises real Store admission across Run, editing, Ctrl+S, thread establishment and cancellation. Actual latest Codex 0.162.1 first/follow-up requests passed at 120×36 and 60×18 with inline streaming and previous-response attribution (`target/015-request-first-final`).

### [2026-10-15] A source-purge shortcut could approve a hidden resized dialog

- **Context / Symptom:** Review found that `y` in a Sources removal confirmation lacked the visible-summary guard used by other confirmations; below-minimum rendering hid that dialog.
- **Root Cause:** The inner source handler had no terminal-size or rendered-scope state. Adding a direct shortcut exposed the existing cross-layer omission.
- **Solution:** Added current-target summary visibility to consent, invalidated it on resize and hidden renders, and rejected affirmative handling below minimum size. Source effects stay concise and fixed above optional exact capture details.
- **Prevention / Reference:** An actual-render regression covers unrendered consent, shrink/grow without repaint, long Unicode paths, paste/repeat/modified input and the final intended purge identity.

### [2026-10-15] Native smoke helpers could not lock an incremental target over WSL UNC

- **Context / Symptom:** The published Windows install journey passed live self-update, then its lifecycle-probe build failed with `incremental compilation: could not create session directory lock file` and OS error `-2147024895`.
- **Root Cause:** The helper hard-coded the repository's `target`, placing native Cargo incremental locks on a WSL UNC filesystem instead of a local Windows volume.
- **Solution:** Both lifecycle and older-release fixture builders now honor `CARGO_TARGET_DIR`. Native verification sets it to a local-C temporary directory; the pinned Rust 1.90.0 suite and release build complete there.
- **Prevention / Reference:** Keep native binary/build output local to Windows when the source checkout is accessed through `\\\\wsl.localhost\\…`.

### [2026-10-15] Installed-update smoke terminated the native cleanup helper too early

- **Context / Symptom:** Native self-update succeeded but the fixture observed a retained `.ev-stage*` directory after closing its ConPTY session.
- **Root Cause:** The updater's cleanup helper belonged to the harness-owned Windows Job. Closing that Job immediately after the old process exited killed the helper before it could remove the retired directory.
- **Solution:** Waited up to ten seconds for retired-stage cleanup before closing the session/Job. The original final cleanup assertion remains; the product updater was not changed.

### [2026-10-15] Latest OpenCode required its native-package postinstall

- **Context / Symptom:** An isolated latest-provider verification install used `--ignore-scripts`; OpenCode then reported `opencode-ai's postinstall script was not run`.
- **Root Cause:** OpenCode's official package needs its postinstall to select/link the installed native optional dependency.
- **Solution:** Inspected and explicitly ran that official postinstall inside the disposable verification prefix, without changing global providers. OpenCode 1.19.0 and Gemini 0.64.0 subsequently passed actual ACP1 initialization with no model request.
- **Prevention / Reference:** Normal documented OpenCode npm installation runs its required setup. Claude's separate latest-SDK setup intentionally retains `--ignore-scripts` and was verified independently.

### [2026-10-15] Published-release installer smoke duplicated checksum entries

- **Context / Symptom:** The published 0.1.4 archive passed fresh install and ten preservation/security cases, then the local HTTP bootstrap fixture failed with `malformed SHA256SUMS or missing unique checksum for ev-linux-x86_64.tar.gz`.
- **Root Cause:** `ReleaseFixture` always appended `install.sh` and `install.ps1` checksums. Native build-artifact manifests omit those entries, but the downloaded public release manifest already contained them; the fixture introduced duplicates that the real installer correctly refused.
- **Solution:** Preserved the supplied manifest and appended only missing installer entries. No installer validation, checksum assertion or release package changed.
- **Prevention / Reference:** Run installation lifecycle verification against downloaded public-release packages as well as pre-publication build artifacts; their checksum-manifest scopes differ.

### [2026-10-15] ConPTY full clears could escape effective visual synchronization

- **Context / Symptom:** The native save journey emitted about fifty full-screen clears. A minimal native probe showed `Begin → Clear → End → text` for a batched write: this ConPTY forwards DEC 2026 markers before publishing its rendered cells, so markers alone cannot prevent a blank flash.
- **Root Cause:** The main loop forced a full Windows clear on every dirty frame instead of using the terminal cell diff. The previous native smoke also relied on optional CSI 8 resize hints and on retaining the text inside an injected OSC sequence; ConPTY omitted the growth hint and filtered that OSC before the application received it.
- **Solution:** Removed ordinary Windows full clears while retaining startup, resize and native-image invalidation. Drained the old frame and updated the smoke terminal's requested dimensions directly, while still honoring explicit hints. Added a plain-text draft tail to the existing hostile-control paste fixture, retaining its no-execution and no-OSC-escape checks rather than asserting a platform-specific control-filter residue.
- **Prevention / Reference:** Native ConPTY editing at 120×36 and 60×18 produced zero full clears after startup, preserved exact draft/caret storage and restored console state. Six dark/light/mono-ASCII native journeys passed popup return, CJK/combining input, shrink/grow, nine forms, idle silence and exit (`target/native-014-{persistence,ui}`). Do not restore blanket per-input clears or assume every ConPTY version implements synchronized publication.

### [2026-10-15] Windows pipe fixture stalled before its test body

- **Context / Symptom:** Native Windows tests launched from WSL timed out in the piped stdin round-trip with an alive child, no output and no entry into the fixture. The failure reproduced with both Rust 1.92 and the release-pinned 1.90.0, including while the parent kept stdin open.
- **Root Cause:** A CDB child-stack capture located the wait in `NtCreateFile` → Rust libtest's `terminfo::get_dbpath_for_term` / `TermInfo::from_env`, before the fixture ran. The interop environment supplied `TERM=xterm-color`. Libtest initializes its terminal even with `--color=never`; this was not evidence of a pipe EOF or backpressure defect.
- **Solution:** Cleared `TERM` only in the native PowerShell verification process (`$env:TERM=$null`). Removed all diagnostic waits, markers and test-argument changes, retaining the original five-second deadline and round-trip/backpressure assertions. The pinned native suite passed 428 tests with 2 ignored and the optimized `ev 0.1.4` build completed.
- **Prevention / Reference:** Keep native verification environments separate from WSL terminal metadata. Do not increase transport timeouts or weaken process assertions to compensate for a test harness stalled before its fixture.

### [2026-10-15] Performance harness could not measure the current composer and child scope

- **Context / Symptom:** Baseline runs failed first on the removed `Ctrl+K Actions` footer, then with `POSIX raw input mode not observed`, and finally with `approved workload resource scope was not observed; refusing a core-only measurement`. The final terminal matrix also reported idle output while its before-capture visibly said `Storage: autosave pending`.
- **Root Cause:** Readiness was pinned to obsolete wording. The low-latency reader bypassed the normal raw-mode snapshot, so a clean, immediate quit could end before it was recorded. A completed child lost its command line before reaping and was reclassified from `local_commands` to `app_helpers`, despite a nonzero recorded local-command RSS peak. The idle check started before deferred navigation persistence completed.
- **Solution:** Removed the wording dependency; sampled terminal modes without adding a timed wait after the measured readiness instant; retained previously observed scope for the same PID/creation-time identity; awaited committed storage before asserting zero idle output. Kept resource-scope, zero-output and restoration assertions intact.
- **Prevention / Reference:** `target/performance-{baseline,after}/results.json` both completed with `measured_local_targets_met`, including the real approved stream, 10 MiB output and 10,000-line diff. Scope identity must not regress when an exited process stops exposing argv; pending storage is work, not idle.

### [2026-10-15] UX smoke dependencies were unavailable in the system Python

- **Context / Symptom:** The real-PTY UX capture failed with `ModuleNotFoundError: No module named 'pyte'`; installing into the system interpreter was refused with `externally-managed-environment`.
- **Root Cause:** This host uses a distribution-managed Python environment; the smoke harness requires its own pinned `pyte` and `wcwidth` dependencies.
- **Solution:** Created `target/ux-venv` with `uv venv` and installed `pyte==0.8.2 wcwidth==0.2.13` using `uv pip install --python target/ux-venv/bin/python`. Ran terminal verification with that interpreter without changing system packages.
- **Prevention / Reference:** Use an isolated smoke environment; do not override the distribution's externally-managed protection. Rust LSP reference lookup also exited before serving requests on this host; on 2026-10-16, `rust-analyzer --version` identified the resolving rustup shim's missing component (`Unknown binary 'rust-analyzer'`). Scoped source inspection and compiler/runtime checks were used without changing global tools.

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

- Opening `/model` or `/effort` uses cached state and can automatically inspect metadata when empty, including before task creation. `/models-refresh` provides a separately reviewed refresh. Inspection sends no task prompt; an error retains the previous list visibly stale. Exact-ID entry remains available independently of list membership.
- An unauthenticated Devin 3000.11.3 session can report a current model with an empty option list. That default is not evidence of selectable models or entitlement; connect through the native login and explicitly refresh again. No guessed model list is substituted.
- ACP reasoning options can belong only to the model active during inspection. After selecting another model, refresh the catalogue for that selected model before choosing effort. Unsupported controls show an explanation and reject requests instead of ignoring the selection.
- Claude SDK effort choices are advertised request values, not observed applied settings; organization caps/downgrades may apply. Exact model selection uses official setter/context acknowledgment even when discovery omits that ID. Update the complete optional bridge to revision 3 as well as its SDK dependencies; an older bridge is refused before dispatch.

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
