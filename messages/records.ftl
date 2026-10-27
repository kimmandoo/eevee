records-small = Records need 60 columns × 18 rows. Esc returns; no changes made.
records-summary = Partner summary
records-routes = Form routes · local non-evidence
records-dex = Failure Dex · this project
records-resolve = Review historical resolution link
records-habits = Repeated authored instructions
records-pairings = Partner provider pairings
records-discard = Review unsaved record discard
records-position = Entries { $index } / { $count }
records-local-only = Local metadata only; never added to provider packets or handoffs.
records-waiting = Waiting for committed storage; exact pending records are retained.
records-queue-full = Local record capacity reached. Finish storage or review a failed write before continuing.
records-load-failed = Records could not be loaded. /records-retry retries; read failure does not block quitting.
records-none = none
records-produced = produced
records-stale = stale
records-current = current support
records-unavailable = unavailable
records-unknown = unknown / ambiguous
records-unknown-producer = producer unknown
records-marker = { $status } · { $count } artifacts · { $producer } · { $fact }
records-identity = { $name } · { $form } / { $element }
records-met = Met on { $date } UTC { $origin } in { $project }
records-derived = (derived date)
records-unknown-date = unknown date
records-unknown-project = unknown project
records-bond = Bond: { $count } shared project/personal memories (not a partner-specific count)
records-no-moves = No moves yet.
records-form-counts = { $form }: { $moves } moves · { $tasks } distinct producing tasks
records-unknown-artifacts = { $count } artifacts have unknown producers; not assigned to this partner.
records-task-markers = Current task · role artifacts (production is not verification):
records-empty = No committed partner records are available yet.
records-routes-disclosure = Routes are local navigation journals, NOT evidence. No exact visit-output link is retained; sequence/time proximity never establishes visit authorship. Use c for task/form contributions.
records-no-routes = No recorded switches for this partner on this task.
records-accepted-suggestion = { " " }[accepted suggestion: { $trigger }]
records-route-receipt = sequence { $ordinal } · { $date } UTC · { $reference }
records-signature = { $kind }: { $detail }
    Seen in { $count } distinct failing runs · first seen { $first } UTC
    Signature: { $key }
records-run-refs = Retained run references:
    { $runs }
records-historical-lead = Seen before: historical resolution in task #{ $task }
    Supported diagnosis: { $support }
    Later passing run: { $run }
    Source transition/cause: { $cause }
records-lead-only = A previous resolution is a historical lead, NOT a current fix or verification.
records-no-signatures = No retained failure signatures in this project.
records-resolution-summary = Link historical relation: { $signature } · { $support } · { $run }
    This is not current verification.
records-resolution-choice = Failure signature: { $signature }
    Supported diagnosis receipt: { $support }
    Later matching passing run: { $run }
    Source transition/cause: { $cause }
    Manual source transitions do not establish who made the fix.
records-no-resolution = No eligible relation yet. A supported diagnosis receipt must retain current support, then an observed source transition and a later owned complete passing run with matching command/cwd identity are required. No relation was guessed.
records-support-receipt = Diagnosis receipt { $reference } · { $freshness }
    { $detail }
records-habit-disclosure = Only an identical normalized authored instruction actually dispatched in at least three distinct requests is offered. Approval opens the existing project-memory review; nothing is saved automatically.
records-habit-hash = Stable instruction hash: { $key }
records-habit-source = Submitted request { $request } · task #{ $task } · { $owner }
records-no-habits = No unapproved repeated-instruction proposal. Dismissed hashes do not return.
records-habit-memory-disclosure = Submitted sources: { $sources }
    Explicit approval saves and selects a project memory and binds this exact instruction. A future exact current-instruction repeat uses that selected memory reference instead of duplicating the text. Original drafts and past/provider history remain unchanged. Editing, deleting or deselecting the memory disables substitution.
records-pairing-disclosure = Project/partner/form scoped local selections only. Save requires an explicit settled provider/model choice. Remembering never connects, probes, sends, grants permission or changes billing. Unavailable selections never fall back. Removing restores the independent global default.
records-pairing-offer = Explicit current selection · not saved yet:
records-pairing-committed = Committed remembered pairing:
records-pairing-value = Partner #{ $partner } as { $form }
    { $agent } / { $model } / effort { $effort }
records-no-pairings = No committed pairing selected.
records-default = Default
records-remembered = Remembered
records-remembered-unavailable = Remembered · unavailable or unverified
records-discard-disclosure = Discard only the displayed failed local write, retaining other queued work. Dispatched-instruction ownership is released only after this explicit decision. This does not undo committed work or erase provider history. Stay is selected initially.
records-failed-owner = Task #{ $task } · { $operation }
records-failed-instruction = Request { $request } · original owner { $owner }
    Retained authored instruction:
    { $instruction }
records-op-instruction = dispatched instruction observation
records-op-pairing = local pairing change
records-op-flavor = factual-line preference
records-op-dismiss = repeated-instruction dismissal
records-op-bind = approved memory binding
records-op-resolve = historical resolution relation
records-op-run = owned run observation
records-op-load = record load
records-op-usage = provider usage receipt
records-controls = ↑↓ choose · PgUp/PgDn page · Home/End · Esc back
records-dex-controls = ↑↓ signature · PgUp/PgDn page · r review Umbreon relay · /dex-resolve link · Esc
records-habits-controls = ↑↓ proposal · /habit-approve memory review · /habit-dismiss · PgUp/PgDn · Esc
records-pairing-controls = ↑↓ pairing · /save-pairing · /remove-pairing · PgUp/PgDn · Esc
records-review-controls = { $stay } / { $action } · ←→ · Enter
    { $review }
    PgUp/PgDn details · Esc back
records-discard-action = Discard unsaved write
records-discard-selected = [Discard unsaved write]
records-link-action = Link historical relation
records-link-selected = [Link historical relation]
records-summary-visible = Scope and effect shown · y/n select
records-summary-unavailable = Summary unavailable; cannot confirm
chips-inspect = Enter inspect
chips-remove = Del remove
chips-hint = Tab/Shift+Tab focus chip · ←/→ select · Enter inspect · Del remove · Esc keeps draft
chips-current = captured source current
chips-stale = captured source stale
chips-unavailable = capture unavailable · sending blocked
chips-unchecked = freshness unchecked
chips-wire-pending = exact wire receipt pending
chips-failure = failure location
chips-changed = changed file
chips-tests = test file
chips-changed-tests = changed test
records-remove-pairing = Remove pairing for partner #{ $partner } as { $form }; restore global default.
records-flavor-enabled = Enable factual activity lines after commit.
records-flavor-disabled = Disable factual activity lines after commit; consume events without a backlog.
records-memory-binding = Bind instruction hash { $key } to approved memory #{ $memory } revision { $revision }.
records-usage-owner = Provider usage request { $request } · original owner { $owner }
records-habit-available = /habits — a repeated authored instruction is available for explicit memory review.
records-help-summary = Inspect factual partner history, shared Bond memories and actual per-form output.
records-help-routes = Inspect local form-switch journals; these are not evidence.
records-help-dex = Inspect this project's failure signatures and retained run references.
records-help-resolve = Explicitly review a historical diagnosis/source-change/passing-run relation; no current-fix claim.
records-help-habits = Review identical authored instructions actually dispatched in three or more distinct requests.
records-help-habit-approve = Open the selected repeated instruction in explicit project-memory approval; nothing saves automatically.
records-help-habit-dismiss = Persist dismissal of the selected instruction's exact stable hash.
records-help-pairings = Inspect project/partner/form provider and model settings; no connection or request.
records-help-pairing-save = Save the explicit current provider/model/effort choice for this partner and form.
records-help-pairing-remove = Remove this form's remembered setting and restore the independent global default.
records-help-flavor-off = Persistently hide factual activity lines while still consuming events without replay.
records-help-flavor-on = Show future factual activity lines; old events are not replayed.
records-help-retry = Retry the exact failed records write, or explicitly reload after a read failure.
records-help-discard = Review discarding only the retained failed local write; Stay is selected initially.
chips-focus = Tab focus files
records-write-failed = Local record save failed /records-retry /records-discard
records-pairing-use-offer = /save-pairing · Use for { $partner } here
records-discard-summary = Discard unsaved write: { $identity }
    Committed work and other queued writes remain.
records-compact-marker = { $status } · { $count } artifacts · { $producer }
records-project-setting = This project's local setting
ribbons-title = Ribbons / project history
ribbons-history = Historical records / selected evidence below
ribbons-disclosure = First recorded qualified events, not present correctness. Local only; no permissions or work are started here.
ribbons-empty = No qualified historical milestone has been recorded in this project.
ribbons-accepted-review = Effective accepted review recorded
ribbons-verified-handoff = Verified handoff recorded
ribbons-learned-move = Explicitly learned move recorded
ribbons-line = { $kind } / { $name } / { $status }
ribbons-details = Task #{ $task } / producer #{ $partner } { $name } / { $form }
    Recorded { $date } UTC
    Evidence now: { $status }
    { $reason }
ribbons-hidden = Hidden from partner summary; evidence retained.
ribbons-visible = Visible in partner summary when flavor is on.
ribbons-foreign-task = Different source task. Enter opens Tasks only; select it deliberately to resume. Current task stays unchanged.
ribbons-artifact = Retained artifact: { $id }
ribbons-move = Original move #{ $id } / revision { $revision }
ribbons-foreign-owner = Different producer or form. Enter opens the ordinary party/form picker only; no lead or form is changed.
ribbons-open-artifact = Enter opens the source task's ordinary review/handoff inspector when already on that task; never exports or sends.
ribbons-open-move = Enter opens ordinary move details/menu for this owner/form only. Deleted sources are not replaced.
ribbons-controls = Up/Down select / h hide/show / Enter inspect
    PgUp/PgDn page / Home/End / Esc back
ribbons-visibility = historical-line visibility
ribbons-visibility-detail = { $kind } / hidden = { $hidden }
    Only local summary visibility changes; no evidence is deleted.
ribbons-help = Inspect this project's historical milestones, including hidden records and current source status.
ribbons-help-hide = Hide the selected ribbon from automatic summary lines; keep its historical evidence.
ribbons-help-show = Show the selected ribbon in partner summaries when local flavor is enabled.
ribbons-help-inspect = Open selected evidence through ordinary local navigation, never resume a task or change its producer implicitly.
ribbons-source-unavailable = The original source is unavailable or ambiguous. Historical details remain here; no replacement source was opened.
ribbons-hidden-tag = [hidden]{ " " }
ribbons-source-evidence = Source / task #{ $task } / capture #{ $excerpt } / { $path }:{ $start }-{ $end }
ribbons-run-evidence = Run / task #{ $task } / { $run }
dex-comparison-unavailable = Current-task comparison unavailable: referenced failure runs are missing or belong to another task. Retained project references are not current verification.
dex-unresolved-new = Unresolved new/unmatched failure / no earlier exact command identity
dex-unresolved-recurring = Unresolved recurring failure / exact command
dex-historical-supported-pass = Historical supported pass after this failure / never current verification
dex-support-retained = retained historical evidence
dex-support-stale = stale support against current source
dex-support-missing = missing reference
dex-support-unavailable = unavailable or incomplete support
dex-support-purged = purged/missing/replaced run output; unavailable
dex-support-different-command = different or unknown command/cwd/executable identity; not supporting this comparison
dex-comparison = Failure { $run }: { $state }
    Ordered command: { $command }
    Cwd: { $cwd }
    Failure source: { $source }; evidence: { $support }
    Failure source identity: { $fingerprint }
    Earlier exact-command failures: { $earlier }
dex-historical-support = Historical diagnosis { $support }: { $diagnosis }
    Passing run { $run }: { $pass }
    Passing source against current observation: { $source }
    Passing source identity: { $fingerprint }
    This historical relation never establishes current verification or causality.
dex-source-unknown = unknown source identity
dex-foreign-support = Other-task historical reference: support was not inspected for this task. Do not infer matching command/source or current availability.
dex-relay-selection = Relay selection: latest task-owned failure { $run }. r opens neutral Umbreon reference review only; navigation never sends or executes.
