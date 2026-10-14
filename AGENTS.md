# Rules

Commit per each query session.
Commit messages must follow `type(scope): subject`, for example `feat(trading): harden futures runtime`.
When strategy code, strategy defaults, or strategy selection behavior changes, run the relevant backtest before completion and report the result.

# Changelog

When a feature is added, a bug is fixed, or any breaking change is introduced, upsert to the CHANGELOG.md file.
The changelog should be written in the past tense and follow the same format as the commit messages.
Group changelog entries under reverse-chronological `## YYYY-MM-DD` headings using the date of the change.
Add new entries under the current date heading, creating it when needed.

## Product design rules

- `docs/DESIGN_RULES.md` is the binding implementation contract for this product. Keep code, tests, and future design changes consistent with it; update the document when an intentional contract changes.
- Record implementation failures, environment issues, and their resolutions in
  `docs/TROUBLESHOOTING.md` during the same session.

## Troubleshooting Documentation Guidelines

If a non-trivial issue, build error, unexpected bug, or breaking dependency issue occurs and is resolved during the task, document it in `docs/TROUBLESHOOTING.md`.

### When to Record

- Recurring or unexpected build/runtime errors.
- Non-obvious workarounds, environment configuration issues, or third-party library quirks.
- Root causes that required multi-step debugging. *(Do not record trivial typos, transient network glitches, or standard iterative code changes.)*

### Format Template

Each entry must follow this structure:

```
### [YYYY-MM-DD] Short, descriptive issue title

- **Context / Symptom:** Brief description of what went wrong, including exact error messages or unexpected behavior.
- **Root Cause:** Why the error occurred.
- **Solution:** Concrete steps or code changes applied to fix it.
- **Prevention / Reference:** (Optional) Relevant links, CLI commands, or tips to avoid recurrence.

## Intermediate checkpoints and continuation

- Maintain `docs/WORK_CHECKPOINT.md` as the repository handoff record for work
  that may continue in a later query session.
- At the start of a resumed session, read `docs/WORK_CHECKPOINT.md`, this file,
  `task.md`, and the active implementation-plan section before changing code.
- Confirm `git status --short --branch` and the latest commit. Continue the
  exact active task and recorded plan step; never infer that a task is complete
  from a commit title or jump to the next task because the working tree is
  clean.
- Before ending a session, update the checkpoint with the active task, exact
  next action, changed files, verification commands and results, and blockers.
- Commit the checkpoint together with the session's changes. If work is still
  incomplete, keep the task active and record the next RED/GREEN or diagnostic
  step. Mark completion only after the plan's required verification passes.
```

## Intermediate checkpoints and continuation

- Maintain `docs/WORK_CHECKPOINT.md` as the repository handoff record for work
  that may continue in a later query session.
- At the start of a resumed session, read `docs/WORK_CHECKPOINT.md`, this file,
  `task.md`, and the active implementation-plan section before changing code.
- Confirm `git status --short --branch` and the latest commit. Continue the
  exact active task and recorded plan step; never infer that a task is complete
  from a commit title or jump to the next task because the working tree is
  clean.
- Before ending a session, update the checkpoint with the active task, exact
  next action, changed files, verification commands and results, and blockers.
- Commit the checkpoint together with the session's changes. If work is still
  incomplete, keep the task active and record the next RED/GREEN or diagnostic
  step. Mark completion only after the plan's required verification passes.
