tm-menu-controls = i Import TM | e Export TM | t Teach
    Arrows choose | Enter use | d Details | r Reminder | Esc
tm-import-path = Import TM: literal file path
tm-export-path = Export TM: NEW file path (never overwrite)
tm-path-controls = Type literal path | Enter continue
    Esc cancel | no effect
tm-path-boundary = No links, secret paths, traversal or expansion; 64 KiB TOML only.
tm-import-title = Import TM / choose local partner
tm-import-summary = { $name } / { $kind } / { $form }
tm-import-partner = Learn for: { $partner }
tm-import-controls = Up/Down partner | Enter name/scope
    PgUp/PgDn steps | Esc cancel
tm-import-boundary = Explicit local learning only. No old teacher, results or grants. Missing pins stay unavailable until ordinary capture review at use.
tm-import-action = Import TM
tm-export-title = Create-only TM export / no execution
tm-export-details = Output: { $path }
    { $definition }
    Semantics only; no local IDs, teacher, evidence, sessions or approvals.
tm-export-controls = [Stay] | Create NEW file | Enter Stay | y Create
    Left/Right choose | PgUp/PgDn details | Esc cancel
tm-export-armed = Stay / [Create NEW file] / Enter Create
    Left/Right choose / PgUp/PgDn details / Esc cancel
tm-path-refused = Path insertion refused: single literal line, at most 4096 UTF-8 bytes.
tm-export-saved = Exported to a new TM file; no approvals or evidence included.
cmd-move-import = Import a portable TM file; review local learning
cmd-move-export = Review exporting a move to a new TM file
tm-free-slot = Free slot { $slot } of 4 / explicit local learning
tm-semantic-old = OLD / move #{ $id }, revision { $revision }
tm-semantic-new = PROPOSED / exact local definition
tm-semantic-changed = Semantics changed; old usage stays with the original record. Taught copies remain independent.
tm-semantic-equal = Same action semantics (ordered argv; canonical pin sets). This is not new execution evidence.
tm-semantic-name-only = Name-only edit; steps, scope, provenance and historical usage stay unchanged.
tm-semantic-boundary = Learning changes only this local inventory. No run, request, pin selection or permission. Cancel preserves inventory and the invoking draft/caret.
tm-semantic-scope = Local recipient: { $partner } / partner #{ $id } / { $form }
    Scope: { $scope } / local name: { $name }
tm-semantic-argument = argv[{ $index }] (zero-based): { $old } -> { $new }
tm-semantic-cwd = cwd: { $old } -> { $new }
tm-semantic-kind = Action kind: { $old } -> { $new }
tm-semantic-pins = Pin selectors changed; exact old/new paths and ranges follow.
tm-semantic-instruction = User instruction changed; exact old/new instruction bytes follow.
tm-semantic-goal = Goal changed; exact old/new goal bytes follow.
tm-semantic-criterion = criterion[{ $index }] (zero-based): { $old } -> { $new }
tm-semantic-absent = <absent>
tm-semantic-stale = Review is stale or unavailable. Cancel and open a fresh review; nothing changed.
tm-semantic-expose = Confirm local change: PgDn all old/new details.
    Enter stays / Esc cancels; hidden scope cannot approve.
tm-semantic-slot = Replaces local slot { $slot } of 4 / move #{ $id } revision { $revision }
tm-semantic-teacher = Local teaching source: move #{ $id }, revision { $revision }. The source stays unchanged; this is an independent copy.
tm-semantic-fresh = New local definition; no existing record is changed. No prior execution evidence is inherited.
tm-semantic-literals = Quoted values are literal data, not shell syntax: "" is an empty value; <absent> is missing. Escapes preserve exact whitespace.
tm-semantic-reminder = Relearn original move #{ $id }, revision { $revision }, with its own retained history and identical steps.
