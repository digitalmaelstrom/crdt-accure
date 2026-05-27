//! Compensation / text rendering.
//!
//! For POC simplicity we render the document text by applying every
//! currently-valid `DocOp` in deterministic dot order against an empty
//! string. This sidesteps the need for an in-place undo of Automerge
//! operations: any validity flip simply re-renders the text. The
//! "compensation" trace is still surfaced for visualization.

use crate::dot::Dot;
use crate::op::{DocOp, Operation, TextEdit};

/// Render text by applying every doc op in `ops` (in `ops` order) that is
/// marked valid in `valid`.
pub fn render_text(ops: &[Operation], valid: &std::collections::BTreeMap<Dot, bool>) -> String {
    let mut docs: Vec<&DocOp> = ops.iter().filter_map(|o| o.as_document()).collect();
    docs.sort_by(|a, b| (a.dot.site.as_str(), a.dot.n).cmp(&(b.dot.site.as_str(), b.dot.n)));
    let mut s: Vec<char> = Vec::new();
    for d in docs {
        if !valid.get(&d.dot).copied().unwrap_or(false) { continue; }
        match &d.edit {
            TextEdit::Insert { pos, ch } => {
                let p = (*pos).min(s.len());
                s.insert(p, *ch);
            }
            TextEdit::Delete { pos } => {
                if *pos < s.len() { s.remove(*pos); }
            }
        }
    }
    s.into_iter().collect()
}
