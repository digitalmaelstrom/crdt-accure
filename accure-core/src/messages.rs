//! Wire-level messages for both peer-to-peer and server-to-client links.

use serde::{Deserialize, Serialize};

use crate::dot::SiteId;
use crate::op::Right;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMessage {
    /// Sent immediately after TCP connection, identifying the sender.
    Hello { site: SiteId },
    /// An Automerge sync protocol message payload.
    Sync(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientCommand {
    /// Insert a character at `pos`.
    Insert { pos: usize, ch: char },
    /// Delete the character at `pos`.
    Delete { pos: usize },
    /// Grant a right to a site.
    Allow { target: SiteId, right: Right },
    /// Revoke a right from a site.
    Deny { target: SiteId, right: Right },
    /// Ask for an immediate full snapshot.
    Snapshot,
    /// Subscribe to live state and trace events.
    Subscribe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub site: SiteId,
    pub document: String,
    /// (site, right) -> Allow/Deny computed from current valid policy.
    pub policy: Vec<(SiteId, Right, bool)>,
    pub log_len: usize,
    pub peers: Vec<SiteId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerEvent {
    State(Snapshot),
    Trace(String),
    Error(String),
}
