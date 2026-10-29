//! Frozen ownership for native read-only children; never a permission grant.
use std::path::PathBuf;

pub const MAX_CHILDREN: usize = 3;
pub const MAX_ROUNDS: usize = 64;
pub const MAX_RESULT_BYTES: usize = 16 * 1024;
pub const MAX_SECONDS: u64 = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub call_id: String,
    pub role: String,
    pub instruction: String,
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub parent_request_id: String,
    pub child_request_id: String,
    pub role: String,
    pub instruction: String,
    pub call_id: String,
    pub paths: Vec<PathBuf>,
}

impl Call {
    pub fn validate(&self) -> Result<(), String> {
        validate(&self.call_id, &self.role, &self.instruction, &self.paths)
    }
}

impl Context {
    pub fn validate(&self) -> Result<(), String> {
        validate(&self.call_id, &self.role, &self.instruction, &self.paths)
    }
}

fn validate(call_id: &str, role: &str, instruction: &str, paths: &[PathBuf]) -> Result<(), String> {
    if call_id.is_empty()
        || call_id.len() > 256
        || call_id.chars().any(char::is_control)
        || !matches!(role, "explore" | "review")
        || instruction.trim().is_empty()
        || instruction.len() > 4096
        || instruction
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
        || crate::workspace::privacy::redact(instruction) != instruction
        || paths.is_empty()
        || paths.len() > 32
        || paths.iter().any(|path| {
            path.is_absolute()
                || path
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || path.as_os_str().len() > 4096
                || path.to_str().is_none_or(|path| {
                    path.chars()
                        .any(|c| c.is_control() || matches!(c, '\\' | ':'))
                })
        })
    {
        return Err(
            "Invalid read-only delegation instruction, role or project-relative paths".into(),
        );
    }
    Ok(())
}

pub fn bounded_result(mut text: String) -> String {
    if text.len() > MAX_RESULT_BYTES {
        const SUFFIX: &str = "\n[Delegated summary truncated at 16 KiB]";
        let mut end = MAX_RESULT_BYTES - SUFFIX.len();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(SUFFIX);
    }
    text
}
