use std::env;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::{IPC_MAGIC, MSG_COMMAND, MSG_TREE};

pub async fn ipc_connect() -> std::io::Result<UnixStream> {
    let path = env::var("SWAYSOCK")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "SWAYSOCK not set"))?;
    UnixStream::connect(&path).await
}

pub async fn ipc_send(
    stream: &mut UnixStream,
    msg_type: u32,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(14 + payload.len());
    buf.extend_from_slice(IPC_MAGIC);
    buf.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    buf.extend_from_slice(&msg_type.to_ne_bytes());
    buf.extend_from_slice(payload);
    stream.write_all(&buf).await
}

pub async fn ipc_recv(stream: &mut UnixStream) -> std::io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    stream.read_exact(&mut header).await?;
    let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
    let msg_type = u32::from_ne_bytes(header[10..14].try_into().unwrap());
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok((msg_type, body))
}

pub async fn sway_cmd(cmd: &str) -> std::io::Result<()> {
    let mut stream = ipc_connect().await?;
    ipc_send(&mut stream, MSG_COMMAND, cmd.as_bytes()).await?;
    ipc_recv(&mut stream).await?;
    Ok(())
}

pub async fn sway_tree() -> std::io::Result<Value> {
    let mut stream = ipc_connect().await?;
    ipc_send(&mut stream, MSG_TREE, &[]).await?;
    let (_, body) = ipc_recv(&mut stream).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
