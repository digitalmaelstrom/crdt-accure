//! Tests modeled on the paper's figures, adapted to our universal-self
//! bootstrap (every site has A/R/W on itself).

use accure_core::integrate::{
    new_shared_doc, new_state_from_bytes, rebuild_from_automerge,
};
use accure_core::model::{Document, Policy};
use accure_core::model::document::TextMutation;
use accure_core::dot::Dot;
use accure_core::op::{DocOp, Effect, PolicyOp, Right};
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

    Document::update(&mut s1, &mut d1, TextMutation::Insert { pos: 0, ch: 'a' }).unwrap();
    assert_eq!(Document::compensation(&s1), "a");

    Policy::update(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s2, &"S1".into(), Right::Write));

    sync_pair(&mut d1, &mut d2);
    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);

    assert!(!eval(&s1, &"S1".into(), Right::Write));
    assert!(!eval(&s2, &"S1".into(), Right::Write));
    let dot = Dot::new("S1", 1);
    assert_eq!(s1.valid.get(&dot), Some(&false));
    assert_eq!(s2.valid.get(&dot), Some(&false));
    assert_eq!(Document::compensation(&s1), "");
    assert_eq!(Document::compensation(&s2), "");
}

#[test]
fn fig1_policy_convergence_integrity() {
    let mut sites = peers(&["S1", "S2", "S3"]);
    let (mut s3, mut d3) = sites.pop().unwrap();
    let (mut s2, mut d2) = sites.pop().unwrap();
    let (mut s1, mut d1) = sites.pop().unwrap();

    Policy::update(&mut s1, &mut d1, "S2".into(), Right::Admin, Effect::Deny).unwrap();
    Policy::update(&mut s2, &mut d2, "S1".into(), Right::Admin, Effect::Deny).unwrap();

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
    Policy::update(&mut s, &mut d, "S1".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s, &"S1".into(), Right::Write));
    assert!(Document::update(&mut s, &mut d, TextMutation::Insert { pos: 0, ch: 'x' }).is_err());
}

#[test]
fn toggle_deny_then_allow() {
    let mut sites = peers(&["A"]);
    let (mut s, mut d) = sites.pop().unwrap();
    Policy::update(&mut s, &mut d, "B".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s, &"B".into(), Right::Write));
    Policy::update(&mut s, &mut d, "B".into(), Right::Write, Effect::Allow).unwrap();
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
    Policy::effect(&mut state, &deny);

    let allow = PolicyOp {
        dot: Dot::new("S0", 2),
        target: "S1".into(),
        right: Right::Write,
        effect: Effect::Allow,
        deps: vec![deny.dot.clone()],
        last_dot_seen: Some(Dot::new("S1", 3)),
        missing_dots: vec![Dot::new("S1", 2)],
    };
    Policy::effect(&mut state, &allow);

    let covered_gap = DocOp { dot: Dot::new("S1", 2), deps: vec![], edit: TextMutation::Insert { pos: 0, ch: 'a' } };
    let seen_before_allow = DocOp {
        dot: Dot::new("S1", 3),
        deps: vec![],
        edit: TextMutation::Insert { pos: 1, ch: 'b' },
    };
    let concurrent_after_allow = DocOp {
        dot: Dot::new("S1", 4),
        deps: vec![],
        edit: TextMutation::Insert { pos: 2, ch: 'c' },
    };
    Document::effect(&mut state, &covered_gap);
    Document::effect(&mut state, &seen_before_allow);
    Document::effect(&mut state, &concurrent_after_allow);

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
        let op = Document::update(&mut s1, &mut d1, TextMutation::Insert { pos: 99, ch }).unwrap();
        doc_dots.push(op.dot);
    }
    assert_eq!(Document::compensation(&s1), "abcdef");

    // 7 concurrent operations total: 6 document writes on S1, 1 deny on S2.
    Policy::update(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Deny).unwrap();
    sync_pair(&mut d1, &mut d2);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    assert_eq!(Document::compensation(&s1), "");
    assert_eq!(Document::compensation(&s2), "");
    for dot in &doc_dots {
        assert_eq!(s1.valid.get(dot), Some(&false));
        assert_eq!(s2.valid.get(dot), Some(&false));
    }

    Policy::update(&mut s2, &mut d2, "S1".into(), Right::Write, Effect::Allow).unwrap();
    sync_pair(&mut d1, &mut d2);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    assert_eq!(Document::compensation(&s1), "abcdef");
    assert_eq!(Document::compensation(&s2), "abcdef");
    for dot in &doc_dots {
        assert_eq!(s1.valid.get(dot), Some(&true));
        assert_eq!(s2.valid.get(dot), Some(&true));
    }
}

/// Multi-peer cascading deny across two policy intervals requiring undo/redo
/// reconciliation on both S1 and S2.
///
/// Timeline:
///   Interval 0 (all allowed):
///     S1 inserts 'a','b'   — concurrently — S2 inserts 'x','y'
///   Interval 1 (S3 denies S1 Write):
///     S3 → deny(S1, Write); sync all → S1's 'a','b' undone
///     S2 inserts 'z' (still allowed)
///   Interval 2 (S3 denies S2 Write):
///     S3 → deny(S2, Write); sync all → S2's 'x','y','z' undone
///   Interval 3 (S3 re-allows both):
///     S3 → allow(S1, Write); S3 → allow(S2, Write); sync all
///     → S1's 'a','b' redone, S2's 'x','y','z' redone
///
/// Asserts convergence of all three peers after each interval transition.
#[test]
fn multi_peer_cascading_deny_undo_redo() {
    let mut sites = peers(&["S1", "S2", "S3"]);
    let (mut s3, mut d3) = sites.pop().unwrap();
    let (mut s2, mut d2) = sites.pop().unwrap();
    let (mut s1, mut d1) = sites.pop().unwrap();

    // -- Interval 0: S1 and S2 make concurrent document edits --
    let op_a = Document::update(&mut s1, &mut d1, TextMutation::Insert { pos: 0, ch: 'a' }).unwrap();
    let op_b = Document::update(&mut s1, &mut d1, TextMutation::Insert { pos: 99, ch: 'b' }).unwrap();
    assert_eq!(Document::compensation(&s1), "ab");

    let op_x = Document::update(&mut s2, &mut d2, TextMutation::Insert { pos: 0, ch: 'x' }).unwrap();
    let op_y = Document::update(&mut s2, &mut d2, TextMutation::Insert { pos: 99, ch: 'y' }).unwrap();
    assert_eq!(Document::compensation(&s2), "xy");

    let s1_dots = vec![op_a.dot.clone(), op_b.dot.clone()];
    let s2_dots_early = vec![op_x.dot.clone(), op_y.dot.clone()];

    // -- Interval 1: S3 denies S1 Write concurrently with the edits above --
    Policy::update(&mut s3, &mut d3, "S1".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s3, &"S1".into(), Right::Write));

    // Sync all peers
    for _ in 0..3 {
        sync_pair(&mut d1, &mut d2);
        sync_pair(&mut d2, &mut d3);
        sync_pair(&mut d1, &mut d3);
    }

    // Capture validity BEFORE rebuild to prove undo transition occurs
    let s1_valid_before: Vec<_> = s1_dots.iter().map(|d| s1.valid.get(d).copied()).collect();
    // S1's dots were initially valid (created locally with Write)
    assert!(s1_valid_before.iter().all(|v| *v == Some(true)),
        "S1 dots should be valid before rebuild: {:?}", s1_valid_before);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    rebuild_from_automerge(&mut s3, &mut d3);

    // Verify undo: S1's dots transitioned from valid → invalid (compensation)
    for dot in &s1_dots {
        assert_eq!(s1.valid.get(dot), Some(&false),
            "S1 dot {:?} must transition to invalid (undo) on S1", dot);
    }
    for dot in &s1_dots {
        assert_eq!(s1.valid.get(dot), Some(&false), "S1 dot {:?} should be invalid on S1", dot);
        assert_eq!(s2.valid.get(dot), Some(&false), "S1 dot {:?} should be invalid on S2", dot);
        assert_eq!(s3.valid.get(dot), Some(&false), "S1 dot {:?} should be invalid on S3", dot);
    }
    // S2's edits remain valid (S2 still has Write)
    for dot in &s2_dots_early {
        assert_eq!(s1.valid.get(dot), Some(&true), "S2 dot {:?} should be valid on S1", dot);
        assert_eq!(s2.valid.get(dot), Some(&true), "S2 dot {:?} should be valid on S2", dot);
        assert_eq!(s3.valid.get(dot), Some(&true), "S2 dot {:?} should be valid on S3", dot);
    }
    // All three peers converge on the same text (only S2's edits visible)
    let text_after_interval1 = Document::compensation(&s1);
    assert_eq!(text_after_interval1, Document::compensation(&s2));
    assert_eq!(text_after_interval1, Document::compensation(&s3));
    assert!(!text_after_interval1.contains('a') && !text_after_interval1.contains('b'));
    assert!(text_after_interval1.contains('x') && text_after_interval1.contains('y'));

    // -- S2 makes another edit while still allowed --
    let op_z = Document::update(&mut s2, &mut d2, TextMutation::Insert { pos: 99, ch: 'z' }).unwrap();
    let s2_dots_all = vec![op_x.dot.clone(), op_y.dot.clone(), op_z.dot.clone()];

    // -- Interval 2: S3 denies S2 Write --
    Policy::update(&mut s3, &mut d3, "S2".into(), Right::Write, Effect::Deny).unwrap();
    assert!(!eval(&s3, &"S2".into(), Right::Write));

    // Sync all peers
    for _ in 0..3 {
        sync_pair(&mut d1, &mut d2);
        sync_pair(&mut d2, &mut d3);
        sync_pair(&mut d1, &mut d3);
    }

    // Capture S2 validity BEFORE rebuild to prove undo transition occurs
    let s2_valid_before: Vec<_> = s2_dots_all.iter().map(|d| s2.valid.get(d).copied()).collect();
    // S2's dots were valid (created locally while S2 still had Write)
    assert!(s2_valid_before.iter().all(|v| *v == Some(true)),
        "S2 dots should be valid before rebuild: {:?}", s2_valid_before);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    rebuild_from_automerge(&mut s3, &mut d3);

    // Verify undo: S2's dots transitioned from valid → invalid (compensation)
    for dot in &s2_dots_all {
        assert_eq!(s2.valid.get(dot), Some(&false),
            "S2 dot {:?} must transition to invalid (undo) on S2", dot);
    }

    // Now both S1 and S2 edits are undone
    for dot in &s1_dots {
        assert_eq!(s1.valid.get(dot), Some(&false), "S1 dot {:?} still invalid after interval 2", dot);
        assert_eq!(s2.valid.get(dot), Some(&false));
        assert_eq!(s3.valid.get(dot), Some(&false));
    }
    for dot in &s2_dots_all {
        assert_eq!(s1.valid.get(dot), Some(&false), "S2 dot {:?} should be invalid after deny", dot);
        assert_eq!(s2.valid.get(dot), Some(&false));
        assert_eq!(s3.valid.get(dot), Some(&false));
    }
    // All peers converge on empty text
    assert_eq!(Document::compensation(&s1), "");
    assert_eq!(Document::compensation(&s2), "");
    assert_eq!(Document::compensation(&s3), "");

    // -- Interval 3: S3 re-allows both S1 and S2 Write (redo) --
    Policy::update(&mut s3, &mut d3, "S1".into(), Right::Write, Effect::Allow).unwrap();
    Policy::update(&mut s3, &mut d3, "S2".into(), Right::Write, Effect::Allow).unwrap();
    assert!(eval(&s3, &"S1".into(), Right::Write));
    assert!(eval(&s3, &"S2".into(), Right::Write));

    // Sync all peers
    for _ in 0..3 {
        sync_pair(&mut d1, &mut d2);
        sync_pair(&mut d2, &mut d3);
        sync_pair(&mut d1, &mut d3);
    }

    // Capture validity BEFORE rebuild to prove redo transition occurs
    let s1_valid_before_redo: Vec<_> = s1_dots.iter().map(|d| s1.valid.get(d).copied()).collect();
    let s2_valid_before_redo: Vec<_> = s2_dots_all.iter().map(|d| s2.valid.get(d).copied()).collect();
    // Both should be invalid before redo
    assert!(s1_valid_before_redo.iter().all(|v| *v == Some(false)),
        "S1 dots should be invalid before redo: {:?}", s1_valid_before_redo);
    assert!(s2_valid_before_redo.iter().all(|v| *v == Some(false)),
        "S2 dots should be invalid before redo: {:?}", s2_valid_before_redo);

    rebuild_from_automerge(&mut s1, &mut d1);
    rebuild_from_automerge(&mut s2, &mut d2);
    rebuild_from_automerge(&mut s3, &mut d3);

    // Verify redo: dots transitioned from invalid → valid (compensation)
    for dot in &s1_dots {
        assert_eq!(s1.valid.get(dot), Some(&true),
            "S1 dot {:?} must transition to valid (redo) on S1", dot);
        assert_eq!(s2.valid.get(dot), Some(&true),
            "S1 dot {:?} must transition to valid (redo) on S2", dot);
    }
    for dot in &s2_dots_all {
        assert_eq!(s1.valid.get(dot), Some(&true),
            "S2 dot {:?} must transition to valid (redo) on S1", dot);
        assert_eq!(s2.valid.get(dot), Some(&true),
            "S2 dot {:?} must transition to valid (redo) on S2", dot);
    }

    // Both S1 and S2 edits are redone on all peers
    for dot in &s1_dots {
        assert_eq!(s1.valid.get(dot), Some(&true), "S1 dot {:?} should be valid after allow", dot);
        assert_eq!(s2.valid.get(dot), Some(&true));
        assert_eq!(s3.valid.get(dot), Some(&true));
    }
    for dot in &s2_dots_all {
        assert_eq!(s1.valid.get(dot), Some(&true), "S2 dot {:?} should be valid after allow", dot);
        assert_eq!(s2.valid.get(dot), Some(&true));
        assert_eq!(s3.valid.get(dot), Some(&true));
    }
    // All peers converge on the same final text containing all edits
    let final_text = Document::compensation(&s1);
    assert_eq!(final_text, Document::compensation(&s2));
    assert_eq!(final_text, Document::compensation(&s3));
    assert!(final_text.contains('a') && final_text.contains('b'),
        "S1 edits missing from final text: {}", final_text);
    assert!(final_text.contains('x') && final_text.contains('y') && final_text.contains('z'),
        "S2 edits missing from final text: {}", final_text);
}
