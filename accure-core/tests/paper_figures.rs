//! Tests modeled on the paper's figures, adapted to our universal-self
//! bootstrap (every site has A/R/W on itself).

use accure_core::integrate::{
    current_text, effect_document, effect_policy, new_shared_doc, new_state_from_bytes,
    rebuild_from_automerge, update_document, update_policy,
};
use accure_core::dot::Dot;
use accure_core::op::{DocOp, Effect, PolicyOp, Right, TextEdit};
use accure_core::state::{State, Strategy};
use accure_core::validity::eval;
use automerge::sync::SyncDoc;

fn peers(ids: &[&str]) -> Vec<(accure_core::state::State, automerge::AutoCommit)> {
    let mut shared = new_shared_doc();
    let bytes = shared.save();
    ids.iter()
        .map(|id| new_state_from_bytes((*id).into(), Strategy::Integrity, &bytes).unwrap())
        .collect()
}

fn sync_pair(a: &mut automerge::AutoCommit, b: &mut automerge::AutoCommit) {
    let mut sa = automerge::sync::State::new();
    let mut sb = automerge::sync::State::new();
    for _ in 0..8 {
        if let Some(m) = a.sync().generate_sync_message(&mut sa) {
            b.sync().receive_sync_message(&mut sb, m).unwrap();
        }
        if let Some(m) = b.sync().generate_sync_message(&mut sb) {
            a.sync().receive_sync_message(&mut sa, m).unwrap();
        }
    }
}

#[test]
fn fig2_compensation_after_concurrent_deny() {
    let mut sites = peers(&["S1", "S2"]);
    let (mut s2, mut d2) = sites.pop().unwrap();
    let (mut s1, mut d1) = sites.pop().unwrap();

    assert!(eval(&s1, &"S1".into(), Right::Write));
    assert!(eval(&s2, &"S2".into(), Right::Admin));

    update_document(&mut s1, &mut d1, TextEdit::Insert { pos: 0, ch: 'a' }).unwrap();
    assert_eq!(current_text(&s1), "a");

    update_policy(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s2, &"S1".into(), Right::Write));

    sync_pair(&mut d1, &mut d2);
    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);

    assert!(!eval(&s1, &"S1".into(), Right::Write));
    assert!(!eval(&s2, &"S1".into(), Right::Write));
    let dot = Dot::new("S1", 1);
    assert_eq!(s1.valid.get(&dot), Some(&false));
    assert_eq!(s2.valid.get(&dot), Some(&false));
    assert_eq!(current_text(&s1), "");
    assert_eq!(current_text(&s2), "");
}

#[test]
fn fig1_policy_convergence_integrity() {
    let mut sites = peers(&["S1", "S2", "S3"]);
    let (mut s3, mut d3) = sites.pop().unwrap();
    let (mut s2, mut d2) = sites.pop().unwrap();
    let (mut s1, mut d1) = sites.pop().unwrap();

    update_policy(&mut s1, &mut d1, "S2".into(), Right::Admin, Effect::Deny).unwrap();
    update_policy(&mut s2, &mut d2, "S1".into(), Right::Admin, Effect::Deny).unwrap();

    for _ in 0..3 {
        sync_pair(&mut d1, &mut d2);
        sync_pair(&mut d2, &mut d3);
        sync_pair(&mut d1, &mut d3);
    }
    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    rebuild_from_automerge(&mut s3, &mut d3);

    let s2_admin = [
        eval(&s1, &"S2".into(), Right::Admin),
        eval(&s2, &"S2".into(), Right::Admin),
        eval(&s3, &"S2".into(), Right::Admin),
    ];
    let s1_admin = [
        eval(&s1, &"S1".into(), Right::Admin),
        eval(&s2, &"S1".into(), Right::Admin),
        eval(&s3, &"S1".into(), Right::Admin),
    ];
    assert!(s2_admin.iter().all(|v| *v == s2_admin[0]),
        "sites disagree on S2 admin: {:?}", s2_admin);
    assert!(s1_admin.iter().all(|v| *v == s1_admin[0]),
        "sites disagree on S1 admin: {:?}", s1_admin);
}

#[test]
fn invalid_deps_propagate() {
    let mut sites = peers(&["S1"]);
    let (mut s, mut d) = sites.pop().unwrap();
    update_policy(&mut s, &mut d, "S1".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s, &"S1".into(), Right::Write));
    assert!(update_document(&mut s, &mut d, TextEdit::Insert { pos: 0, ch: 'x' }).is_err());
}

#[test]
fn toggle_deny_then_allow() {
    let mut sites = peers(&["A"]);
    let (mut s, mut d) = sites.pop().unwrap();
    update_policy(&mut s, &mut d, "B".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s, &"B".into(), Right::Write));
    update_policy(&mut s, &mut d, "B".into(), Right::Write, Effect::Allow).unwrap();
    assert!(eval(&s, &"B".into(), Right::Write));
}

#[test]
fn missing_dots_allow_can_cover_gap() {
    let mut state = State::new("S0", Strategy::Integrity);
    state.bootstrap_self();

    let deny = PolicyOp {
        dot: Dot::new("S0", 1),
        target: "S1".into(),
        right: Right::Write,
        effect: Effect::Deny,
        deps: vec![],
        last_dot_seen: None,
        missing_dots: vec![],
    };
    effect_policy(&mut state, &deny);

    let allow = PolicyOp {
        dot: Dot::new("S0", 2),
        target: "S1".into(),
        right: Right::Write,
        effect: Effect::Allow,
        deps: vec![deny.dot.clone()],
        last_dot_seen: Some(Dot::new("S1", 3)),
        missing_dots: vec![Dot::new("S1", 2)],
    };
    effect_policy(&mut state, &allow);

    let covered_gap = DocOp { dot: Dot::new("S1", 2), deps: vec![], edit: TextEdit::Insert { pos: 0, ch: 'a' } };
    let seen_before_allow = DocOp {
        dot: Dot::new("S1", 3),
        deps: vec![],
        edit: TextEdit::Insert { pos: 1, ch: 'b' },
    };
    let concurrent_after_allow = DocOp {
        dot: Dot::new("S1", 4),
        deps: vec![],
        edit: TextEdit::Insert { pos: 2, ch: 'c' },
    };
    effect_document(&mut state, &covered_gap);
    effect_document(&mut state, &seen_before_allow);
    effect_document(&mut state, &concurrent_after_allow);

    assert_eq!(state.valid.get(&covered_gap.dot), Some(&true));
    assert_eq!(state.valid.get(&seen_before_allow.dot), Some(&true));
    assert_eq!(state.valid.get(&concurrent_after_allow.dot), Some(&false));
}

#[test]
fn concurrent_batch_triggers_bulk_undo_redo() {
    let mut sites = peers(&["S1", "S2"]);
    let (mut s2, mut d2) = sites.pop().unwrap();
    let (mut s1, mut d1) = sites.pop().unwrap();

    let mut doc_dots = Vec::new();
    for ch in ['a', 'b', 'c', 'd', 'e', 'f'] {
        let op = update_document(&mut s1, &mut d1, TextEdit::Insert { pos: 99, ch }).unwrap();
        doc_dots.push(op.dot);
    }
    assert_eq!(current_text(&s1), "abcdef");

    // 7 concurrent operations total: 6 document writes on S1, 1 deny on S2.
    update_policy(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Deny).unwrap();
    sync_pair(&mut d1, &mut d2);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    assert_eq!(current_text(&s1), "");
    assert_eq!(current_text(&s2), "");
    for dot in &doc_dots {
        assert_eq!(s1.valid.get(dot), Some(&false));
        assert_eq!(s2.valid.get(dot), Some(&false));
    }

    update_policy(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Allow).unwrap();
    sync_pair(&mut d1, &mut d2);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    assert_eq!(current_text(&s1), "abcdef");
    assert_eq!(current_text(&s2), "abcdef");
    for dot in &doc_dots {
        assert_eq!(s1.valid.get(dot), Some(&true));
        assert_eq!(s2.valid.get(dot), Some(&true));
    }
}
