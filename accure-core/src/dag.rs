//! Add-only monotonic DAG keyed by `Dot` for policy operations within a
//! single access tuple. Edges go from a dependency to the operation that
//! depends on it (matching `tuple.ADD_EDGE(Dep, o.Dot_Source)` in the
//! paper's CRDT – part 2).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::dot::Dot;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dag {
    /// All nodes ever added (idempotent on re-add).
    nodes: BTreeSet<Dot>,
    /// Outgoing edges: dep -> {dependents}.
    out: BTreeMap<Dot, BTreeSet<Dot>>,
    /// Incoming edges: op -> {its deps that are members of this DAG}.
    parents: BTreeMap<Dot, BTreeSet<Dot>>,
}

impl Dag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, d: Dot) {
        self.nodes.insert(d);
    }

    /// Add an edge `dep -> op`. Both endpoints are added to the node set.
    /// Idempotent — repeated calls are no-ops.
    pub fn add_edge(&mut self, dep: Dot, op: Dot) {
        self.nodes.insert(dep.clone());
        self.nodes.insert(op.clone());
        self.out.entry(dep.clone()).or_default().insert(op.clone());
        self.parents.entry(op).or_default().insert(dep);
    }

    pub fn contains(&self, d: &Dot) -> bool {
        self.nodes.contains(d)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Dot> {
        self.nodes.iter()
    }

    pub fn parents(&self, d: &Dot) -> impl Iterator<Item = &Dot> {
        self.parents.get(d).into_iter().flat_map(|s| s.iter())
    }

    pub fn children(&self, d: &Dot) -> impl Iterator<Item = &Dot> {
        self.out.get(d).into_iter().flat_map(|s| s.iter())
    }

    /// All ancestors of `d` (transitive closure of `parents`), excluding `d`.
    pub fn ancestors(&self, d: &Dot) -> BTreeSet<Dot> {
        let mut out = BTreeSet::new();
        let mut q: VecDeque<Dot> = self.parents(d).cloned().collect();
        while let Some(x) = q.pop_front() {
            if out.insert(x.clone()) {
                for p in self.parents(&x) {
                    q.push_back(p.clone());
                }
            }
        }
        out
    }

    /// Level (paper definition): shortest-path distance from any root
    /// (node with no parents inside the DAG) to `d`. Root nodes are level 1.
    pub fn level(&self, d: &Dot) -> usize {
        if !self.nodes.contains(d) {
            return 0;
        }
        let mut dist: BTreeMap<&Dot, usize> = BTreeMap::new();
        let mut q: VecDeque<&Dot> = VecDeque::new();
        for n in &self.nodes {
            if self.parents.get(n).map_or(true, |p| p.is_empty()) {
                dist.insert(n, 1);
                q.push_back(n);
            }
        }
        while let Some(x) = q.pop_front() {
            let dx = dist[x];
            for c in self.children(x) {
                let dc = dx + 1;
                let entry = dist.entry(c).or_insert(usize::MAX);
                if dc < *entry {
                    *entry = dc;
                    q.push_back(c);
                }
            }
        }
        dist.get(d).copied().unwrap_or(0)
    }

    /// All operations grouped by level.
    pub fn levels(&self) -> BTreeMap<usize, Vec<Dot>> {
        let mut by: BTreeMap<usize, Vec<Dot>> = BTreeMap::new();
        for n in &self.nodes {
            by.entry(self.level(n)).or_default().push(n.clone());
        }
        by
    }

    /// Maximum depth reached in this DAG. 0 if empty.
    pub fn max_level(&self) -> usize {
        self.nodes.iter().map(|n| self.level(n)).max().unwrap_or(0)
    }

    /// All nodes whose level equals the given level.
    pub fn at_level(&self, lvl: usize) -> Vec<Dot> {
        self.nodes.iter().filter(|n| self.level(n) == lvl).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str, n: u64) -> Dot { Dot::new(s, n) }

    #[test]
    fn idempotent_and_monotone() {
        let mut g = Dag::new();
        g.add_edge(d("S1", 1), d("S1", 2));
        g.add_edge(d("S1", 1), d("S1", 2));
        assert_eq!(g.nodes().count(), 2);
        assert_eq!(g.children(&d("S1", 1)).count(), 1);
    }

    #[test]
    fn levels_and_ancestors() {
        let mut g = Dag::new();
        // 1:1 -> 1:2 -> 1:3 ; 2:1 -> 1:3
        g.add_edge(d("S1", 1), d("S1", 2));
        g.add_edge(d("S1", 2), d("S1", 3));
        g.add_edge(d("S2", 1), d("S1", 3));
        assert_eq!(g.level(&d("S1", 1)), 1);
        assert_eq!(g.level(&d("S2", 1)), 1);
        assert_eq!(g.level(&d("S1", 2)), 2);
        assert_eq!(g.level(&d("S1", 3)), 2); // shortest path via S2:1
        let anc = g.ancestors(&d("S1", 3));
        assert!(anc.contains(&d("S1", 1)));
        assert!(anc.contains(&d("S1", 2)));
        assert!(anc.contains(&d("S2", 1)));
    }
}
