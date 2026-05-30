//! ACCURE core library.
//!
//! Implements the data types and algorithms of the paper
//! "Access Control based on CRDTs for Collaborative Distributed
//! Applications" (Rault, Ignat, Perrin, 2023).

pub mod dot;
pub mod op;
pub mod dag;
pub mod state;
pub mod validity;
pub mod compensation;
pub mod integrate;
pub mod automerge_bridge;
pub mod wire;
pub mod messages;
pub mod model;

pub use dot::{Dot, SiteId};
pub use op::{DocOp, Effect, Operation, PolicyOp, Right, TextEdit, TextMutation};
pub use state::{AccessTuple, State, Strategy};
pub use model::{Document, Policy};

// Re-export note: `TextMutation` is a type alias for `TextEdit`, provided
// to align with the domain model naming. New code should prefer
// `TextMutation`; `TextEdit` is retained for backward compatibility.
