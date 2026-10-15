use super::{ModelSelection, Picker, State};

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

    pub(super) fn effort_model(&self) -> Option<&crate::providers::ModelOption> {
        let catalogue = self.catalogues.get(&self.agent)?;
        let id = self
            .selection
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
            self.picker_models.extend(
                catalogue
                    .models
                    .iter()
                    .enumerate()
                    .filter(|(_, model)| matches(&model.name) || matches(&model.id))
                    .map(|(index, _)| index),
            );
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

    pub(super) fn choose_selection(&mut self, selection: ModelSelection) {
        if selection == self.selection {
            self.picker = None;
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
            return Some(Choice::new(
                "selection-default",
                "Default (provider configuration)",
            ));
        }
        Some(match self.picker? {
            Picker::Model if index + 1 == self.picker_len() => Choice::new(
                "models-refresh",
                "Refresh / inspect available models (review connection first)",
            ),
            Picker::Model if index + 2 == self.picker_len() => {
                Choice::new("model-custom", "Enter custom model ID (local only)")
            }
            Picker::Model => {
                if let Some(catalogue) = self.catalogues.get(&self.agent) {
                    if let Some(model) = self.picker_model(index) {
                        Choice::new("model-select", format!("{} [{}]", model.name, model.id))
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
        let model_changed = selection.model != self.selection.model;
        self.picker = None;
        self.selection_change = None;
        self.custom_model = None;
        if self.preview.is_some() {
            self.cancel();
        }
        self.selection = selection;
        self.selections.insert(self.agent, self.selection.clone());
        if fresh {
            self.thread_id = None;
            self.items.clear();
            self.selected = 0;
            self.omitted = 0;
            self.observed_model = None;
        }
        self.review_start();
        self.notice = "Selection updated locally. Previous payload consent invalidated; /new or compatible /resume requires fresh review. No connection was authorized.".into();
        // Continue straight to effort when the new model advertises choices.
        if model_changed
            && self.selection.model.is_some()
            && self
                .effort_model()
                .is_some_and(|model| !model.efforts.is_empty())
        {
            self.open_picker(Picker::Effort);
            self.notice = "Model updated locally. Choose reasoning effort, or Esc to keep the provider default.".into();
        }
    }
}
