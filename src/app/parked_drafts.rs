//! Explicit text parking and copying. No provider/session/execution integration.
use super::*;
use crate::store::parked_drafts::{self, Entry};
use std::cell::Cell;

#[derive(Clone)]
struct Pending {
    task: i64,
    partner: i64,
    revision: u64,
    text: String,
    cursor: usize,
    entry: Entry,
}
#[derive(Clone)]
pub struct Review {
    pub entry: Entry,
    pub discard: bool,
    pub original: String,
    task: i64,
    partner: i64,
    revision: u64,
    cursor: usize,
}
#[derive(Default)]
pub struct State {
    pub selected: usize,
    pub inspect: bool,
    pub scroll: u16,
    pub review: Option<Review>,
    pub affirmative: bool,
    pub viewed: Cell<usize>,
    pub complete: Cell<bool>,
    pending: Option<Pending>,
}
impl State {
    pub fn invalidate(&mut self) {
        self.affirmative = false;
        self.viewed.set(0);
        self.complete.set(false);
    }
}
impl App {
    pub fn parked_entries(&self) -> impl Iterator<Item = &Entry> {
        self.workflows
            .parked_drafts
            .iter()
            .filter(|e| e.partner == self.lead.id)
    }
    fn selected_parked_entry(&self) -> Option<Entry> {
        self.parked_entries()
            .nth(self.parked_drafts.selected)
            .cloned()
    }
    fn parked_request(&self) -> (&str, usize) {
        if self.composer_is_request() {
            (&self.input, self.cursor)
        } else {
            self.drafts
                .iter()
                .find(|draft| draft.field == store::EditorField::Request)
                .map_or(("", 0), |draft| (draft.text.as_str(), draft.cursor))
        }
    }
    pub fn parked_draft_command(&mut self, name: &str) -> bool {
        if !matches!(name, "drafts" | "park-draft") {
            return false;
        }
        if self.transitioning
            || self.quit_after_save
            || self.reset_after_save
            || self.confirmation.is_some()
        {
            self.notice =
                "Finish the current ownership decision before opening parked drafts.".into();
            return true;
        }
        if name == "drafts" {
            self.parked_drafts.review = None;
            self.parked_drafts.inspect = false;
            self.parked_drafts.scroll = 0;
            self.parked_drafts.invalidate();
            self.open_overlay(Overlay::ParkedDrafts);
            return true;
        }
        if self.width < 60 || self.height < 18 {
            self.notice = "Resize to at least 60x18 before parking a Request.".into();
            return true;
        }
        if matches!(self.storage, StorageState::Loading)
            || self.archived
            || self.pending_request.is_some()
        {
            self.notice =
                "Wait for task loading or the pending explicit Send decision before parking text."
                    .into();
            return true;
        }
        if !self.editing
            || self.editor_field != store::EditorField::Request
            || self.input.is_empty()
        {
            self.notice = "Type a nonempty Request before parking it; nothing was sent.".into();
            return true;
        }
        if self.parked_drafts.pending.is_some() {
            self.notice = "A parked draft is awaiting storage acknowledgment; keep the recoverable Request or retry storage.".into();
            return true;
        }
        let Some(id) = self
            .workflows
            .parked_drafts
            .iter()
            .map(|e| e.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
        else {
            self.notice = "Parked draft identity capacity reached.".into();
            return true;
        };
        let entry = Entry {
            id,
            partner: self.lead.id,
            text: self.input.clone(),
        };
        self.workflows.parked_drafts.push(entry.clone());
        if let Err(error) = parked_drafts::validate(&self.workflows.parked_drafts) {
            self.workflows.parked_drafts.pop();
            self.notice = error;
            return true;
        }
        self.changed_task();
        self.parked_drafts.pending = Some(Pending {
            task: self.task_id,
            partner: self.lead.id,
            revision: self.revision,
            text: self.input.clone(),
            cursor: self.cursor,
            entry,
        });
        self.notice = "Parking locally; the exact Request stays recoverable until storage acknowledges it. Nothing was sent.".into();
        true
    }
    pub(super) fn parked_draft_saved(&mut self, snapshot: &store::Snapshot) {
        let Some(pending) = self.parked_drafts.pending.as_ref() else {
            return;
        };
        if pending.task != 0 && pending.task != snapshot.id || snapshot.revision < pending.revision
        {
            return;
        }
        let acknowledged = snapshot
            .workflows
            .parked_drafts
            .iter()
            .any(|e| e == &pending.entry);
        let clear = acknowledged
            && snapshot.revision == pending.revision
            && self.revision == pending.revision
            && self.task_id == snapshot.id
            && self.lead.id == pending.partner
            && self.editing
            && self.editor_field == store::EditorField::Request
            && self.input == pending.text
            && self.cursor == pending.cursor;
        self.parked_drafts.pending = None;
        if clear {
            self.input.clear();
            self.cursor = 0;
            self.sources.clear_wire_request();
            self.edit_original.clear();
            self.drafts
                .retain(|d| d.field != store::EditorField::Request);
            self.changed_task();
            self.notice =
                "Request parked locally. Nothing will run until an ordinary explicit Send.".into();
        } else {
            self.notice = if acknowledged {
                "Draft parked; the newer or changed Request and caret were kept. Nothing was sent."
            } else {
                "Storage masked or changed parked text; the original Request was kept recoverable. Inspect before discarding it."
            }.into();
        }
    }
    fn stage_parked_text(&mut self, entry: &Entry) {
        self.close_overlay();
        self.open_editor(store::EditorField::Request);
        self.input.clone_from(&entry.text);
        self.sources.clear_wire_request();
        self.cursor = self.input.len();
        self.parked_drafts.review = None;
        self.changed_task();
        self.notice = "Parked text copied to Request; tray entry retained. Only explicit Send prepares current context and fresh authority.".into();
    }
    fn begin_parked_review(&mut self, entry: Entry, discard: bool) {
        if self.width < 60 || self.height < 18 {
            self.notice = "Resize to at least 60x18 before restoring or discarding text.".into();
            return;
        }
        if !discard && self.editing && self.editor_field != store::EditorField::Request {
            self.notice =
                "Return to the Request editor before restoring text; the current editor was kept."
                    .into();
            return;
        }
        let (original, cursor) = self.parked_request();
        let original = original.to_owned();
        if !discard && original.is_empty() {
            self.stage_parked_text(&entry);
            return;
        }
        self.parked_drafts.review = Some(Review {
            entry,
            discard,
            original,
            task: self.task_id,
            partner: self.lead.id,
            revision: self.revision,
            cursor,
        });
        self.parked_drafts.scroll = 0;
        self.parked_drafts.invalidate();
    }
    pub(super) fn parked_draft_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Esc {
            if self.parked_drafts.review.take().is_some() || self.parked_drafts.inspect {
                self.parked_drafts.inspect = false;
                self.parked_drafts.scroll = 0;
                self.parked_drafts.invalidate();
            } else {
                self.close_overlay();
            }
            return true;
        }
        if let Some(review) = self.parked_drafts.review.clone() {
            match key.code {
                KeyCode::Down | KeyCode::PageDown => {
                    self.parked_drafts.scroll = self.parked_drafts.scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::PageUp => {
                    self.parked_drafts.scroll = self.parked_drafts.scroll.saturating_sub(1)
                }
                KeyCode::Left => self.parked_drafts.affirmative = false,
                KeyCode::Right
                    if self.parked_drafts.complete.get()
                        && self.width >= 60
                        && self.height >= 18 =>
                {
                    self.parked_drafts.affirmative = true
                }
                KeyCode::Enter => {
                    if !self.parked_drafts.affirmative {
                        self.parked_drafts.review = None;
                        return true;
                    }
                    let (current, cursor) = self.parked_request();
                    if !self.parked_drafts.complete.get()
                        || self.width < 60
                        || self.height < 18
                        || review.task != self.task_id
                        || review.partner != self.lead.id
                        || review.revision != self.revision
                        || review.cursor != cursor
                        || review.original != current
                        || !self.workflows.parked_drafts.contains(&review.entry)
                    {
                        self.parked_drafts.review = None;
                        self.notice = "Draft or owner changed; review cancelled without replacing or discarding text.".into();
                    } else if review.discard {
                        self.workflows.parked_drafts.retain(|e| e != &review.entry);
                        self.parked_drafts.review = None;
                        self.parked_drafts.inspect = false;
                        self.parked_drafts.selected = self
                            .parked_drafts
                            .selected
                            .min(self.parked_entries().count().saturating_sub(1));
                        self.changed_task();
                        self.notice = "Parked draft discard pending local storage. This is not secure erasure.".into();
                    } else {
                        self.stage_parked_text(&review.entry);
                    }
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Up if !self.parked_drafts.inspect => {
                self.parked_drafts.selected = self.parked_drafts.selected.saturating_sub(1)
            }
            KeyCode::Down if !self.parked_drafts.inspect => {
                self.parked_drafts.selected = (self.parked_drafts.selected + 1)
                    .min(self.parked_entries().count().saturating_sub(1))
            }
            KeyCode::Enter => {
                self.parked_drafts.inspect = true;
                self.parked_drafts.scroll = 0;
            }
            KeyCode::Down | KeyCode::PageDown => {
                self.parked_drafts.scroll = self.parked_drafts.scroll.saturating_add(1)
            }
            KeyCode::Up | KeyCode::PageUp => {
                self.parked_drafts.scroll = self.parked_drafts.scroll.saturating_sub(1)
            }
            KeyCode::Char('r') | KeyCode::Char('d') => {
                if let Some(entry) = self.selected_parked_entry() {
                    self.begin_parked_review(entry, key.code == KeyCode::Char('d'));
                }
            }
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests;
