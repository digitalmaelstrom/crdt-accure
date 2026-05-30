//! Policy model — access-control operations and their integration.

use automerge::AutoCommit;

use crate::dot::SiteId;
use crate::integrate;
use crate::op::{Effect, PolicyOp, Right};
use crate::state::State;

/// Type alias surfacing the policy operation type under the model namespace.
pub type Op = PolicyOp;

/// Policy model entry point.
///
/// Groups policy-related generation, effect, and query functions.
pub struct Policy;

impl Policy {
    /// Locally generate and integrate a policy operation.
    ///
    /// Equivalent to [`integrate::update_policy`].
    pub fn update(
        state: &mut State,
        doc: &mut AutoCommit,
        target: SiteId,
        right: Right,
        effect: Effect,
    ) -> Result<PolicyOp, &'static str> {
        integrate::update_policy(state, doc, target, right, effect)
    }

    /// Apply a policy operation's effect to state (CRDT part 2).
    ///
    /// Equivalent to [`integrate::effect_policy`].
    pub fn effect(state: &mut State, p: &PolicyOp) {
        integrate::effect_policy(state, p)
    }
}
