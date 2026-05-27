//! Integration of operations: `effect(...)` (CRDT part 2) and the
//! generators `update_policy` / `update_document` (CRDT part 1). The
//! authoritative replicated state lives in two Automerge `List`s — one
//! for `PolicyOp`s and one for `DocOp`s — both holding bincoded entries.
//! `rebuild_from_automerge` re-derives `State.log`, `State.g`, and
//! `State.valid` from these lists.

use std::collections::BTreeSet;

use automerge::{AutoCommit, ObjType, ReadDoc, ScalarValue, transaction::Transactable};

use crate::compensation;
use crate::dot::{Dot, SiteId};
use crate::op::{DocOp, Effect, Operation, PolicyOp, Right, TextEdit};
use crate::state::{AccessTuple, State, Strategy, TupleKey};
use crate::validity;

pub const POLICY_LOG_KEY: &str = "policy_log";
pub const DOC_LOG_KEY: &str = "doc_log";

#[derive(Debug, Clone)]
pub enum Trace {
    Generated(Operation),
    Received(Operation),
    Validity { dot: Dot, valid: bool },
    Compensation(String),
}

fn ensure_list(doc: &mut AutoCommit, key: &str) -> automerge::ObjId {
    if let Ok(Some((automerge::Value::Object(ObjType::List), id))) =
        doc.get(automerge::ROOT, key)
    {
        return id;
    }
    doc.put_object(automerge::ROOT, key, ObjType::List).expect("create list")
}

pub fn ensure_policy_log(doc: &mut AutoCommit) -> automerge::ObjId {
    ensure_list(doc, POLICY_LOG_KEY)
}

pub fn ensure_doc_log(doc: &mut AutoCommit) -> automerge::ObjId {
    ensure_list(doc, DOC_LOG_KEY)
}

fn append_bytes(doc: &mut AutoCommit, list: &automerge::ObjId, bytes: Vec<u8>) {
    let idx = doc.length(list);
    doc.insert(list, idx, ScalarValue::Bytes(bytes)).expect("append");
}

fn read_list_bytes(doc: &mut AutoCommit, list: &automerge::ObjId) -> Vec<Vec<u8>> {
    let len = doc.length(list);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        if let Ok(Some((value, _))) = doc.get(list, i) {
            if let automerge::Value::Scalar(s) = value {
                if let ScalarValue::Bytes(b) = s.as_ref() {
                    out.push(b.clone());
                }
            }
        }
    }
    out
}

pub fn read_policy_log(doc: &mut AutoCommit) -> Vec<PolicyOp> {
    let list = ensure_policy_log(doc);
    read_list_bytes(doc, &list)
        .into_iter()
        .filter_map(|b| bincode::deserialize::<PolicyOp>(&b).ok())
        .collect()
}

pub fn read_doc_log(doc: &mut AutoCommit) -> Vec<DocOp> {
    let list = ensure_doc_log(doc);
    read_list_bytes(doc, &list)
        .into_iter()
        .filter_map(|b| bincode::deserialize::<DocOp>(&b).ok())
        .collect()
}

/// Locally generate and integrate a policy operation.
pub fn update_policy(
    state: &mut State,
    doc: &mut AutoCommit,
    target: SiteId,
    right: Right,
    effect: Effect,
) -> Result<PolicyOp, &'static str> {
    let me = state.me.clone();
    if !validity::eval(state, &me, Right::Admin) {
        return Err("local site lacks Admin right");
    }
    let t = AccessTuple::new(target.clone(), right);
    let n = state.alloc_n(TupleKey::Policy(t.clone()));
    let dot = Dot::new(me.clone(), n);

    let last_dot_seen = last_dot_for_write_tuple(state, &target);
    let missing_dots = derive_missing_dots(state, &target, &last_dot_seen);
    let deps = current_top_level_deps(state, &t);
    let p = PolicyOp { dot, target, right, effect, deps, last_dot_seen, missing_dots };

    let list = ensure_policy_log(doc);
    let bytes = bincode::serialize(&p).expect("encode PolicyOp");
    append_bytes(doc, &list, bytes);
    effect_policy(state, &p);
    Ok(p)
}

/// Locally generate and integrate a document operation.
pub fn update_document(
    state: &mut State,
    doc: &mut AutoCommit,
    edit: TextEdit,
) -> Result<DocOp, &'static str> {
    let me = state.me.clone();
    if !validity::eval(state, &me, Right::Write) {
        return Err("local site lacks Write right");
    }
    let n = state.alloc_n(TupleKey::Document(me.clone()));
    let dot = Dot::new(me, n);
    let t = AccessTuple::new(state.me.clone(), Right::Write);
    let deps = current_top_level_deps(state, &t);
    let d = DocOp { dot, deps, edit };

    let list = ensure_doc_log(doc);
    let bytes = bincode::serialize(&d).expect("encode DocOp");
    append_bytes(doc, &list, bytes);
    effect_document(state, &d);
    Ok(d)
}

fn last_dot_for_write_tuple(state: &State, target: &SiteId) -> Option<Dot> {
    state
        .log
        .iter()
        .filter_map(|o| o.as_document())
        .filter(|d| &d.dot.site == target)
        .max_by_key(|d| d.dot.n)
        .map(|d| d.dot.clone())
}

fn derive_missing_dots(state: &State, target: &SiteId, last: &Option<Dot>) -> Vec<Dot> {
    let last_n = match last { Some(d) => d.n, None => return vec![] };
    let mut seen: BTreeSet<u64> = state
        .log
        .iter()
        .filter_map(|o| o.as_document())
        .filter(|d| &d.dot.site == target)
        .map(|d| d.dot.n)
        .collect();
    seen.insert(last_n);
    (1..last_n).filter(|n| !seen.contains(n)).map(|n| Dot::new(target.clone(), n)).collect()
}

fn current_top_level_deps(state: &State, t: &AccessTuple) -> Vec<Dot> {
    let dag = match state.tuple(t) { Some(d) => d, None => return vec![] };
    let top = dag.max_level();
    if top == 0 { return vec![]; }
    dag.at_level(top)
        .into_iter()
        .filter(|d| state.valid.get(d).copied().unwrap_or(false))
        .collect()
}

pub fn effect_policy(state: &mut State, p: &PolicyOp) {
    let t = AccessTuple::new(p.target.clone(), p.right);
    {
        let dag = state.tuple_mut(&t);
        dag.add_node(p.dot.clone());
        for dep in &p.deps {
            dag.add_edge(dep.clone(), p.dot.clone());
        }
    }
    state.note_n(&p.dot.site, TupleKey::Policy(t), p.dot.n);
    state.log.push(Operation::Policy(p.clone()));
    rebuild_validity(state);
}

pub fn effect_document(state: &mut State, d: &DocOp) {
    state.log.push(Operation::Document(d.clone()));
    state.note_n(&d.dot.site, TupleKey::Document(d.dot.site.clone()), d.dot.n);
    let valid = validity::is_valid_document(state, d);
    state.valid.insert(d.dot.clone(), valid);
}

/// Fixed-point validity rebuild over the entire log. Bounded to a safe
/// number of iterations to prevent oscillation under pathological inputs.
pub fn rebuild_validity(state: &mut State) -> Vec<Trace> {
    let mut traces = Vec::new();
    let max_iters = state.log.len().saturating_mul(4) + 8;
    for _ in 0..max_iters {
        let mut changed = false;
        let ops: Vec<Operation> = state.log.clone();
        for op in &ops {
            let new_v = validity::is_valid(state, op);
            if state.valid.get(op.dot()).copied() != Some(new_v) {
                state.valid.insert(op.dot().clone(), new_v);
                traces.push(Trace::Validity { dot: op.dot().clone(), valid: new_v });
                if let Operation::Document(_) = op {
                    traces.push(Trace::Compensation(format!(
                        "{} → {}",
                        op.dot(),
                        if new_v { "redo" } else { "undo" }
                    )));
                }
                changed = true;
            }
        }
        if !changed { break; }
    }
    traces
}

/// Re-derive `State` from the Automerge document. Brings in any policy /
/// doc ops not yet in `state.log`.
pub fn rebuild_from_automerge(state: &mut State, doc: &mut AutoCommit) -> Vec<Trace> {
    let mut traces = Vec::new();
    let prior_policy: BTreeSet<Dot> =
        state.log.iter().filter_map(|o| o.as_policy()).map(|p| p.dot.clone()).collect();
    let prior_docs: BTreeSet<Dot> =
        state.log.iter().filter_map(|o| o.as_document()).map(|d| d.dot.clone()).collect();

    for p in read_policy_log(doc) {
        if prior_policy.contains(&p.dot) { continue; }
        traces.push(Trace::Received(Operation::Policy(p.clone())));
        effect_policy(state, &p);
    }
    for d in read_doc_log(doc) {
        if prior_docs.contains(&d.dot) { continue; }
        traces.push(Trace::Received(Operation::Document(d.clone())));
        effect_document(state, &d);
    }
    traces.extend(rebuild_validity(state));
    traces
}

/// Build a fresh, shareable Automerge document with the two top-level
/// lists. Use `AutoCommit::save()` on the returned doc and `AutoCommit::
/// load()` on every peer to share the same root objects so concurrent
/// appends survive `sync`.
pub fn new_shared_doc() -> AutoCommit {
    let mut doc = AutoCommit::new();
    let _ = ensure_policy_log(&mut doc);
    let _ = ensure_doc_log(&mut doc);
    doc
}

/// Bootstrap helper that creates a fresh, independent doc (only correct
/// for a single-site test). For multi-site convergence tests, load the
/// same `new_shared_doc()` bytes on every peer instead.
pub fn new_state_with_doc(me: SiteId, strategy: Strategy) -> (State, AutoCommit) {
    let mut state = State::new(me, strategy);
    state.bootstrap_self();
    let doc = new_shared_doc();
    (state, doc)
}

/// Bootstrap helper for multi-site tests / real servers: load a shared
/// initial doc on every peer.
pub fn new_state_from_bytes(
    me: SiteId,
    strategy: Strategy,
    bytes: &[u8],
) -> Result<(State, AutoCommit), automerge::AutomergeError> {
    let mut state = State::new(me, strategy);
    state.bootstrap_self();
    let doc = AutoCommit::load(bytes)?;
    Ok((state, doc))
}

/// Compute the current rendered document text.
pub fn current_text(state: &State) -> String {
    compensation::render_text(&state.log, &state.valid)
}
