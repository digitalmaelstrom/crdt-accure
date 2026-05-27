//! Helpers around the Automerge document used by both server and tests.

use automerge::{sync, sync::SyncDoc, AutoCommit};

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("automerge: {0}")]
    Automerge(#[from] automerge::AutomergeError),
    #[error("sync decode: {0}")]
    SyncDecode(String),
}

pub struct Bridge {
    pub doc: AutoCommit,
    pub sync_states: std::collections::HashMap<String, sync::State>,
}

impl Bridge {
    pub fn new(doc: AutoCommit) -> Self {
        Self { doc, sync_states: Default::default() }
    }

    pub fn generate(&mut self, peer: &str) -> Option<Vec<u8>> {
        let st = self.sync_states.entry(peer.to_string()).or_insert_with(sync::State::new);
        self.doc.sync().generate_sync_message(st).map(|m| m.encode())
    }

    pub fn receive(&mut self, peer: &str, bytes: &[u8]) -> Result<(), BridgeError> {
        let msg = sync::Message::decode(bytes).map_err(|e| BridgeError::SyncDecode(e.to_string()))?;
        let st = self.sync_states.entry(peer.to_string()).or_insert_with(sync::State::new);
        self.doc.sync().receive_sync_message(st, msg)?;
        Ok(())
    }
}
