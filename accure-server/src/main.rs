//! ACCURE server binary.

mod peer;
mod client;
mod visualize;

use std::net::SocketAddr;
use std::sync::Arc;

use accure_core::integrate::{new_shared_doc, new_state_from_bytes};
use accure_core::state::{State, Strategy};
use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum StrategyArg {
    Integrity,
    Accessibility,
}

impl From<StrategyArg> for Strategy {
    fn from(a: StrategyArg) -> Self {
        match a {
            StrategyArg::Integrity => Strategy::Integrity,
            StrategyArg::Accessibility => Strategy::Accessibility,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "accure-server", about = "ACCURE protocol server")]
struct Args {
    #[arg(long)]
    id: String,
    #[arg(long, default_value = "127.0.0.1:7000")]
    listen: SocketAddr,
    #[arg(long, default_value = "127.0.0.1:7100")]
    client: SocketAddr,
    #[arg(long = "peer")]
    peers: Vec<SocketAddr>,
    #[arg(long, value_enum, default_value_t = StrategyArg::Integrity)]
    strategy: StrategyArg,
    #[arg(long, default_value_t = 500)]
    dial_delay_ms: u64,
}

pub struct Server {
    pub state: Mutex<State>,
    pub doc: Mutex<automerge::AutoCommit>,
    pub events: broadcast::Sender<visualize::Event>,
    pub notify: broadcast::Sender<()>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args = Args::parse();
    let mut shared = new_shared_doc();
    let bytes = shared.save();
    let (state, doc) = new_state_from_bytes(args.id.clone(), args.strategy.into(), &bytes)
        .context("init from shared doc bytes")?;
    let (events, _) = broadcast::channel::<visualize::Event>(256);
    let (notify, _) = broadcast::channel::<()>(64);
    let server = Arc::new(Server {
        state: Mutex::new(state),
        doc: Mutex::new(doc),
        events,
        notify,
    });

    {
        let mut rx = server.events.subscribe();
        let site = args.id.clone();
        tokio::spawn(async move { visualize::run(&site, &mut rx).await; });
    }

    {
        let srv = server.clone();
        let site = args.id.clone();
        let addr = args.listen;
        tokio::spawn(async move {
            if let Err(e) = peer::listen(srv, site, addr).await {
                tracing::error!("peer listener: {e}");
            }
        });
    }

    {
        let srv = server.clone();
        let site = args.id.clone();
        let peers = args.peers.clone();
        let delay = args.dial_delay_ms;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            for p in peers {
                let srv = srv.clone();
                let site = site.clone();
                tokio::spawn(async move {
                    if let Err(e) = peer::dial(srv, site, p).await {
                        tracing::error!("dial {p}: {e}");
                    }
                });
            }
        });
    }

    {
        let srv = server.clone();
        let addr = args.client;
        tokio::spawn(async move {
            if let Err(e) = client::listen(srv, addr).await {
                tracing::error!("client listener: {e}");
            }
        });
    }

    tokio::signal::ctrl_c().await.ok();
    tracing::info!("shutting down");
    Ok(())
}
