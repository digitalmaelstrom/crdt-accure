//! Domain model types organized by concept.
//!
//! Provides `Policy` and `Document` as the primary entry points for
//! working with the ACCURE CRDT. Each type gathers its associated
//! operations, effects, and queries into a cohesive interface.

pub mod policy;
pub mod document;

pub use policy::Policy;
pub use document::Document;
