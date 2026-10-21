//! Explicit proposals derived only from the store's submitted-instruction records.
use crate::workspace::{
    memory::{self, Record, Scope},
    records::{self, Habit, InstructionReplacement},
};

#[derive(Default)]
pub struct State {
    pub selected: usize,
}

/// Recheck source uniqueness instead of trusting a displayed occurrence count.
pub fn eligible(habit: &Habit) -> bool {
    !habit.dismissed
        && memory::validate_text(&habit.instruction, memory::MAX_CONTENT_BYTES).is_ok()
        && records::instruction_key(&habit.instruction)
            .is_ok_and(|(key, normalized)| key == habit.key && normalized == habit.instruction)
        && habit
            .sources
            .iter()
            .enumerate()
            .filter(|(index, source)| {
                source.task_id > 0
                    && !source.request_id.is_empty()
                    && !habit.sources[..*index]
                        .iter()
                        .any(|prior| prior.request_id == source.request_id)
            })
            .take(3)
            .count()
            == 3
}

impl State {
    pub fn selected<'a>(&self, habits: &'a [Habit]) -> Option<&'a Habit> {
        habits
            .iter()
            .filter(|habit| eligible(habit) && habit.memory.is_none())
            .nth(self.selected)
    }

    pub fn select(&mut self, index: usize, habits: &[Habit]) {
        self.selected = index.min(
            habits
                .iter()
                .filter(|habit| eligible(habit) && habit.memory.is_none())
                .count()
                .saturating_sub(1),
        );
    }

    pub fn propose(
        &self,
        habits: &[Habit],
        memory: &mut super::memory::State,
    ) -> Result<(), String> {
        memory.propose_habit(
            self.selected(habits)
                .ok_or("No repeated instruction is available for approval.")?,
        )
    }

    pub fn dismiss(&self, habits: &[Habit]) -> Result<crate::store::records::Operation, String> {
        Ok(crate::store::records::Operation::DismissInstruction {
            key: self
                .selected(habits)
                .ok_or("No repeated instruction is available to dismiss.")?
                .key
                .clone(),
        })
    }
}

/// Freeze this once before stage_work. Only the explicitly selected, exact
/// approved revision can substitute a current instruction; history is untouched.
pub fn replacement(
    habits: &[Habit],
    selected_memories: &[Record],
    project: &str,
    current: &str,
) -> Option<InstructionReplacement> {
    let (key, normalized) = records::instruction_key(current).ok()?;
    let habit = habits
        .iter()
        .find(|habit| habit.key == key && eligible(habit))?;
    if habit.instruction != normalized {
        return None;
    }
    let binding = habit.memory.as_ref()?;
    let record = selected_memories.iter().find(|record| {
        record.id == binding.id
            && record.revision == binding.revision
            && record.project == project
            && record.proposal.scope == Scope::Project
            && record.proposal.content == habit.instruction
            && record.proposal.provenance.reference == format!("habit:{}", habit.key)
            && memory::validate_record(record).is_ok()
    })?;
    Some(InstructionReplacement {
        original: current.into(),
        replacement: InstructionReplacement::reference_text(record.id, record.revision),
        memory_id: record.id,
        memory_revision: record.revision,
        habit_key: key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        store::Attribution,
        workspace::{
            memory::{Proposal, Provenance, SourceKind},
            records::{InstructionSource, MemoryBinding},
        },
    };

    fn habit() -> Habit {
        let (key, instruction) = records::instruction_key("Use focused tests.").unwrap();
        Habit {
            key,
            instruction,
            sources: (1..=3)
                .map(|id| InstructionSource {
                    request_id: format!("request-{id}"),
                    task_id: id,
                    owner: Attribution {
                        partner: 1,
                        name: "Bolt".into(),
                        form: 0,
                    },
                    at: id,
                })
                .collect(),
            dismissed: false,
            memory: Some(MemoryBinding { id: 7, revision: 1 }),
        }
    }

    fn memory(habit: &Habit) -> Record {
        Record {
            id: 7,
            revision: 1,
            project: "/project".into(),
            task_id: None,
            proposal: Proposal {
                scope: Scope::Project,
                content: habit.instruction.clone(),
                provenance: Provenance {
                    kind: SourceKind::UserAuthored,
                    reference: format!("habit:{}", habit.key),
                    task_id: 1,
                },
            },
        }
    }

    #[test]
    fn exact_repeat_requires_three_distinct_sources_and_selected_bound_revision() {
        let mut habit = habit();
        let mut approved = memory(&habit);
        let frozen = replacement(
            &[habit.clone()],
            &[approved.clone()],
            "/project",
            " Use focused tests.\r\n",
        )
        .unwrap();
        assert_eq!(frozen.original, " Use focused tests.\r\n");
        assert!(!frozen.replacement.contains(&habit.instruction));
        assert!(replacement(&[habit.clone()], &[], "/project", &habit.instruction).is_none());
        approved.revision += 1;
        assert!(
            replacement(
                &[habit.clone()],
                &[approved],
                "/project",
                &habit.instruction
            )
            .is_none()
        );
        habit.sources[2] = habit.sources[0].clone();
        assert!(!eligible(&habit));
    }

    #[test]
    fn normalization_is_conservative_and_dismissed_or_secret_text_never_substitutes() {
        let mut habit = habit();
        let approved = memory(&habit);
        assert!(
            replacement(
                &[habit.clone()],
                std::slice::from_ref(&approved),
                "/other",
                &habit.instruction
            )
            .is_none()
        );
        assert!(
            replacement(
                &[habit.clone()],
                std::slice::from_ref(&approved),
                "/project",
                "Use  focused tests."
            )
            .is_none()
        );
        habit.dismissed = true;
        assert!(
            replacement(
                &[habit.clone()],
                &[approved],
                "/project",
                &habit.instruction
            )
            .is_none()
        );
        habit.dismissed = false;
        habit.instruction = "Use [REDACTED]".into();
        assert!(!eligible(&habit));
        assert_ne!(
            records::instruction_key("a\n b"),
            records::instruction_key("a\nb")
        );
    }
    #[test]
    fn literal_unicode_codepoints_do_not_inherit_a_different_instruction_memory() {
        let original = "Use the literal filename `cafe\u{301}.txt`.";
        let different = "Use the literal filename `café.txt`.";
        let mut habit = habit();
        (habit.key, habit.instruction) = records::instruction_key(original).unwrap();
        let approved = memory(&habit);
        assert!(
            replacement(
                std::slice::from_ref(&habit),
                std::slice::from_ref(&approved),
                "/project",
                different,
            )
            .is_none(),
            "distinct literal path bytes inherited another instruction's approved memory",
        );
        let frozen = replacement(&[habit], &[approved], "/project", original).unwrap();
        assert_eq!(frozen.original, original);
    }
}
