use super::State;
use crate::store;

impl State {
    pub fn journal_started(&mut self) {
        if let Some(request) = self.pending_journal.pop_front() {
            self.journal_inflight.push_back(request);
        }
    }

    pub fn journal_event(&mut self, task_id: i64, result: Result<store::JournalResponse, String>) {
        let Some(request) = self.journal_inflight.pop_front() else {
            self.journal_notice = "Unsolicited journal acknowledgement ignored.".into();
            return;
        };
        let correlated = request.task_id == task_id
            && match (&request.operation, &result) {
                (
                    store::JournalOperation::Put(expected),
                    Ok(store::JournalResponse::Saved(record)),
                ) => {
                    expected.id == record.id
                        && expected.agent == record.agent
                        && expected.generation == record.generation
                        && expected.thread_id == record.thread_id
                        && expected.turn_id == record.turn_id
                        && expected.item_id == record.item_id
                        && expected.complete == record.complete
                }
                (store::JournalOperation::List, Ok(store::JournalResponse::Records(_)))
                | (store::JournalOperation::Purge, Ok(store::JournalResponse::Purged))
                | (_, Err(_)) => true,
                _ => false,
            };
        if !correlated {
            self.failed_journal.push_back(request);
            self.journal_notice = "Uncorrelated journal acknowledgement refused; retained operation requires storage-only retry.".into();
            return;
        }
        match result {
            Ok(store::JournalResponse::Saved(_)) => {
                self.journal_notice =
                    "Provider-origin journal committed; not local execution evidence.".into();
            }
            Ok(store::JournalResponse::Records(records)) if task_id == self.task => {
                self.journal_records = records;
                self.journal_selected = self
                    .journal_selected
                    .min(self.journal_records.len().saturating_sub(1));
                self.scroll = 0;
                self.scroll_max = 0;
                self.journal_notice =
                    "Retained provider observations, with actual origin and coverage.".into();
            }
            Ok(store::JournalResponse::Purged) if task_id == self.task => {
                self.journal_records.clear();
                self.journal_selected = 0;
                self.scroll = 0;
                self.scroll_max = 0;
                self.failed_journal
                    .retain(|request| request.task_id != task_id);
                self.journal_notice = "This task's local provider observations purged; provider history and exports remain.".into();
            }
            Ok(_) => {}
            Err(error) => {
                self.failed_journal.push_back(request);
                self.journal_notice = format!(
                    "Provider journal save/read failed: {}. r retries storage only; never replays a turn.",
                    crate::workspace::handoff::sanitize(&error)
                );
            }
        }
    }

    pub fn journal_failed(&self) -> bool {
        !self.failed_journal.is_empty()
    }

    pub fn retry_journal(&mut self) {
        self.pending_journal.extend(self.failed_journal.drain(..));
        self.journal_notice =
            "Retrying retained journal operation only; no provider request.".into();
    }

    pub(super) fn queue_record(&mut self, task_id: i64, mut record: store::ProviderRecord) {
        record.partner = self.owner().cloned();
        if self.pending_journal.len() + self.journal_inflight.len() + self.failed_journal.len()
            >= 128
        {
            self.journal_notice = "Provider journal queue limit reached; further observations cannot substantiate retained evidence.".into();
            return;
        }
        self.pending_journal.push_back(store::JournalRequest {
            task_id,
            operation: store::JournalOperation::Put(Box::new(record)),
        });
    }
}
