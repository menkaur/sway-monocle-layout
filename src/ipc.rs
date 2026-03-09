// ═══════════════════════════════════════════════════════════════
// FILE: src/ipc.rs
// ROLE: Low-level sway IPC transport layer.
//
// LLM CONTEXT:
//   This file implements the sway/i3 IPC binary protocol over
//   Unix domain sockets.  It provides two levels of abstraction:
//
//   LOW-LEVEL (used by src/events.rs for persistent subscriptions):
//     • ipc_connect()  — opens a new UnixStream to $SWAYSOCK
//     • ipc_send()     — writes a framed IPC message
//     • ipc_recv()     — reads one framed IPC response/event
//
//   HIGH-LEVEL (used by src/snapshot.rs and src/policy.rs):
//     • sway_cmd(cmd)  — runs a sway command, returns JSON result
//     • sway_tree()    — fetches the full container tree as JSON
//
//   Each high-level call opens a NEW connection (ipc_oneshot),
//   sends one request, reads one response, and closes.  This is
//   intentional: it avoids stale-connection issues and is still
//   <0.5 ms per round-trip on a local Unix socket.
//
// DEPENDENCIES:
//   • src/config.rs: IPC_MAGIC, MSG_COMMAND, MSG_TREE
//     (compile-time constants only — no runtime config needed)
//
// WIRE FORMAT (per i3 IPC spec):
//   Request/Response = "i3-ipc" (6 bytes)
//                    + payload_length (u32 LE, 4 bytes)
//                    + message_type   (u32 LE, 4 bytes)
//                    + payload        (payload_length bytes)
// ═══════════════════════════════════════════════════════════════

use std::env;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::{IPC_MAGIC, MSG_COMMAND, MSG_TREE};

/// Resolve the sway IPC socket path from the SWAYSOCK environment
/// variable.  Returns an error if the variable is not set (e.g.
/// running outside a sway session).
fn swaysock_path() -> std::io::Result<String> {
    env::var("SWAYSOCK")
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::NotFound, e))
}

/// Open a new async Unix domain socket connection to sway's IPC.
///
/// Each connection is independent — sway supports unlimited
/// concurrent connections.  Callers are responsible for closing
/// the stream when done (Rust's Drop handles this automatically).
pub async fn ipc_connect() -> std::io::Result<UnixStream> {
    UnixStream::connect(swaysock_path()?).await
}

/// Write one IPC-framed message to an open stream.
///
/// Format: 6-byte magic + 4-byte payload length (LE) +
///         4-byte message type (LE) + payload bytes.
pub async fn ipc_send(
    stream: &mut UnixStream,
    msg_type: u32,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(14 + payload.len());
    buf.extend_from_slice(IPC_MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&msg_type.to_le_bytes());
    buf.extend_from_slice(payload);
    stream.write_all(&buf).await?;
    stream.flush().await
}

/// Read one IPC-framed response or event from an open stream.
///
/// Returns (message_type, body_bytes).  Validates the magic prefix
/// and returns an error if it doesn't match (corrupted stream).
pub async fn ipc_recv(stream: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut hdr = [0u8; 14];
    stream.read_exact(&mut hdr).await?;
    if &hdr[..6] != IPC_MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad IPC magic — stream may be corrupted or out of sync",
        ));
    }
    let len = u32::from_le_bytes(hdr[6..10].try_into().unwrap()) as usize;
    let mtype = u32::from_le_bytes(hdr[10..14].try_into().unwrap());
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok((mtype, body))
}

/// Open a fresh connection, send one request, read one response.
///
/// This is the building block for all one-shot queries.  Each call
/// gets its own connection, so there's no risk of reading a stale
/// response from a previous query.
async fn ipc_oneshot(msg_type: u32, payload: &[u8]) -> std::io::Result<Value> {
    let mut s = ipc_connect().await?;
    ipc_send(&mut s, msg_type, payload).await?;
    let (_, body) = ipc_recv(&mut s).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Run a sway command string and return the JSON result array.
///
/// Example: `sway_cmd("[con_id=42] fullscreen enable").await`
///
/// Sway returns `[{"success": true}]` for each semicolon-separated
/// sub-command.  Callers typically use `.ok()` to discard the
/// result, since failures are non-fatal in this daemon.
pub async fn sway_cmd(cmd: &str) -> std::io::Result<Value> {
    ipc_oneshot(MSG_COMMAND, cmd.as_bytes()).await
}

/// Fetch the complete sway container tree as a JSON Value.
///
/// The tree contains all outputs, workspaces, containers, and
/// windows.  Typical response is 10-100 KB depending on how many
/// windows are open.  Parsing takes <0.1 ms with serde_json.
pub async fn sway_tree() -> std::io::Result<Value> {
    ipc_oneshot(MSG_TREE, b"").await
}
