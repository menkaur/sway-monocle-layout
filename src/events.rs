use serde_json::Value;
use tokio::net::UnixStream;

use crate::config::MSG_SUBSCRIBE;
use crate::ipc::{ipc_connect, ipc_recv, ipc_send};

// ── Monitor mode: hint extraction ──────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintType {
    New,
    Move,
}

pub fn extract_hint(event: &Value) -> Option<(i64, HintType)> {
    let change = event.get("change")?.as_str()?;
    let id = event.get("container")?.get("id")?.as_i64()?;
    match change {
        "new" => Some((id, HintType::New)),
        "move" => Some((id, HintType::Move)),
        _ => None,
    }
}

// ── Focus-back mode: full event info ───────────────────────────

#[derive(Debug)]
pub struct WindowEventInfo {
    pub change: String,
    pub con_id: i64,
    pub app_id: Option<String>,
    pub pid: i64,
}

pub fn extract_window_info(event: &Value) -> Option<WindowEventInfo> {
    let change = event.get("change")?.as_str()?.to_owned();
    let container = event.get("container")?;
    let con_id = container.get("id")?.as_i64()?;
    let pid = container.get("pid").and_then(|v| v.as_i64()).unwrap_or(0);

    let app_id = container
        .get("app_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .or_else(|| {
            container
                .get("window_properties")
                .and_then(|wp| wp.get("class"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_lowercase())
        });

    Some(WindowEventInfo {
        change,
        con_id,
        app_id,
        pid,
    })
}

// ── Shared: subscription and reading ───────────────────────────

pub async fn subscribe_events() -> std::io::Result<UnixStream> {
    let mut stream = ipc_connect().await?;

    let payload = b"[\"window\",\"workspace\"]";
    ipc_send(&mut stream, MSG_SUBSCRIBE, payload).await?;

    let (_, body) = ipc_recv(&mut stream).await?;
    let resp: Value = serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    if resp.get("success").and_then(|v| v.as_bool()) != Some(true) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "sway subscription failed",
        ));
    }

    Ok(stream)
}

pub async fn read_event(stream: &mut UnixStream) -> std::io::Result<Value> {
    let (_, body) = ipc_recv(stream).await?;
    serde_json::from_slice(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
