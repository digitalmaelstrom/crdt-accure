use serde::{Deserialize, Serialize};

use crate::dot::Dot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Right {
    Admin,
    Write,
    Read,
}

impl Right {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "a" | "admin" => Some(Right::Admin),
            "w" | "write" => Some(Right::Write),
            "r" | "read" => Some(Right::Read),
            _ => None,
        }
    }
    pub fn short(&self) -> &'static str {
        match self {
            Right::Admin => "A",
            Right::Write => "W",
            Right::Read => "R",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Allow,
    Deny,
}

impl Effect {
    pub fn opposite(self) -> Self {
        match self {
            Effect::Allow => Effect::Deny,
            Effect::Deny => Effect::Allow,
        }
    }
}

/// Text edits applied against the Automerge `Text` object holding the
/// document. Document operations in the paper are commutative and have no
/// causal dependencies between each other; Automerge `Text` provides this
/// guarantee for us, and we only carry the high-level intent here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextEdit {
    Insert { pos: usize, ch: char },
    Delete { pos: usize },
}

/// Type alias aligning `TextEdit` to the domain model naming convention.
/// Prefer using `model::document::TextMutation` for new code.
pub type TextMutation = TextEdit;

/// Policy operation, mirroring `BasicOperation` of `Type ..= policy`
/// extended with `LastDotSeen` / `MissingDots` (Basic Type 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyOp {
    pub dot: Dot,
    pub target: crate::dot::SiteId,
    pub right: Right,
    pub effect: Effect,
    pub deps: Vec<Dot>,
    pub last_dot_seen: Option<Dot>,
    pub missing_dots: Vec<Dot>,
}

/// Document operation, mirroring `BasicOperation` of `Type ..= document`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocOp {
    pub dot: Dot,
    pub deps: Vec<Dot>,
    pub edit: TextEdit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    Policy(PolicyOp),
    Document(DocOp),
}

impl Operation {
    pub fn dot(&self) -> &Dot {
        match self {
            Operation::Policy(p) => &p.dot,
            Operation::Document(d) => &d.dot,
        }
    }
    pub fn deps(&self) -> &[Dot] {
        match self {
            Operation::Policy(p) => &p.deps,
            Operation::Document(d) => &d.deps,
        }
    }
    pub fn is_policy(&self) -> bool {
        matches!(self, Operation::Policy(_))
    }
    pub fn as_policy(&self) -> Option<&PolicyOp> {
        if let Operation::Policy(p) = self { Some(p) } else { None }
    }
    pub fn as_document(&self) -> Option<&DocOp> {
        if let Operation::Document(d) = self { Some(d) } else { None }
    }
}
