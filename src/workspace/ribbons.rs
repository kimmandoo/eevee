//! Optional local first-recorded history; never provider context or authority.
use crate::store::Attribution;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    AcceptedReview,
    VerifiedHandoff,
    LearnedMove,
}
impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AcceptedReview => "accepted_review",
            Self::VerifiedHandoff => "verified_handoff",
            Self::LearnedMove => "learned_move",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "accepted_review" => Some(Self::AcceptedReview),
            "verified_handoff" => Some(Self::VerifiedHandoff),
            "learned_move" => Some(Self::LearnedMove),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Evidence {
    Artifact { id: String },
    Move { id: i64, revision: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ribbon {
    pub kind: Kind,
    pub task_id: i64,
    pub owner: Attribution,
    pub at: i64,
    pub evidence: Evidence,
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub ribbon: Ribbon,
    pub freshness: super::records::Freshness,
    pub explanation: String,
}
