//! Validity evaluation for ACCURE operations.
//!
//! Implements Algorithms 1 and 2 of the paper. Document-operation validity
//! is the trickiest part; the paper's authoritative interval merging
//! depends on `LastDotSeen` / `MissingDots`. Our implementation reduces
//! the rule to:
//!
//!   * A policy op is valid if every conflict it loses against a concurrent
//!     valid policy op of opposite effect is resolved in its favor under
//!     the configured `Strategy`, **and** all its `Deps` are valid.
//!   * A document op `d = e:k` in tuple `T = (e, Write)` is valid iff
//!     `deps` are valid **and** the deepest-level valid policy op in
//!     `G[T]` that "covers" `d` has effect `Allow`. A policy op `p` covers
//!     `d` when:
//!       - `d` is listed in `p.missing_dots` (explicit exclusion → covers
//!         even though the dot wasn't yet seen), OR
//!       - `p.last_dot_seen.n >= k` (the policy op was generated with
//!         knowledge of `d`), OR
//!       - `p.last_dot_seen.n <  k` and `p.effect == Deny` (the deny
//!         extends to ops concurrent with or after it).
//!
//! Tie breaks at the same level use the `Strategy`:
//!   * `Integrity`     → `Deny` wins; n = n' boundary is denied.
//!   * `Accessibility` → `Allow` wins; n = n' boundary is allowed.

use crate::dag::Dag;
use crate::dot::{Dot, SiteId};
use crate::op::{DocOp, Effect, Operation, PolicyOp, Right};
use crate::state::{AccessTuple, State, Strategy};

/// Result of `eval(site, right)` per CRDT – part 3, lines 46–48.
/// Bootstrap: when no policy operation exists for the access tuple
/// `(site, right)`, the site has the right on itself — this is universal
/// initial policy known to all peers.
pub fn eval(state: &State, site: &SiteId, right: Right) -> bool {
    let t = AccessTuple::new(site.clone(), right);
    let dag = match state.tuple(&t) {
        Some(d) if d.nodes().next().is_some() => d,
        _ => return true, // bootstrap: self-A/R/W
    };
    // Last operation of the longest arc within G[t] such that isValid(o).
    let mut best: Option<(usize, &PolicyOp)> = None;
    for n in dag.nodes() {
        if state.valid.get(n).copied().unwrap_or(false) {
            if let Some(p) = state.policy_op(n) {
                let lvl = dag.level(n);
                match best {
                    Some((bl, _)) if bl > lvl => {}
                    Some((bl, _bp)) if bl == lvl => {
                        let p_wins = match state.strategy {
                            Strategy::Integrity => p.effect == Effect::Deny,
                            Strategy::Accessibility => p.effect == Effect::Allow,
                        };
                        if p_wins { best = Some((lvl, p)); }
                    }
                    _ => best = Some((lvl, p)),
                }
            }
        }
    }
    match best {
        Some((_, p)) => p.effect == Effect::Allow,
        None => true, // all candidate ops invalid → bootstrap allows
    }
}

/// Bootstrap helper retained for backwards-compat; now identical to `eval`.
pub fn eval_with_self(state: &State, site: &SiteId, right: Right) -> bool {
    eval(state, site, right)
}

/// Algorithm 1: validity of a policy operation.
pub fn is_valid_policy(state: &State, op: &PolicyOp, dag: &Dag) -> bool {
    // Concurrent ancestors of `op` that are of opposite effect form the
    // conflict set. Two ops are "concurrent" iff neither is in the
    // other's ancestor set within the DAG.
    let anc = dag.ancestors(&op.dot);
    let mut conflicts: Vec<&PolicyOp> = Vec::new();
    for n in dag.nodes() {
        if n == &op.dot { continue; }
        if anc.contains(n) { continue; }
        let n_anc = dag.ancestors(n);
        if n_anc.contains(&op.dot) { continue; }
        // n is concurrent with op.
        if let Some(p) = state.policy_op(n) {
            if p.effect != op.effect {
                conflicts.push(p);
            }
        }
    }
    if !conflicts.is_empty() {
        // op is valid w.r.t. conflicts iff it wins every one currently valid.
        for c in conflicts {
            if state.valid.get(&c.dot).copied().unwrap_or(false)
                && !op_wins_against(state.strategy, op, c)
            {
                return false;
            }
        }
    }
    // Dependencies must themselves be valid.
    op.deps.iter().all(|d| state.valid.get(d).copied().unwrap_or(false))
        || op.deps.is_empty()
}

/// Tie break between two concurrent policy ops of opposite effect.
fn op_wins_against(strategy: Strategy, a: &PolicyOp, _b: &PolicyOp) -> bool {
    match strategy {
        Strategy::Integrity => a.effect == Effect::Deny,
        Strategy::Accessibility => a.effect == Effect::Allow,
    }
}

/// Algorithm 2 — validity of a document operation.
pub fn is_valid_document(state: &State, d: &DocOp) -> bool {
    let emitter = d.dot.site.clone();
    let t = AccessTuple::new(emitter.clone(), Right::Write);
    let dag = match state.tuple(&t) {
        Some(g) => g,
        None => {
            // No policy ops for this tuple — bootstrap self-rights apply.
            let bootstrap_ok = emitter == state.me || true; // every site self-admin
            return bootstrap_ok
                && d.deps.iter().all(|x| state.valid.get(x).copied().unwrap_or(true));
        }
    };

    // Find the deepest-level valid policy op covering `d`. Strategy resolves
    // ties at the same level.
    let mut best: Option<(usize, &PolicyOp)> = None;
    for n in dag.nodes() {
        if !state.valid.get(n).copied().unwrap_or(false) { continue; }
        let p = match state.policy_op(n) { Some(p) => p, None => continue };
        if !covers(p, &d.dot) { continue; }
        let lvl = dag.level(n);
        match best {
            None => best = Some((lvl, p)),
            Some((bl, _)) if lvl > bl => best = Some((lvl, p)),
            Some((bl, _bp)) if lvl == bl => {
                let p_wins = match state.strategy {
                    Strategy::Integrity => p.effect == Effect::Deny,
                    Strategy::Accessibility => p.effect == Effect::Allow,
                };
                if p_wins { best = Some((lvl, p)); }
            }
            _ => {}
        }
    }
    let policy_allows = match best {
        Some((_, p)) => p.effect == Effect::Allow,
        None => true, // no covering policy → bootstrap self-write applies
    };
    if !policy_allows { return false; }
    d.deps.iter().all(|x| state.valid.get(x).copied().unwrap_or(true))
}

/// Does policy op `p` cover document op with dot `d`?
fn covers(p: &PolicyOp, d: &Dot) -> bool {
    if p.missing_dots.iter().any(|m| m == d) { return true; }
    match &p.last_dot_seen {
        Some(lds) if lds.site == d.site => {
            if lds.n >= d.n { return true; }
            // Concurrent — denies cover, allows do not (so a later allow
            // doesn't retroactively re-authorize concurrent doc ops).
            p.effect == Effect::Deny
        }
        _ => p.effect == Effect::Deny, // unknown last_dot_seen — deny still covers
    }
}

/// Top-level validity for any operation.
pub fn is_valid(state: &State, op: &Operation) -> bool {
    match op {
        Operation::Policy(p) => {
            let t = AccessTuple::new(p.target.clone(), p.right);
            let dag = match state.tuple(&t) { Some(d) => d, None => return p.deps.is_empty() };
            is_valid_policy(state, p, dag)
        }
        Operation::Document(d) => is_valid_document(state, d),
    }
}
