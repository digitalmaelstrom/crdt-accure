//! Integration test: spawn two real `accure-server` processes connected
//! over loopback TCP. Drive each via the binary client wire protocol and
//! assert their snapshots converge.

use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;

use accure_core::messages::{ClientCommand, ServerEvent, Snapshot};
use accure_core::wire::{read_frame, write_frame};
use assert_cmd::cargo::CommandCargoExt;
use tokio::io::BufReader;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::time::sleep;

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

async fn wait_connect(addr: SocketAddr) -> TcpStream {
    for _ in 0..40 {
        if let Ok(s) = TcpStream::connect(addr).await {
            return s;
        }
        sleep(Duration::from_millis(200)).await;
    }
    panic!("never connected to {addr}");
}

async fn _send_unused(_sock: &mut TcpStream, _cmd: ClientCommand) {}

async fn drain_latest_snapshot(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    expected_text: &str,
    timeout_secs: u64,
) -> Snapshot {
    let mut last: Option<Snapshot> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() { break; }
        let next = tokio::time::timeout(remaining, read_frame::<_, ServerEvent>(reader)).await;
        match next {
            Ok(Ok(ServerEvent::State(s))) => {
                let matched = s.document == expected_text;
                last = Some(s);
                if matched { break; }
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    last.expect("at least one snapshot")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_server_convergence() {
    let p1_peer = free_port();
    let p2_peer = free_port();
    let p1_client = free_port();
    let p2_client = free_port();

    let mut s1 = Command::from(
        std::process::Command::cargo_bin("accure-server").unwrap(),
    )
    .args([
        "--id", "S1",
        "--listen", &format!("127.0.0.1:{p1_peer}"),
        "--client", &format!("127.0.0.1:{p1_client}"),
        "--peer", &format!("127.0.0.1:{p2_peer}"),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .expect("spawn S1");

    let mut s2 = Command::from(
        std::process::Command::cargo_bin("accure-server").unwrap(),
    )
    .args([
        "--id", "S2",
        "--listen", &format!("127.0.0.1:{p2_peer}"),
        "--client", &format!("127.0.0.1:{p2_client}"),
        "--peer", &format!("127.0.0.1:{p1_peer}"),
    ])
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .expect("spawn S2");

    let c1 = wait_connect(format!("127.0.0.1:{p1_client}").parse().unwrap()).await;
    let c2 = wait_connect(format!("127.0.0.1:{p2_client}").parse().unwrap()).await;
    let (r1, mut w1) = c1.into_split();
    let (r2, mut w2) = c2.into_split();
    let mut r1 = BufReader::new(r1);
    let mut r2 = BufReader::new(r2);

    // Subscribe both.
    write_frame(&mut w1, &ClientCommand::Subscribe).await.unwrap();
    write_frame(&mut w2, &ClientCommand::Subscribe).await.unwrap();

    // Wait for peer-to-peer connection to be established before any
    // writes, so the initial sync doesn't race with the inserts.
    sleep(Duration::from_secs(2)).await;

    // S1 inserts "h", "i".
    write_frame(&mut w1, &ClientCommand::Insert { pos: 0, ch: 'h' }).await.unwrap();
    write_frame(&mut w1, &ClientCommand::Insert { pos: 1, ch: 'i' }).await.unwrap();

    // Allow time for sync. Request snapshots periodically so we can
    // drain to the latest.
    for _ in 0..20 {
        write_frame(&mut w1, &ClientCommand::Snapshot).await.unwrap();
        write_frame(&mut w2, &ClientCommand::Snapshot).await.unwrap();
        sleep(Duration::from_millis(500)).await;
    }

    let snap1 = drain_latest_snapshot(&mut r1, "hi", 10).await;
    let snap2 = drain_latest_snapshot(&mut r2, "hi", 10).await;

    assert_eq!(snap1.document, "hi", "S1 doc state");
    assert_eq!(snap2.document, "hi", "S2 doc state");

    s1.start_kill().ok();
    s2.start_kill().ok();
}
