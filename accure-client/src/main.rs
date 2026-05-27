//! ACCURE TUI client binary.

mod ui;

use std::net::SocketAddr;

use accure_core::messages::{ClientCommand, ServerEvent};
use accure_core::wire::{read_frame, write_frame};
use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::sync::mpsc;

pub use accure_client::{parse_command, ParseError};

#[derive(Parser, Debug)]
#[command(name = "accure-client", about = "ACCURE protocol TUI client")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7100")]
    server: SocketAddr,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let sock = TcpStream::connect(args.server)
        .await
        .with_context(|| format!("connecting to {}", args.server))?;
    sock.set_nodelay(true).ok();
    let (read_half, mut write_half) = sock.into_split();

    let (ev_tx, ev_rx) = mpsc::channel::<ServerEvent>(256);
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ClientCommand>(64);

    let reader_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(read_half);
        loop {
            match read_frame::<_, ServerEvent>(&mut reader).await {
                Ok(ev) => { if ev_tx.send(ev).await.is_err() { break; } }
                Err(_) => break,
            }
        }
    });

    let writer_handle = tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if write_frame(&mut write_half, &cmd).await.is_err() { break; }
        }
    });

    let _ = cmd_tx.send(ClientCommand::Subscribe).await;
    let _ = cmd_tx.send(ClientCommand::Snapshot).await;

    let ui_result = ui::run(args.server, ev_rx, cmd_tx).await;
    reader_handle.abort();
    writer_handle.abort();
    ui_result
}
