//! Execution relays reuse source capture and one-shot command admission, never old grants.
use super::{App, Overlay, StorageState, execution, relays};
use crate::{
    store,
    workspace::{locations::Location, runner},
};
use serde::ser::{SerializeSeq, Serializer};

pub fn command_draft(spec: &runner::CommandSpec) -> String {
    match spec {
        runner::CommandSpec::Direct { program, args } => {
            let mut bytes = Vec::new();
            let mut serializer = serde_json::Serializer::new(&mut bytes);
            let mut sequence = serializer
                .serialize_seq(Some(args.len() + 1))
                .expect("argv sequence");
            sequence.serialize_element(program).expect("program JSON");
            for arg in args {
                sequence.serialize_element(arg).expect("argument JSON");
            }
            sequence.end().expect("argv JSON");
            String::from_utf8(bytes).expect("JSON is UTF-8")
        }
        runner::CommandSpec::Shell {
            executable,
            dialect,
            text,
        } => serde_json::json!({"shell": executable, "dialect": match dialect {
                runner::ShellDialect::Posix => "posix",
                runner::ShellDialect::Cmd => "cmd",
                runner::ShellDialect::PowerShell => "powershell",
            }, "text": text})
        .to_string(),
    }
}

/// Journal redaction is irreversible: a masked command cannot be replayed exactly.
pub fn retained_rerun_spec(
    record: &runner::RunRecord,
) -> Result<&runner::CommandSpec, &'static str> {
    let masked = |text: &str| text.contains("[REDACTED]");
    let missing = match &record.prepared.spec {
        runner::CommandSpec::Direct { program, args } => {
            masked(program) || args.iter().any(|arg| masked(arg))
        }
        runner::CommandSpec::Shell {
            executable, text, ..
        } => masked(executable) || masked(text),
    };
    if missing {
        Err(
            "Retained command contains masked data; its exact argv cannot be recovered. Author a new command explicitly.",
        )
    } else {
        Ok(&record.prepared.spec)
    }
}

impl App {
    pub(super) fn rerun_stale_check(&mut self) {
        let stale = |record: &&runner::RunRecord| {
            record.prepared.task_id == self.task_id
                && (record.revision_moving
                    || record.after.fingerprint.is_some()
                        && record.after.fingerprint != self.execution.latest.fingerprint)
        };
        let run = self
            .execution
            .selected_record()
            .filter(stale)
            .or_else(|| self.execution.history.iter().find(stale))
            .map(|record| record.prepared.run_id.clone());
        let result = run
            .ok_or_else(|| "Refresh sources and select a stale retained check first.".to_owned())
            .and_then(|run| self.stage_execution_rerun(run));
        if let Err(error) = result {
            self.notice = error;
        }
    }

    pub(super) fn pin_execution_location(
        &mut self,
        location: Location,
        run: String,
    ) -> Result<(), String> {
        if self.width < 60 || self.height < 18 {
            return Err("Resize to 60x18 before reviewing a capture.".into());
        }
        if !self
            .execution
            .history
            .iter()
            .chain(self.execution.active.iter())
            .any(|record| record.prepared.task_id == self.task_id && record.prepared.run_id == run)
        {
            return Err("The location's execution does not belong to the current task.".into());
        }
        let report = self
            .repository
            .as_ref()
            .ok_or("Refresh repository evidence before pinning a location.")?;
        self.sources
            .inspect_location(self.task_id, report, &location, run)?;
        self.open_overlay(Overlay::Sources);
        Ok(())
    }

    pub(super) fn stage_execution_rerun(&mut self, run: String) -> Result<(), String> {
        let record = self
            .execution
            .history
            .iter()
            .find(|record| record.prepared.run_id == run && record.prepared.task_id == self.task_id)
            .ok_or("Select a retained run belonging to this task.")?;
        let spec = retained_rerun_spec(record).map_err(str::to_owned)?;
        if !self.execution.latest.complete
            || !self.execution.latest.stable
            || self.execution.latest.fingerprint.is_none()
            || !record.revision_moving
                && (record.after.fingerprint.is_none()
                    || record.after.fingerprint == self.execution.latest.fingerprint)
        {
            return Err("Refresh sources and select a stale run; no command was prepared.".into());
        }
        let other_target = self.party.iter().any(|partner| {
            partner.id != self.lead().id && partner.form == 2 && !self.partner_busy(partner.id)
        });
        if !other_target {
            return self.execution_rerun(&run);
        }
        self.stage_relay(relays::RelayRequest {
            target_form: 2,
            text: format!("Rerun retained run {} with its exact argv and cwd. This creates new evidence; the prior run is unchanged.", run),
            paths: Vec::new(),
            source: relays::RelaySource::Rerun(run),
            command: Some(spec.clone()),
        })
    }

    /// Explicit rerun intent prepares fresh identities; normal command approval is still required.
    pub fn execution_rerun(&mut self, run_id: &str) -> Result<(), String> {
        if let Some(reason) = self.work_capacity_reason(self.lead().id) {
            return Err(reason);
        }
        let record = self
            .execution
            .history
            .iter()
            .find(|record| {
                record.prepared.task_id == self.task_id && record.prepared.run_id == run_id
            })
            .ok_or("The retained source run is unavailable.")?;
        let spec = retained_rerun_spec(record).map_err(str::to_owned)?;
        let root = self
            .repository
            .as_ref()
            .ok_or("Repository observation unavailable.")?
            .repository
            .root
            .as_path();
        if record.prepared.cwd != root || std::path::Path::new(&self.cwd) != root {
            return Err(
                "The original cwd differs from this task's execution root; nothing was changed."
                    .into(),
            );
        }
        let command = command_draft(spec);
        self.request_navigation(execution::Action::SelectForm(2));
        if self.form() != 2 {
            return Err(
                "Form switch was refused; the retained run and command are unchanged.".into(),
            );
        }
        let partner = self.lead().id;
        self.workflows
            .command_relays
            .retain(|origin| origin.partner != partner);
        self.workflows
            .command_relays
            .push(crate::workspace::workflows::CommandRelay {
                partner,
                run_id: run_id.into(),
            });
        self.command.clone_from(&command);
        self.drafts
            .retain(|draft| draft.field != store::EditorField::Command);
        self.drafts.push(store::Draft {
            field: store::EditorField::Command,
            cursor: command.len(),
            text: command.clone(),
        });
        self.pending_command = Some((command, self.attribution()));
        self.changed_task();
        self.flush_save();
        self.execution.open(self.task_id);
        self.open_overlay(Overlay::Execution);
        self.notice =
            "Saving exact rerun command before a new one-shot approval; no execution yet.".into();
        Ok(())
    }

    /// A relayed command keeps provenance only while it still matches the cited run.
    pub(super) fn prepare_command_relay(&mut self, command: &str, owner: store::Attribution) {
        if let Some(reason) = self.work_capacity_reason(owner.partner) {
            self.notice = format!("{reason} Saved command draft retained; nothing prepared.");
            return;
        }
        let origin = self
            .workflows
            .command_relays
            .iter()
            .find(|origin| origin.partner == owner.partner)
            .and_then(|origin| {
                self.execution_state(owner.partner)?
                    .history
                    .iter()
                    .find(|record| {
                        record.prepared.task_id == self.task_id
                            && record.prepared.run_id == origin.run_id
                            && execution::parse_command(command)
                                .is_ok_and(|spec| spec == record.prepared.spec)
                            && record.prepared.cwd == std::path::Path::new(&self.cwd)
                    })
            })
            .map(|record| record.prepared.run_id.clone());
        let task = self.task_id;
        let partner = owner.partner;
        let shown = partner == self.lead().id && matches!(self.storage, StorageState::Saved);
        let state = self.execution_state_mut(partner);
        if let Some(origin) = origin {
            state.prepare_rerun_for(task, owner, command, origin);
        } else {
            state.prepare_command_for(task, Some(owner), command);
        }
        if shown {
            state.shown = true;
        }
    }
}
