//! Formation file I/O and all-or-nothing owned partner staging on the existing writer.
use super::*;
use crate::workspace::{handoff, workflows::Recipe};

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Read {
        path: PathBuf,
    },
    Export {
        path: PathBuf,
        recipe: Recipe,
    },
    Apply {
        expected: Box<Snapshot>,
        roster: Vec<Partner>,
        recipe: Recipe,
        mapping: Vec<Option<i64>>,
    },
}
#[derive(Debug)]
pub enum Response {
    Read(Recipe),
    Exported,
    Applied {
        task: Box<Snapshot>,
        roster: Vec<Partner>,
    },
}

impl State {
    pub(super) fn formation(&mut self, request: Request) -> Result<Response, String> {
        match request {
            Request::Read { path } => {
                let recipe = Recipe::read_file(&path)?;
                if recipe.roles.is_empty() {
                    return Err(
                        "This recipe has no formation roles; use the ordinary recipe desk".into(),
                    );
                }
                Ok(Response::Read(recipe))
            }
            Request::Export { path, recipe } => {
                recipe.validate()?;
                if recipe.roles.is_empty() {
                    return Err("A saved formation needs at least one role".into());
                }
                let bytes =
                    toml::to_string_pretty(&recipe).map_err(|_| "Cannot encode formation")?;
                if bytes.len() > crate::workspace::workflows::MAX_RECIPE_BYTES {
                    return Err("Formation exceeds 64 KiB".into());
                }
                handoff::write_new_bytes(&path, bytes.as_bytes())?;
                Ok(Response::Exported)
            }
            Request::Apply {
                expected,
                roster,
                recipe,
                mapping,
            } => {
                recipe.validate()?;
                if recipe.roles.is_empty() || mapping.len() != recipe.roles.len() {
                    return Err("Preview a complete formation mapping first".into());
                }
                let connection = self.connection.as_mut().ok_or("Storage unavailable")?;
                let tx = connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(|error| error.to_string())?;
                let mut task = load_task(&tx, &self.project, expected.id)?
                    .ok_or("Formation task is no longer available")?;
                if task.revision != expected.revision
                    || task.lead_partner != expected.lead_partner
                    || task.partner_drafts != expected.partner_drafts
                    || task.complete
                    || task.archived
                {
                    return Err("Formation task or drafts changed; preview again".into());
                }
                if partners(&tx)? != roster {
                    return Err("Party changed; reload the formation mapping".into());
                }
                no_active(&tx, Some(task.id))?;
                let mut chosen = std::collections::HashSet::new();
                let mut routes = Vec::new();
                for (slot, target) in recipe.roles.iter().zip(mapping) {
                    let Some(target) = target else { continue };
                    if !chosen.insert(target) {
                        return Err("Each formation role must use a different partner".into());
                    }
                    let member = roster
                        .iter()
                        .find(|member| member.id == target)
                        .ok_or("Mapped partner is no longer present")?;
                    super::partners::idle(&tx, target)?;
                    let form = crate::domain::FORMS
                        .iter()
                        .position(|form| form.name == slot.form)
                        .ok_or("Unknown formation form")?;
                    let draft = task.partner_drafts.entry(target).or_default();
                    // Existing text and caret win. A formation is not permission to replace a draft.
                    let existing = draft
                        .drafts
                        .iter()
                        .any(|d| d.field == EditorField::Request && !d.text.is_empty());
                    if !slot.request.is_empty() && !existing {
                        draft.drafts.retain(|d| d.field != EditorField::Request);
                        draft.drafts.push(Draft {
                            field: EditorField::Request,
                            text: slot.request.clone(),
                            cursor: slot.request.len(),
                        });
                        draft.editor_field = EditorField::Request;
                        draft.editing = true;
                    }
                    if member.form != form {
                        draft.previous_form = Some(member.form);
                        let mut owner = Attribution::of(member);
                        owner.form = form;
                        routes.push(crate::workspace::records::LocalIntent::FormRoute {
                            id: format!("formation:{}:{}:{}", task.id, task.revision + 1, target),
                            owner,
                            from: member.form,
                            to: form,
                            accepted_suggestion: None,
                        });
                    }
                    set_partner_form(&tx, target, form, form == 0)?;
                    if target == task.lead_partner {
                        task.form = form;
                        task.auto_form = form == 0;
                        task.navigation.form_selection = form;
                        task.navigation.previous_form = draft.previous_form;
                    }
                }
                if chosen.is_empty() {
                    return Err("Map at least one role before applying a formation".into());
                }
                task.revision = task
                    .revision
                    .checked_add(1)
                    .filter(|v| *v <= i64::MAX as u64)
                    .ok_or("Task revision exhausted")?;
                task.updated_at = timestamp()?;
                validate(&task)?;
                tx.execute("UPDATE tasks SET form=?1,revision=?2,partner_drafts=?3,navigation=?4,updated_at=?5 WHERE id=?6 AND project=?7",
                    params![task.form as i64, task.revision as i64,
                        bounded_json(&task.partner_drafts, MAX_PARTNER_DRAFTS_BYTES)?,
                        bounded_json(&task.navigation, 1024)?, task.updated_at, task.id, self.project])
                    .map_err(|error| error.to_string())?;
                super::records::save_intents(&tx, &self.data_dir, &task, &routes)?;
                enforce_evidence_quota(&tx, &self.data_dir, self.policy)?;
                let roster = partners(&tx)?;
                tx.commit().map_err(|error| error.to_string())?;
                Ok(Response::Applied {
                    task: Box::new(task),
                    roster,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "formations_tests.rs"]
mod tests;
