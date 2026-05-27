//! Peer-to-peer Automerge sync over TCP.

use std::net::SocketAddr;
use std::sync::Arc;

use accure_core::integrate::rebuild_from_automerge;
use accure_core::messages::PeerMessage;
use accure_core::wire::{read_frame, write_frame};
use anyhow::Result;
use automerge::sync::SyncDoc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

use crate::visualize::Event;
use crate::Server;

pub async fn listen(server: Arc<Server>, site: String, addr: SocketAddr) -> Result<()> {
    let lis = TcpListener::bind(addr).await?;
    tracing::info!("peer listener on {addr}");
    loop {
        let (sock, peer_addr) = lis.accept().await?;
        let srv = server.clone();
        let me = site.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_inbound(srv, me, sock, peer_addr).await {
                tracing::warn!("peer {peer_addr} disconnected: {e}");
            }
        });
    }
}

pub async fn dial(server: Arc<Server>, site: String, addr: SocketAddr) -> Result<()> {
    loop {
        match TcpStream::connect(addr).await {
            Ok(sock) => {
                tracing::info!("dialed peer {addr}");
                if let Err(e) = run_session(server.clone(), site.clone(), sock, addr, true).await {
                    tracing::warn!("session with {addr} ended: {e}");
                }
            }
            Err(e) => {
                tracing::debug!("dial {addr} failed: {e}");
            }
        }
        sleep(Duration::from_secs(2)).await;
    }
}

async fn handle_inbound(
    server: Arc<Server>,
    site: String,
    sock: TcpStream,
    addr: SocketAddr,
) -> Result<()> {
    run_session(server, site, sock, addr, false).await
}

async fn run_session(
    server: Arc<Server>,
    site: String,
    sock: TcpStream,
    addr: SocketAddr,
    initiator: bool,
) -> Result<()> {
    let (read_half, mut write_half) = sock.into_split();
    let mut reader = tokio::io::BufReader::new(read_half);

    write_frame(&mut write_half, &PeerMessage::Hello { site: site.clone() }).await?;
    let hello: PeerMessage = read_frame(&mut reader).await?;
    let peer_site = match hello {
        PeerMessage::Hello { site } => site,
        _ => return Err(anyhow::anyhow!("expected Hello from peer")),
    };
    let _ = server
        .events
        .send(Event::PeerConnected { addr, site: peer_site.clone() });

    // Outbound task: periodically generate sync messages and push them out.
    let (tx_out, mut rx_out) = mpsc::channel::<PeerMessage>(64);
    let mut notify_rx = server.notify.subscribe();
    let outbound = {
        let server = server.clone();
        let peer_site = peer_site.clone();
        tokio::spawn(async move {
            loop {
                // Try to generate a sync message.
                let msg = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    let mut sync_state = st
                        .peer_sync_states
                        .remove(&peer_site)
                        .unwrap_or_default();
                    let m = doc.sync().generate_sync_message(&mut sync_state);
                    st.peer_sync_states.insert(peer_site.clone(), sync_state);
                    m.map(|m| m.encode())
                };
                if let Some(bytes) = msg {
                    let n = bytes.len();
                    if tx_out.send(PeerMessage::Sync(bytes)).await.is_err() {
                        break;
                    }
                    let _ = server
                        .events
                        .send(Event::SyncOut { peer: peer_site.clone(), bytes: n });
                } else {
                    tokio::select! {
                        _ = notify_rx.recv() => {}
                        _ = sleep(Duration::from_millis(250)) => {}
                    }
                }
            }
        })
    };

    let writer_task = tokio::spawn(async move {
        while let Some(m) = rx_out.recv().await {
            if write_frame(&mut write_half, &m).await.is_err() {
                break;
            }
        }
    });

    // Inbound loop.
    loop {
        let msg: PeerMessage = match read_frame(&mut reader).await {
            Ok(m) => m,
            Err(e) => {
                let _ = server.events.send(Event::PeerDisconnected {
                    site: peer_site.clone(),
                    reason: e.to_string(),
                });
                outbound.abort();
                writer_task.abort();
                return Ok(());
            }
        };
        match msg {
            PeerMessage::Hello { .. } => {}
            PeerMessage::Sync(bytes) => {
                let n = bytes.len();
                let _ = server
                    .events
                    .send(Event::SyncIn { peer: peer_site.clone(), bytes: n });
                let traces = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    let mut sync_state = st
                        .peer_sync_states
                        .remove(&peer_site)
                        .unwrap_or_default();
                    let decoded = match automerge::sync::Message::decode(&bytes) {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!("decode sync from {peer_site}: {e}");
                            continue;
                        }
                    };
                    if let Err(e) = doc.sync().receive_sync_message(&mut sync_state, decoded) {
                        tracing::warn!("apply sync from {peer_site}: {e}");
                    }
                    st.peer_sync_states.insert(peer_site.clone(), sync_state);
                    rebuild_from_automerge(&mut *st, &mut *doc)
                };
                for t in traces {
                    let _ = server.events.send(Event::Trace(format!("{t:?}")));
                }
                let _ = server.notify.send(());
            }
        }
        let _ = initiator;
    }
}
