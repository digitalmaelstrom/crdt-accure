//! Client TCP listener.

use std::net::SocketAddr;
use std::sync::Arc;

use accure_core::messages::{ClientCommand, ServerEvent, Snapshot};
use accure_core::model::{Document, Policy};
use accure_core::model::document::TextMutation;
use accure_core::op::{Effect, Right};
use accure_core::wire::{read_frame, write_frame};
use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};

use crate::visualize::Event;
use crate::Server;

pub async fn listen(server: Arc<Server>, addr: SocketAddr) -> Result<()> {
    let lis = TcpListener::bind(addr).await?;
    tracing::info!("client listener on {addr}");
    loop {
        let (sock, addr) = lis.accept().await?;
        let srv = server.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(srv, sock).await {
                tracing::warn!("client {addr} ended: {e}");
            }
        });
    }
}

async fn handle(server: Arc<Server>, sock: TcpStream) -> Result<()> {
    let (read_half, mut write_half) = sock.into_split();
    let mut reader = tokio::io::BufReader::new(read_half);

    // Subscribe to events; forwarding task pushes them to the client.
    let mut events = server.events.subscribe();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerEvent>(64);
    let forward_tx = tx.clone();
    let forward = tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            let s = match ev {
                Event::PeerConnected { addr, site } => format!("peer {site} connected from {addr}"),
                Event::PeerDisconnected { site, reason } => format!("peer {site} disconnected: {reason}"),
                Event::SyncIn { peer, bytes } => format!("sync ← {peer} ({bytes} B)"),
                Event::SyncOut { peer, bytes } => format!("sync → {peer} ({bytes} B)"),
                Event::Trace(s) => s,
            };
            if forward_tx.send(ServerEvent::Trace(s)).await.is_err() {
                break;
            }
        }
    });

    let writer_task = tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if write_frame(&mut write_half, &ev).await.is_err() {
                break;
            }
        }
    });

    loop {
        let cmd: ClientCommand = match read_frame(&mut reader).await {
            Ok(c) => c,
            Err(_) => break,
        };
        let send_snapshot = |srv: Arc<Server>, tx: tokio::sync::mpsc::Sender<ServerEvent>| async move {
            let snap = build_snapshot(&srv).await;
            let _ = tx.send(ServerEvent::State(snap)).await;
        };
        match cmd {
            ClientCommand::Insert { pos, ch } => {
                let res = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    Document::update(&mut *st, &mut *doc, TextMutation::Insert { pos, ch })
                };
                match res {
                    Ok(op) => {
                        let _ = server.events.send(Event::Trace(format!("generated {op:?}")));
                        let _ = server.notify.send(());
                        send_snapshot(server.clone(), tx.clone()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(ServerEvent::Error(e.to_string())).await;
                    }
                }
            }
            ClientCommand::Delete { pos } => {
                let res = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    Document::update(&mut *st, &mut *doc, TextMutation::Delete { pos })
                };
                match res {
                    Ok(op) => {
                        let _ = server.events.send(Event::Trace(format!("generated {op:?}")));
                        let _ = server.notify.send(());
                        send_snapshot(server.clone(), tx.clone()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(ServerEvent::Error(e.to_string())).await;
                    }
                }
            }
            ClientCommand::Allow { target, right } => {
                let res = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    Policy::update(&mut *st, &mut *doc, target.clone(), right, Effect::Allow)
                };
                match res {
                    Ok(op) => {
                        let _ = server.events.send(Event::Trace(format!("generated {op:?}")));
                        let _ = server.notify.send(());
                        send_snapshot(server.clone(), tx.clone()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(ServerEvent::Error(e.to_string())).await;
                    }
                }
            }
            ClientCommand::Deny { target, right } => {
                let res = {
                    let mut st = server.state.lock().await;
                    let mut doc = server.doc.lock().await;
                    Policy::update(&mut *st, &mut *doc, target.clone(), right, Effect::Deny)
                };
                match res {
                    Ok(op) => {
                        let _ = server.events.send(Event::Trace(format!("generated {op:?}")));
                        let _ = server.notify.send(());
                        send_snapshot(server.clone(), tx.clone()).await;
                    }
                    Err(e) => {
                        let _ = tx.send(ServerEvent::Error(e.to_string())).await;
                    }
                }
            }
            ClientCommand::Snapshot | ClientCommand::Subscribe => {
                send_snapshot(server.clone(), tx.clone()).await;
            }
        }
    }
    forward.abort();
    writer_task.abort();
    Ok(())
}

pub async fn build_snapshot(server: &Server) -> Snapshot {
    let st = server.state.lock().await;
    let document = Document::compensate(&st);
    // Compute (site, right, allowed) for every known target site.
    let mut sites: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    sites.insert(st.me.clone());
    for op in &st.log {
        if let Some(p) = op.as_policy() {
            sites.insert(p.target.clone());
            sites.insert(p.dot.site.clone());
        }
        if let Some(d) = op.as_document() {
            sites.insert(d.dot.site.clone());
        }
    }
    let mut policy = Vec::new();
    for s in &sites {
        for r in [Right::Admin, Right::Read, Right::Write] {
            policy.push((s.clone(), r, accure_core::validity::eval(&st, s, r)));
        }
    }
    let peers: Vec<String> = st.peer_sync_states.keys().cloned().collect();
    Snapshot {
        site: st.me.clone(),
        document,
        policy,
        log_len: st.log.len(),
        peers,
    }
}
