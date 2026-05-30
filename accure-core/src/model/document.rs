//! Document model — text mutations and their integration.

use automerge::AutoCommit;

use crate::integrate;
use crate::op::{DocOp, TextEdit};
use crate::state::State;

/// Type alias surfacing the document operation type under the model namespace.
pub type Op = DocOp;

/// Type alias aligning `TextEdit` to the domain model naming.
pub type TextMutation = TextEdit;

/// Document model entry point.
///
/// Groups document-related generation, effect, and query functions.
pub struct Document;

impl Document {
    /// Locally generate and integrate a document operation.
    ///
    /// Equivalent to [`integrate::update_document`].
    pub fn update(
        state: &mut State,
        doc: &mut AutoCommit,
        edit: TextEdit,
    ) -> Result<DocOp, &'static str> {
        integrate::update_document(state, doc, edit)
    }

    /// Apply a document operation's effect to state (CRDT part 2).
    ///
    /// Equivalent to [`integrate::effect_document`].
    pub fn effect(state: &mut State, d: &DocOp) {
        integrate::effect_document(state, d)
    }

    /// Compute the current rendered document text (compensation).
    ///
    /// Equivalent to [`integrate::current_text`].
    pub fn compensation(state: &State) -> String {
        integrate::current_text(state)
    }
}
