//! State container for an ACCURE site, mirroring "CRDT – part 1" in the
//! paper. The Automerge document persists the `Log` (a list of serialized
//! `Operation`s) and the document body (`Text`); `G` and validity tracking
//! are derived data recomputed by `integrate::rebuild`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dag::Dag;
use crate::dot::{Dot, SiteId};
use crate::op::{Effect, Operation, PolicyOp, Right};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strategy {
    /// Upper bound when allowing, lower bound when denying. Favors integrity
    /// of the document. n = n' denied at the boundary. This is the default.
    Integrity,
    /// Lower bound when allowing, upper bound when denying. Favors
    /// accessibility. n = n' allowed at the boundary.
    Accessibility,
}

impl Default for Strategy {
    fn default() -> Self { Strategy::Integrity }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AccessTuple {
    pub site: SiteId,
    pub right: Right,
}

impl AccessTuple {
    pub fn new(site: impl Into<SiteId>, right: Right) -> Self {
        Self { site: site.into(), right }
    }
}

/// In-memory replicated/derived state of a site. Each peer has one.
#[derive(Debug, Default)]
pub struct State {
    /// Local site identifier — embedded in every emitted `Dot`.
    pub me: SiteId,
    /// The append-only log of operations, ordered by integration time on
    /// this site. The paper's `local log`.
    pub log: Vec<Operation>,
    /// Per-access-tuple add-only monotonic DAG of policy operations.
    pub g: BTreeMap<AccessTuple, Dag>,
    /// Validity register: operations known to be valid right now.
    pub valid: BTreeMap<Dot, bool>,
    /// Undelivered outbox: operations whose target peer currently lacks the
    /// read right. Drained when the read right is granted.
    pub undelivered: BTreeMap<SiteId, Vec<Operation>>,
    /// Set of document `Dot`s whose effect is currently applied to the
    /// Automerge text. Used to decide whether `on_validity_change` should
    /// emit a local undo or redo compensation.
    pub applied: BTreeSet<Dot>,
    /// Per-(site, access tuple) next operation number, for `Autoincremented
    /// operation number per emitter`. The paper specifies the number is
    /// contiguous within the same access tuple.
    pub next_n: BTreeMap<(SiteId, TupleKey), u64>,
    /// Conflict resolution strategy.
    pub strategy: Strategy,
    /// Per-peer Automerge sync states (populated by the server task; tests
    /// using the library directly leave this empty).
    pub peer_sync_states: BTreeMap<SiteId, automerge::sync::State>,
}

/// Key for `next_n`: either an access tuple for policy ops, or the document
/// write tuple of the emitter for document ops.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TupleKey {
    Policy(AccessTuple),
    Document(SiteId),
    /// Used internally to allocate globally-unique per-site dot numbers.
    Site,
}

impl State {
    pub fn new(me: impl Into<SiteId>, strategy: Strategy) -> Self {
        Self { me: me.into(), strategy, ..Default::default() }
    }

    pub fn tuple_mut(&mut self, t: &AccessTuple) -> &mut Dag {
        if !self.g.contains_key(t) {
            self.g.insert(t.clone(), Dag::new());
        }
        self.g.get_mut(t).unwrap()
    }

    pub fn tuple(&self, t: &AccessTuple) -> Option<&Dag> {
        self.g.get(t)
    }

    pub fn op(&self, dot: &Dot) -> Option<&Operation> {
        self.log.iter().find(|o| o.dot() == dot)
    }

    pub fn policy_op(&self, dot: &Dot) -> Option<&PolicyOp> {
        self.op(dot).and_then(|o| o.as_policy())
    }

    /// Allocate next operation number for the local site. The number is
    /// globally unique within the site, regardless of access tuple — the
    /// paper allows per-tuple contiguity, but global uniqueness is simpler
    /// and unambiguous for `Dot`-keyed lookups.
    pub fn alloc_n(&mut self, _key: TupleKey) -> u64 {
        let entry = self.next_n.entry((self.me.clone(), TupleKey::Site)).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Update `next_n` based on a received op so we never re-use a number.
    pub fn note_n(&mut self, site: &SiteId, _key: TupleKey, n: u64) {
        let e = self.next_n.entry((site.clone(), TupleKey::Site)).or_insert(0);
        if n > *e { *e = n; }
    }

    /// Last known `Dot` for an access tuple on this site, considering the
    /// operations already integrated into `g[t]`. Returns the dot with the
    /// largest operation number (matches `last known Dot for access tuple`
    /// in the paper).
    pub fn last_known_dot_for_tuple(&self, t: &AccessTuple) -> Option<Dot> {
        self.tuple(t).and_then(|d| d.nodes().max_by_key(|x| x.n).cloned())
    }

    /// Apply default bootstrap policy: the local site has Admin, Read,
    /// Write on itself. This is recorded as an initial valid policy state
    /// without emitting operations (analogous to "initial policy" in CRDT
    /// part 1).
    pub fn bootstrap_self(&mut self) {
        for r in [Right::Admin, Right::Read, Right::Write] {
            let t = AccessTuple::new(self.me.clone(), r);
            self.tuple_mut(&t); // ensure entry exists
        }
        // Implicit allow effect — `validity::eval` treats an empty arc as
        // allow for the site against itself; see comment there.
        let _ = Effect::Allow;
    }
}
