use super::{ModelPickerFocus, ModelSelection, Picker, State};

impl State {
    pub fn selection_label(&self) -> String {
        format!(
            "{} / requested model {} / requested effort {} / role {:?} / observed session model: {} (catalogue availability is not entitlement)",
            self.agent.label(),
            self.selection
                .model
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            self.selection
                .effort
                .as_deref()
                .unwrap_or("Default (provider configuration)"),
            self.operation,
            self.observed_model.as_deref().unwrap_or("unobserved")
        )
    }

    pub fn reviewed_selection(&self) -> &ModelSelection {
        &self.request_selection
    }

    pub fn picker_selection(&self) -> &ModelSelection {
        self.pending_model.as_ref().unwrap_or(&self.selection)
    }

    pub fn can_back_to_models(&self) -> bool {
        self.picker == Some(Picker::Effort)
            && self.pending_model.is_some()
            && self.model_picker_return.is_some()
            && self.selection_change.is_none()
    }

    pub(super) fn back_to_models(&mut self) {
        let Some(focus) = self.model_picker_return.take() else {
            return;
        };
        let candidate = self.pending_model.take();
        self.picker = Some(Picker::Model);
        self.review_details = false;
        self.filter_picker();
        self.picker_selected = match focus {
            ModelPickerFocus::Default => 0,
            ModelPickerFocus::Custom => self.picker_len() - 2,
            ModelPickerFocus::Model => candidate
                .and_then(|selection| selection.model)
                .and_then(|id| {
                    self.picker_models
                        .iter()
                        .position(|index| self.catalogues[&self.agent].models[*index].id == id)
                        .map(|index| index + 1)
                })
                .unwrap_or(self.picker_len() - 2),
        };
        self.notice = "Choose a model, then reasoning level. Esc keeps current settings.".into();
    }

    /// Exact IDs are not restricted to the cached discovery catalogue.
    pub fn select_model_id(&mut self, id: &str) -> bool {
        if id.is_empty() || id.len() > 256
            || id.chars().any(|ch| ch.is_whitespace() || ch.is_control()
                || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
            || crate::workspace::privacy::redact(id) != id
        {
            self.notice = "Use an exact model ID: 1–256 UTF-8 bytes, no whitespace, control, bidi or credential-like text. Nothing was changed.".into();
            return false;
        }
        let effort = if self.selection.model.as_deref() == Some(id) {
            self.selection.effort.clone()
        } else {
            None
        };
        self.auto_models = false;
        self.auto_picker = false;
        self.catalogue_dismissed = true;
        self.custom_model = None;
        self.choose_model(ModelSelection {
            model: Some(id.into()),
            effort,
        });
        true
    }

    /// Supported values use exact provider bytes; `default` resets provider configuration.
    pub fn select_effort_value(&mut self, value: &str) -> bool {
        let effort = self
            .effort_model()
            .and_then(|model| model.efforts.iter().find(|effort| effort.as_str() == value));
        let effort = match effort {
            Some(effort) => Some(effort.clone()),
            None if value == "default" => None,
            None => {
                self.notice = "Unsupported reasoning level. Choose an advertised value or default; nothing was changed.".into();
                return false;
            }
        };
        let mut selection = self.picker_selection().clone();
        selection.effort = effort;
        self.auto_models = false;
        self.auto_picker = false;
        self.catalogue_dismissed = true;
        self.choose_selection(selection);
        true
    }

    pub(super) fn effort_model(&self) -> Option<&crate::providers::ModelOption> {
        let catalogue = self.catalogues.get(&self.agent)?;
        let id = self
            .picker_selection()
            .model
            .as_ref()
            .or(catalogue.current.as_ref())?;
        catalogue.models.iter().find(|model| &model.id == id)
    }

    /// Opens a local picker on the exact current value, without granting a connection.
    pub(super) fn open_picker(&mut self, picker: Picker) {
        self.picker = Some(picker);
        self.picker_query = super::Answer::default();
        self.custom_model = None;
        self.pending_model = None;
        self.model_picker_return = None;
        self.review_details = false;
        self.filter_picker();
        self.scroll = 0;
        self.picker_selected = match self.picker_current() {
            Some(index) => index,
            None if picker == Picker::Model => self.picker_len() - 1,
            None => 0,
        };
    }

    /// Filtering is computed only when input or catalogue changes, not for each rendered row.
    pub(super) fn filter_picker(&mut self) {
        self.picker_models.clear();
        if let Some(catalogue) = self.catalogues.get(&self.agent) {
            let query = self.picker_query.text.to_lowercase();
            let matches = |text: &str| {
                if query.is_empty() {
                    true
                } else if text.is_ascii() {
                    text.as_bytes()
                        .windows(query.len())
                        .any(|part| part.eq_ignore_ascii_case(query.as_bytes()))
                } else {
                    // No per-model lowercase allocation; names and query are bounded.
                    text.char_indices().any(|(start, _)| {
                        let mut chars = text[start..].chars().flat_map(char::to_lowercase);
                        query.chars().all(|ch| chars.next() == Some(ch))
                    })
                }
            };
            // Ranking shares the same case folding, without copying provider IDs or names.
            let rank = |text: &str| {
                if text.is_ascii() {
                    if text.eq_ignore_ascii_case(&query) {
                        0
                    } else if text
                        .as_bytes()
                        .get(..query.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(query.as_bytes()))
                    {
                        1
                    } else {
                        2
                    }
                } else {
                    let mut chars = text.chars().flat_map(char::to_lowercase);
                    if !query.chars().all(|ch| chars.next() == Some(ch)) {
                        2
                    } else if chars.next().is_none() {
                        0
                    } else {
                        1
                    }
                }
            };
            self.picker_models.extend(
                catalogue
                    .models
                    .iter()
                    .enumerate()
                    .filter(|(_, model)| matches(&model.name) || matches(&model.id))
                    .map(|(index, _)| index),
            );
            if !query.is_empty() {
                self.picker_models.sort_unstable_by_key(|index| {
                    let model = &catalogue.models[*index];
                    (rank(&model.id).min(rank(&model.name)), *index)
                });
            }
        }
        self.picker_selected = if self.picker_models.is_empty() { 0 } else { 1 };
        self.scroll = 0;
    }

    pub fn picker_matches(&self) -> usize {
        self.picker_models.len()
    }

    pub fn catalogue_error(&self) -> Option<&str> {
        self.catalogue_errors.get(&self.agent).map(String::as_str)
    }

    pub fn catalogue_loading(&self) -> bool {
        self.inspect_models && (self.active || self.awaiting_preview || self.preview.is_some())
    }

    pub(super) fn picker_model(&self, index: usize) -> Option<&crate::providers::ModelOption> {
        let model = *self.picker_models.get(index.checked_sub(1)?)?;
        self.catalogues.get(&self.agent)?.models.get(model)
    }
    pub(super) fn publish_catalogue(&mut self, catalogue: crate::providers::ModelCatalogue) {
        let focused_id = if self.picker == Some(Picker::Model) {
            self.picker_model(self.picker_selected)
                .map(|model| model.id.clone())
        } else {
            None
        };
        let focused_action = match self.picker {
            Some(Picker::Model) if self.picker_selected == 0 => Some("selection-default"),
            Some(Picker::Model) if self.picker_selected + 1 == self.picker_len() => {
                Some("models-refresh")
            }
            Some(Picker::Model) if self.picker_selected + 2 == self.picker_len() => {
                Some("model-custom")
            }
            _ => None,
        };
        self.catalogues.insert(self.agent, catalogue);
        if self.picker == Some(Picker::Model) {
            self.filter_picker();
            self.picker_selected = match focused_action {
                Some("selection-default") => 0,
                Some("model-custom") => self.picker_len() - 2,
                Some("models-refresh") => self.picker_len() - 1,
                _ => focused_id
                    .and_then(|id| {
                        self.picker_models
                            .iter()
                            .position(|index| self.catalogues[&self.agent].models[*index].id == id)
                            .map(|index| index + 1)
                    })
                    .unwrap_or(self.picker_len() - 2),
            };
        } else if self.picker == Some(Picker::Effort) {
            // A changed catalogue cannot silently repoint a focused effort row.
            self.picker_selected = self.picker_current().unwrap_or(0);
        }
    }

    /// Model and reasoning are one local transaction; browsing never writes preferences.
    pub(super) fn choose_model(&mut self, selection: ModelSelection) {
        self.model_picker_return = if self.picker == Some(Picker::Model) {
            Some(if self.picker_selected == 0 {
                ModelPickerFocus::Default
            } else if self.picker_selected + 2 == self.picker_len() {
                ModelPickerFocus::Custom
            } else {
                ModelPickerFocus::Model
            })
        } else {
            None
        };
        self.pending_model = Some(selection);
        if self
            .effort_model()
            .is_some_and(|model| !model.efforts.is_empty())
        {
            self.picker = Some(Picker::Effort);
            self.review_details = false;
            self.scroll = 0;
            self.picker_selected = self.picker_current().unwrap_or(0);
            self.notice = if self.can_back_to_models() {
                "Choose reasoning level. Enter accepts; Left returns to models; Esc keeps current settings."
            } else {
                "Choose reasoning level. Enter accepts; Esc keeps current settings."
            }.into();
        } else {
            let selection = self.pending_model.take().expect("candidate model");
            self.choose_selection(selection);
        }
    }

    pub(super) fn choose_selection(&mut self, selection: ModelSelection) {
        if selection == self.selection {
            self.pending_model = None;
            self.custom_model = None;
            self.model_picker_return = None;
            self.picker = None;
            self.notice = "Model and reasoning unchanged.".into();
        } else if selection.model != self.selection.model && self.thread_id.is_some() {
            self.selection_change = Some(selection);
            self.review_start();
        } else {
            self.apply_selection(selection, false);
        }
    }

    /// Exact current row; never mark Default when a retained selection is missing.
    pub fn picker_current(&self) -> Option<usize> {
        match self.picker? {
            Picker::Model => match self.selection.model.as_ref() {
                None => Some(0),
                Some(id) => self
                    .picker_models
                    .iter()
                    .position(|index| self.catalogues[&self.agent].models[*index].id == *id)
                    .map(|index| index + 1),
            },
            Picker::Effort
                if self
                    .pending_model
                    .as_ref()
                    .is_some_and(|selection| selection.model != self.selection.model) =>
            {
                None
            }
            Picker::Effort => match self.selection.effort.as_ref() {
                None => Some(0),
                Some(effort) => self
                    .effort_model()
                    .and_then(|model| model.efforts.iter().position(|value| value == effort))
                    .map(|index| index + 1),
            },
        }
    }

    pub fn picker_len(&self) -> usize {
        match self.picker {
            Some(Picker::Model) => 3 + self.picker_models.len().max(1),
            Some(Picker::Effort) => {
                1 + self
                    .effort_model()
                    .map(|model| model.efforts.len())
                    .unwrap_or(0)
                    .max(1)
            }
            None => 0,
        }
    }

    pub fn picker_choice(&self, index: usize) -> Option<super::super::commands::Choice> {
        use super::super::commands::Choice;
        if index >= self.picker_len() {
            return None;
        }
        if index == 0 {
            return Some(Choice::new("selection-default", "Default"));
        }
        Some(match self.picker? {
            Picker::Model if index + 1 == self.picker_len() => {
                Choice::new("models-refresh", "Refresh models")
            }
            Picker::Model if index + 2 == self.picker_len() => {
                Choice::new("model-custom", "Custom model ID…")
            }
            Picker::Model => {
                if let Some(catalogue) = self.catalogues.get(&self.agent) {
                    if let Some(model) = self.picker_model(index) {
                        let label = if model.name == model.id {
                            model.id.clone()
                        } else {
                            format!("{} · {}", model.name, model.id)
                        };
                        Choice::new("model-select", label)
                    } else {
                        Choice::disabled(
                            "model-select",
                            "No matching models",
                            if self.picker_query.text.is_empty() {
                                format!(
                                    "Provider advertised no selectable models. {}",
                                    catalogue.source
                                )
                            } else {
                                "Clear search with Ctrl+U, or enter an exact custom model ID."
                                    .into()
                            },
                        )
                    }
                } else {
                    Choice::disabled(
                        "model-select",
                        "No inspected models",
                        self.catalogue_errors.get(&self.agent).map_or_else(
                            || "Loading provider metadata only; no request text or tools are sent.".into(),
                            |reason| format!("{reason} Retry with /model, or choose Refresh to review the connection."),
                        ),
                    )
                }
            }
            Picker::Effort => {
                if let Some(effort) = self
                    .effort_model()
                    .and_then(|model| model.efforts.get(index - 1))
                {
                    Choice::new("effort-select", effort)
                } else {
                    Choice::disabled("effort-select", "Reasoning effort unavailable",
                        self.catalogues.get(&self.agent).and_then(|catalogue| catalogue.effort_note.clone())
                            .unwrap_or_else(|| "No supported reasoning effort values advertised for the selected/default model. Inspect models first.".into()))
                }
            }
        })
    }

    pub(super) fn apply_selection(&mut self, selection: ModelSelection, fresh: bool) {
        self.picker = None;
        self.selection_change = None;
        self.custom_model = None;
        self.pending_model = None;
        self.model_picker_return = None;
        if self.preview.is_some() {
            self.cancel();
        }
        self.selection = selection;
        self.preference_changed();
        if fresh {
            self.thread_id = None;
            self.items.clear();
            self.selected = 0;
            self.omitted = 0;
            self.observed_model = None;
        }
        self.review_start();
        self.notice = format!(
            "Model: {} · Reasoning: {} · no request sent.",
            self.selection.model.as_deref().unwrap_or("Default"),
            self.selection.effort.as_deref().unwrap_or("Default"),
        );
    }
}
