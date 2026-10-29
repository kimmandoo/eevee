//! Approval rules (XC01) at the lead's approval boundary. A rule is created
//! only by an explicit `/rule-allow` or `/rule-deny` on a request the user is
//! reviewing; it then answers later matching requests and says so.
use super::*;
use crate::providers::PromptKind;
use crate::workspace::rules::{self, Call, Effect, Expiry, Kind, Rule, WriteOp};

/// A pending lead request reduced to what a rule may match.
enum Pending {
    Command {
        program: std::path::PathBuf,
        sha256: String,
        args: Vec<String>,
        cwd: String,
        run: String,
    },
    Write {
        op: WriteOp,
        path: String,
    },
}

#[derive(Default)]
pub struct State {
    pub selected: usize,
    /// How often each `program first-arg` was presented for review this launch.
    presented: std::collections::BTreeMap<String, (String, u8)>,
}

fn project_relative(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let parts = relative
        .components()
        .map(|part| match part {
            std::path::Component::Normal(name) => name.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    Some(parts.join("/"))
}

impl App {
    pub fn load_rules(&mut self, data_dir: &std::path::Path) {
        self.rules = rules::Store::load(data_dir);
    }

    fn pending_rule_call(&self) -> Option<Pending> {
        if let Some(PromptKind::Permission {
            file: Some(file),
            options,
            ..
        }) = self.planning.request.as_ref().map(|request| &request.kind)
            && options.len() >= 2
            && !file.external
        {
            let op = match file.kind {
                crate::providers::FileReviewKind::Create => WriteOp::Create,
                crate::providers::FileReviewKind::Modify => WriteOp::Edit,
                // Delete and rename always need a fresh review.
                _ => return None,
            };
            return Some(Pending::Write {
                op,
                path: file.path.clone(),
            });
        }
        let (task, partner, _, run) = self.execution.approval_identity()?;
        if task != self.task_id || partner != self.lead.id {
            return None;
        }
        let prepared = self.execution.prepared.as_ref()?;
        let crate::workspace::runner::CommandSpec::Direct { args, .. } = &prepared.spec else {
            return None;
        };
        Some(Pending::Command {
            program: prepared.executable.path.clone(),
            sha256: prepared.executable.sha256.clone(),
            args: args.clone(),
            cwd: project_relative(std::path::Path::new(&self.cwd), &prepared.cwd)?,
            run: run.to_owned(),
        })
    }

    fn rule_for(&self, pending: &Pending) -> Option<Rule> {
        let call = match pending {
            Pending::Command {
                program,
                sha256,
                args,
                cwd,
                ..
            } => Call::Command {
                program,
                program_sha256: sha256,
                args,
                cwd,
            },
            Pending::Write { op, path } => Call::Write { op: *op, path },
        };
        rules::decide(&self.rules.rules, &self.cwd, &call).cloned()
    }

    /// Answers the lead's current request from a matching rule. Never acts
    /// while local effects are unknown, and never on questions.
    pub(super) fn drive_rules(&mut self) -> bool {
        if self.rules.rules.is_empty()
            || self.planning_any_unknown_effects()
            || self.execution_any_unknown()
        {
            return false;
        }
        let Some(pending) = self.pending_rule_call() else {
            return false;
        };
        let command_ready = matches!(pending, Pending::Command { .. })
            && self
                .cooperation
                .command_callbacks
                .contains_key(&self.lead.id)
            && self.execution.confirmation.is_none();
        if matches!(pending, Pending::Command { .. }) && !command_ready {
            return false;
        }
        let Some(rule) = self.rule_for(&pending) else {
            self.rule_hint(&pending);
            return false;
        };
        let subject = match &pending {
            Pending::Command { .. } => self
                .execution
                .prepared
                .as_ref()
                .map(execution::approval_command_summary)
                .unwrap_or_default(),
            Pending::Write { path, .. } => path.clone(),
        };
        let answered = match (&pending, rule.effect) {
            (Pending::Write { .. }, Effect::Allow) => self.planning.answer_permission(1),
            (Pending::Write { .. }, Effect::Deny) => self.planning.answer_permission(0),
            (Pending::Command { .. }, Effect::Allow) => self.execution.auto_approve(),
            (Pending::Command { .. }, Effect::Deny) => {
                self.execution.open_rule_review() && self.command("deny")
            }
        };
        if answered {
            self.notice = format!(
                "{} by rule #{}: {subject} · /rules",
                match rule.effect {
                    Effect::Allow => "Allowed",
                    Effect::Deny => "Denied",
                },
                rule.id
            );
        }
        answered
    }

    /// The third review of the same command prefix suggests a rule once.
    fn rule_hint(&mut self, pending: &Pending) {
        let Pending::Command {
            program, args, run, ..
        } = pending
        else {
            return;
        };
        let key = format!(
            "{} {}",
            program.display(),
            args.first().map_or("", String::as_str)
        );
        let entry = self.rule_state.presented.entry(key).or_default();
        if entry.0 != *run {
            entry.0 = run.clone();
            entry.1 = entry.1.saturating_add(1);
            if entry.1 == 3 {
                self.notice =
                    "Reviewed this command prefix twice before: /rule-allow makes a project rule (shown before saving)."
                        .into();
            }
        }
    }

    fn draft_rule(&self, effect: Effect, argument: &str) -> Result<Rule, String> {
        let pending = self.pending_rule_call().ok_or(
            "Open a lead file create/edit or direct command review first; deletes, renames, shells and selected commits never get rules",
        )?;
        let mut expires = Expiry::Never;
        let mut prefix = None;
        let mut glob = None;
        for token in argument.split_whitespace() {
            match token {
                "launch" => expires = Expiry::ThisLaunch,
                number if number.parse::<usize>().is_ok() => prefix = number.parse().ok(),
                pattern => glob = Some(pattern.to_owned()),
            }
        }
        let kind = match pending {
            Pending::Command {
                program,
                sha256,
                args,
                cwd,
                ..
            } => {
                let keep = prefix.unwrap_or(args.len()).min(args.len());
                Kind::Command {
                    program,
                    program_sha256: sha256,
                    argv_prefix: args[..keep].to_vec(),
                    cwd,
                }
            }
            Pending::Write { op, path } => Kind::Write {
                ops: vec![op],
                paths: vec![glob.unwrap_or(path)],
            },
        };
        Ok(Rule {
            id: 0,
            project: self.cwd.clone(),
            effect,
            kind,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs()),
            expires,
            enabled: true,
        })
    }

    pub(super) fn rules_command(&mut self, name: &str, argument: &str) -> bool {
        match name {
            "rules" => {
                self.rule_state.selected = 0;
                self.open_overlay(Overlay::Rules);
            }
            "rule-allow" | "rule-deny" => {
                let effect = if name == "rule-allow" {
                    Effect::Allow
                } else {
                    Effect::Deny
                };
                match self
                    .draft_rule(effect, argument)
                    .and_then(|rule| {
                        let sentence = rule.sentence();
                        self.rules.add(rule).map(|id| (id, sentence))
                    })
                    .and_then(|saved| self.rules.save().map(|()| saved))
                {
                    Ok((id, sentence)) => {
                        self.notice =
                            format!("Rule #{id}: {sentence}. Applies now; /rules revokes.");
                    }
                    Err(error) => self.notice = format!("No rule saved: {error}"),
                }
            }
            "rule-revoke" => {
                let id = argument.trim().trim_start_matches('#').parse::<u32>();
                self.notice = match id {
                    Ok(id) if self.rules.revoke(id) => match self.rules.save() {
                        Ok(()) => {
                            format!("Rule #{id} revoked; the next matching request is reviewed.")
                        }
                        Err(error) => {
                            format!("Rule #{id} revoked for this launch; not saved: {error}")
                        }
                    },
                    _ => "Use /rule-revoke <id> with an id from /rules".into(),
                };
            }
            _ => return false,
        }
        true
    }

    pub(super) fn rules_key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.request_quit();
            return true;
        }
        let last = self.project_rules().len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_overlay(),
            KeyCode::Up => self.rule_state.selected = self.rule_state.selected.saturating_sub(1),
            KeyCode::Down => self.rule_state.selected = (self.rule_state.selected + 1).min(last),
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(id) = self
                    .project_rules()
                    .get(self.rule_state.selected)
                    .map(|rule| rule.id)
                {
                    self.rules_command("rule-revoke", &id.to_string());
                    self.rule_state.selected = self.rule_state.selected.min(last.saturating_sub(1));
                }
            }
            KeyCode::Char(' ') => {
                if let Some(id) = self
                    .project_rules()
                    .get(self.rule_state.selected)
                    .map(|rule| rule.id)
                    && let Some(rule) = self.rules.rules.iter_mut().find(|rule| rule.id == id)
                {
                    rule.enabled = !rule.enabled;
                    let state = if rule.enabled { "enabled" } else { "disabled" };
                    self.notice = match self.rules.save() {
                        Ok(()) => format!("Rule #{id} {state}."),
                        Err(error) => {
                            format!("Rule #{id} {state} for this launch; not saved: {error}")
                        }
                    };
                }
            }
            _ => return false,
        }
        true
    }

    pub fn project_rules(&self) -> Vec<&Rule> {
        self.rules
            .rules
            .iter()
            .filter(|rule| rule.project == self.cwd)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_relative_paths_refuse_escapes() {
        let root = std::path::Path::new("/p");
        assert_eq!(
            project_relative(root, std::path::Path::new("/p")).as_deref(),
            Some("")
        );
        assert_eq!(
            project_relative(root, std::path::Path::new("/p/a/b")).as_deref(),
            Some("a/b")
        );
        assert_eq!(project_relative(root, std::path::Path::new("/q")), None);
        assert_eq!(
            project_relative(root, std::path::Path::new("/p/../q")),
            None
        );
    }

    #[test]
    fn rules_need_a_pending_reviewable_request() {
        let mut app = App::new("/p".into(), 120, 36);
        assert!(app.rules_command("rule-allow", ""));
        assert!(app.notice.contains("No rule saved"), "{}", app.notice);
        assert!(app.rules.rules.is_empty());
        assert!(app.rules_command("rule-revoke", "x"));
        assert!(app.notice.contains("/rule-revoke <id>"));
    }
}
