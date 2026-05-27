#!/usr/bin/env bash
# bin/simulate.sh — ACCURE protocol simulation
#
# Starts three accure-server instances (S1, S2, S3) arranged in a full peer
# mesh, opens a tmux session with three horizontal panes each running
# accure-client connected to its own server, then fires a scripted series of
# policy and document operations through accure-send to exercise concurrency
# and the ACCURE protocol (CRDT convergence, policy grant/revoke,
# compensation).
#
# Usage:
#   ./bin/simulate.sh [--no-tmux]   # --no-tmux: scripted ops only, no TUI
#
# Prerequisites: tmux, cargo build already done (or will be built here).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$REPO_ROOT/target/debug"
SESSION="accure-sim"

# ── Ports ──────────────────────────────────────────────────────────────────
# Each server needs two ports: --listen (peer-to-peer) and --client (TUI/send)
S1_PEER=17001; S1_CLIENT=17101
S2_PEER=17002; S2_CLIENT=17102
S3_PEER=17003; S3_CLIENT=17103

LOG_DIR="/tmp/accure-sim-logs"
NO_TMUX=0
for arg in "$@"; do [[ "$arg" == "--no-tmux" ]] && NO_TMUX=1; done

# ── Helpers ────────────────────────────────────────────────────────────────
die()  { echo "ERROR: $*" >&2; exit 1; }
log()  { echo "[simulate] $*"; }
send() {
    # send <label> <client-port> <cmd>...
    local label="$1" port="$2"; shift 2
    "$BIN/accure-send" --server "127.0.0.1:$port" --label "$label" "$@"
}
wait_port() {
    local port="$1" timeout=30 i=0
    while ! ss -ltn 2>/dev/null | grep -q ":${port}[[:space:]]"; do
        sleep 0.3
        i=$(( i + 1 ))
        if [[ $i -gt $(( timeout * 3 )) ]]; then
            die "port $port never opened after ${timeout}s"
        fi
    done
}

# ── Build ──────────────────────────────────────────────────────────────────
log "Building workspace…"
source "$HOME/.cargo/env" 2>/dev/null || true
cargo build --workspace -q --manifest-path "$REPO_ROOT/Cargo.toml"

# ── Kill stale processes ───────────────────────────────────────────────────
log "Stopping any previous simulation processes…"
for port in $S1_PEER $S2_PEER $S3_PEER $S1_CLIENT $S2_CLIENT $S3_CLIENT; do
    # find PIDs listening on these ports and kill them
    pids=$(ss -tlnp 2>/dev/null | awk -v p=":$port " '$0 ~ p {
        match($0, /pid=([0-9]+)/, m); if (m[1]) print m[1]
    }')
    for pid in $pids; do kill "$pid" 2>/dev/null || true; done
done
# Kill any lingering tmux session from a prior run
tmux kill-session -t "$SESSION" 2>/dev/null || true
sleep 0.5

mkdir -p "$LOG_DIR"

# ── Start servers ──────────────────────────────────────────────────────────
log "Starting S1 (peer :$S1_PEER  client :$S1_CLIENT)…"
"$BIN/accure-server" \
    --id S1 \
    --listen "127.0.0.1:$S1_PEER" \
    --client "127.0.0.1:$S1_CLIENT" \
    --peer   "127.0.0.1:$S2_PEER" \
    --peer   "127.0.0.1:$S3_PEER" \
    > "$LOG_DIR/s1.log" 2>&1 &
S1_PID=$!

log "Starting S2 (peer :$S2_PEER  client :$S2_CLIENT)…"
"$BIN/accure-server" \
    --id S2 \
    --listen "127.0.0.1:$S2_PEER" \
    --client "127.0.0.1:$S2_CLIENT" \
    --peer   "127.0.0.1:$S1_PEER" \
    --peer   "127.0.0.1:$S3_PEER" \
    > "$LOG_DIR/s2.log" 2>&1 &
S2_PID=$!

log "Starting S3 (peer :$S3_PEER  client :$S3_CLIENT)…"
"$BIN/accure-server" \
    --id S3 \
    --listen "127.0.0.1:$S3_PEER" \
    --client "127.0.0.1:$S3_CLIENT" \
    --peer   "127.0.0.1:$S1_PEER" \
    --peer   "127.0.0.1:$S2_PEER" \
    > "$LOG_DIR/s3.log" 2>&1 &
S3_PID=$!

# Register cleanup
cleanup() {
    log "Shutting down servers (PIDs: $S1_PID $S2_PID $S3_PID)…"
    kill "$S1_PID" "$S2_PID" "$S3_PID" 2>/dev/null || true
    tmux kill-session -t "$SESSION" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

log "Waiting for servers to open client ports…"
wait_port "$S1_CLIENT"
wait_port "$S2_CLIENT"
wait_port "$S3_CLIENT"
log "All three servers are up."

# ── Launch tmux TUI panes ──────────────────────────────────────────────────
if [[ "$NO_TMUX" -eq 0 ]]; then
    log "Creating tmux session '$SESSION' with three horizontal panes…"

    # Create detached session with first pane → S1 client
    tmux new-session -d -s "$SESSION" -x 220 -y 50 \
        "$BIN/accure-client --server 127.0.0.1:$S1_CLIENT"

    # Split right → S2
    tmux split-window -t "$SESSION" -h \
        "$BIN/accure-client --server 127.0.0.1:$S2_CLIENT"

    # Split the rightmost pane right → S3
    tmux split-window -t "$SESSION:0.1" -h \
        "$BIN/accure-client --server 127.0.0.1:$S3_CLIENT"

    # Even out the panes
    tmux select-layout -t "$SESSION" even-horizontal

    log "tmux session '$SESSION' started."
    log "  Attach with:  tmux attach -t $SESSION"
    log "  Detach with:  Ctrl-b d"
    echo ""

    # Give the TUI clients a moment to connect and render before firing ops.
    sleep 2
fi

# ── Scripted simulation ────────────────────────────────────────────────────
log "═══════════════════════════════════════════════════════"
log " Phase 1 — Bootstrap: each site confirms initial policy"
log "═══════════════════════════════════════════════════════"
send S1 $S1_CLIENT "snapshot"
send S2 $S2_CLIENT "snapshot"
send S3 $S3_CLIENT "snapshot"
sleep 1

log "═══════════════════════════════════════════════════"
log " Phase 2 — Cross-site grants: S1 grants S2 and S3"
log "═══════════════════════════════════════════════════"
# S1 grants write to S2 and S3 then confirms
send S1 $S1_CLIENT \
    "allow S2 write" \
    "allow S3 write" \
    "snapshot"
sleep 1

# S2 grants read to S3 then confirms
send S2 $S2_CLIENT \
    "allow S3 read" \
    "snapshot"
sleep 1

log "═══════════════════════════════════════════════════════════════"
log " Phase 3 — Concurrent document edits from all three sites"
log "═══════════════════════════════════════════════════════════════"
# Fire inserts concurrently (subshell parallelism) to stress CRDT merging
(
    send S1 $S1_CLIENT \
        "insert 0 H" \
        "insert 1 e" \
        "insert 2 l" \
        "insert 3 l" \
        "insert 4 o"
) &
P3A=$!
(
    send S2 $S2_CLIENT \
        "insert 0 W" \
        "insert 1 o" \
        "insert 2 r" \
        "insert 3 l" \
        "insert 4 d"
) &
P3B=$!
(
    send S3 $S3_CLIENT \
        "insert 0 !" \
        "insert 1 ?" \
        "insert 2 ."
) &
P3C=$!
# Wait only for the Phase-3 subshells (not the long-running server PIDs).
wait "$P3A" "$P3B" "$P3C"
sleep 2

log "═══════════════════════════════════════════════════════════════"
log " Phase 4 — Snapshot all sites (should converge to same doc)"
log "═══════════════════════════════════════════════════════════════"
send S1 $S1_CLIENT "snapshot"
send S2 $S2_CLIENT "snapshot"
send S3 $S3_CLIENT "snapshot"
sleep 1

log "═══════════════════════════════════════════════════════════════"
log " Phase 5 — Policy revoke: S1 denies S3 write access"
log "═══════════════════════════════════════════════════════════════"
send S1 $S1_CLIENT "deny S3 write" "snapshot"
sleep 1

log "═══════════════════════════════════════════════════════════════"
log " Phase 6 — S3 attempts write after revocation (compensation)"
log "═══════════════════════════════════════════════════════════════"
send S3 $S3_CLIENT \
    "insert 0 X" \
    "snapshot"
sleep 2

log "═══════════════════════════════════════════════════════════════"
log " Phase 7 — S1 re-grants S3 write; compensated ops become valid"
log "═══════════════════════════════════════════════════════════════"
send S1 $S1_CLIENT "allow S3 write"
sleep 1
send S3 $S3_CLIENT "snapshot"
sleep 1

log "═══════════════════════════════════════════════════════════════"
log " Phase 8 — Delete operations from S2"
log "═══════════════════════════════════════════════════════════════"
send S2 $S2_CLIENT \
    "delete 0" \
    "delete 0" \
    "snapshot"
sleep 1

log "═══════════════════════════════════════════════════════════════"
log " Phase 9 — Final convergence snapshot"
log "═══════════════════════════════════════════════════════════════"
sleep 2
send S1 $S1_CLIENT "snapshot"
send S2 $S2_CLIENT "snapshot"
send S3 $S3_CLIENT "snapshot"

log ""
log "Simulation complete."
log "Server logs: $LOG_DIR/s{1,2,3}.log"

if [[ "$NO_TMUX" -eq 0 ]]; then
    log ""
    log "TUI clients are still running in tmux session '$SESSION'."
    log "Attach with:  tmux attach -t $SESSION"
    log "Press Ctrl-C here to stop servers and close the session."
    # Keep servers alive until Ctrl-C
    wait "$S1_PID" "$S2_PID" "$S3_PID" 2>/dev/null || true
fi
