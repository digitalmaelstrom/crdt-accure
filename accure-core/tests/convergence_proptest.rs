//! Random interleaving convergence test: apply a fixed multiset of
//! operations across N in-memory peers in arbitrary order, sync fully,
//! and assert each site's derived state is identical.

use accure_core::integrate::{
    current_text, new_shared_doc, new_state_from_bytes, rebuild_from_automerge, update_document,
    update_policy,
};
use accure_core::op::{Effect, Right, TextEdit};
use accure_core::state::Strategy as ConflictStrategy;
use automerge::{sync::SyncDoc, AutoCommit};
use proptest::prelude::{Strategy, *};

#[derive(Debug, Clone)]
enum Op {
    Insert(usize, char),
    Allow(String, Right),
    Sync,
}

fn arb_right() -> impl Strategy<Value = Right> {
    prop_oneof![Just(Right::Read), Just(Right::Write), Just(Right::Admin)]
}

fn arb_op(sites: Vec<&'static str>) -> impl Strategy<Value = (usize, Op)> {
    let site_n = sites.len();
    let s = sites;
    (0..site_n, 0..100u32).prop_flat_map(move |(who, kind)| {
        let s2 = s.clone();
        match kind % 3 {
            0 => (Just(who), (0usize..3, prop_oneof![Just('a'), Just('b'), Just('c')])
                .prop_map(|(p, c)| Op::Insert(p, c)))
                .boxed(),
            1 => (Just(who), (0..s2.len(), arb_right())
                .prop_map(move |(i, r)| Op::Allow(s2[i].to_string(), r)))
                .boxed(),
            _ => (Just(who), Just(Op::Sync)).boxed(),
        }
    })
}

fn sync_all(docs: &mut [AutoCommit]) {
    for _ in 0..6 {
        for i in 0..docs.len() {
            for j in 0..docs.len() {
                if i == j { continue; }
                let (left, right) = docs.split_at_mut(j.max(i));
                let (a, b) = if i < j {
                    (&mut left[i], &mut right[0])
                } else {
                    (&mut right[0], &mut left[j])
                };
                let mut sa = automerge::sync::State::new();
                let mut sb = automerge::sync::State::new();
                for _ in 0..4 {
                    if let Some(m) = a.sync().generate_sync_message(&mut sa) {
                        b.sync().receive_sync_message(&mut sb, m).unwrap();
                    }
                    if let Some(m) = b.sync().generate_sync_message(&mut sb) {
                        a.sync().receive_sync_message(&mut sa, m).unwrap();
                    }
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, .. ProptestConfig::default() })]

    #[test]
    fn random_interleavings_converge(
        ops in proptest::collection::vec(arb_op(vec!["A", "B", "C"]), 8..24)
    ) {
        let mut shared = new_shared_doc();
        let bytes = shared.save();
        let ids = ["A", "B", "C"];
        let mut peers: Vec<_> = ids
            .iter()
            .map(|id| new_state_from_bytes((*id).into(), ConflictStrategy::Integrity, &bytes).unwrap())
            .collect();

        for (who, op) in ops {
            let (state, doc) = &mut peers[who];
            match op {
                Op::Insert(p, c) => {
                    let _ = update_document(state, doc, TextEdit::Insert { pos: p, ch: c });
                }
                Op::Allow(target, right) => {
                    let _ = update_policy(state, doc, target, right, Effect::Allow);
                }
                Op::Sync => {
                    let mut docs: Vec<AutoCommit> =
                        peers.iter().map(|(_, d)| d.clone()).collect();
                    sync_all(&mut docs);
                    for (i, d) in docs.into_iter().enumerate() {
                        peers[i].1 = d;
                    }
                    for (s, d) in peers.iter_mut() {
                        rebuild_from_automerge(s, d);
                    }
                }
            }
        }

        // Final full sync + rebuild.
        let mut docs: Vec<AutoCommit> = peers.iter().map(|(_, d)| d.clone()).collect();
        sync_all(&mut docs);
        for (i, d) in docs.into_iter().enumerate() {
            peers[i].1 = d;
        }
        for (s, d) in peers.iter_mut() {
            rebuild_from_automerge(s, d);
        }

        let texts: Vec<String> = peers.iter().map(|(s, _)| current_text(s)).collect();
        let valids: Vec<_> = peers.iter().map(|(s, _)| s.valid.clone()).collect();
        prop_assert!(texts.iter().all(|t| t == &texts[0]), "texts diverge: {:?}", texts);
        prop_assert!(valids.iter().all(|v| v == &valids[0]), "validity diverges");
    }
}
