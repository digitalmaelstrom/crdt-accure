//! `accure-send` — headless command sender for scripting / simulation.
//!
//! Usage:
//!   accure-send --server 127.0.0.1:7101 [--label S1] <CMD> [<CMD> ...]
//!
//! Each CMD is a quoted command string:
//!   "insert <pos> <char>"  |  "delete <pos>"
//!   "allow <site> <a|r|w>" |  "deny <site> <a|r|w>"  |  "snapshot"
//!
//! After each command the tool drains the server's response (which always
//! includes a State snapshot) to keep the receive buffer clean.  Only
//! `snapshot` commands print the STATE line to stdout.  Trace events are
//! silently discarded so that concurrent subshells don't congest stdout.

use std::net::SocketAddr;
use std::time::Duration;

use accure_client::parse_command;
use accure_core::messages::{ClientCommand, ServerEvent};
use accure_core::wire::{read_frame, write_frame};
use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::BufReader;
use tokio::net::TcpStream;

#[derive(Parser, Debug)]
#[command(name = "accure-send", about = "Send scripted commands to an ACCURE server")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:7100")]
    server: SocketAddr,

    /// Human-readable label prepended to output lines.
    #[arg(long, default_value = "")]
    label: String,

    /// Commands to send (each as a quoted string, processed in order).
    #[arg(required = true)]
    cmds: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let label = if args.label.is_empty() {
        args.server.to_string()
    } else {
        args.label.clone()
    };

    let sock = TcpStream::connect(args.server)
        .await
        .with_context(|| format!("connecting to {}", args.server))?;
    sock.set_nodelay(true).ok();
    let (r, mut w) = sock.into_split();
    let mut reader = BufReader::new(r);

    for raw in &args.cmds {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let cmd = parse_command(raw).map_err(|e| anyhow::anyhow!("{e}"))?;
        let print_state = matches!(cmd, ClientCommand::Snapshot);

        write_frame(&mut w, &cmd)
            .await
            .with_context(|| format!("send: {raw}"))?;

        // The server emits a State snapshot after every command.  Drain until
        // we receive it (or 300 ms pass) so subsequent commands don't pick up
        // stale responses.  Trace events are silently dropped.
        let deadline = tokio::time::sleep(Duration::from_millis(300));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                res = read_frame::<_, ServerEvent>(&mut reader) => {
                    match res {
                        Ok(ServerEvent::State(s)) => {
                            if print_state {
                                println!("[{label}] STATE site={} doc={:?} log={} peers={}",
                                    s.site, s.document, s.log_len, s.peers.join(","));
                            }
                            break;
                        }
                        Ok(_) => {} // discard Trace / Error
                        Err(e) => {
                            eprintln!("[{label}] CONN {e}");
                            return Ok(());
                        }
                    }
                }
                _ = &mut deadline => break,
            }
        }
    }
    Ok(())
}
