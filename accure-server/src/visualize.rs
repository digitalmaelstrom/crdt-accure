//! Stdout visualizer: pretty-prints every protocol event so the server's
//! activity can be observed for the POC.

use std::net::SocketAddr;

use tokio::sync::broadcast::Receiver;

#[derive(Debug, Clone)]
pub enum Event {
    PeerConnected { addr: SocketAddr, site: String },
    PeerDisconnected { site: String, reason: String },
    SyncIn { peer: String, bytes: usize },
    SyncOut { peer: String, bytes: usize },
    Trace(String),
}

pub async fn run(site: &str, rx: &mut Receiver<Event>) {
    tracing::info!("[{site}] visualizer started");
    while let Ok(ev) = rx.recv().await {
        match ev {
            Event::PeerConnected { addr, site: s } => {
                tracing::info!("[{site}] peer connected: {s} ({addr})");
            }
            Event::PeerDisconnected { site: s, reason } => {
                tracing::info!("[{site}] peer disconnected: {s} ({reason})");
            }
            Event::SyncIn { peer, bytes } => {
                tracing::info!("[{site}] sync ← {peer} ({bytes} B)");
            }
            Event::SyncOut { peer, bytes } => {
                tracing::info!("[{site}] sync → {peer} ({bytes} B)");
            }
            Event::Trace(s) => {
                tracing::info!("[{site}] {s}");
            }
        }
    }
}
