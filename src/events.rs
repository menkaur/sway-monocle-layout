// ═══════════════════════════════════════════════════════════════
// FILE: src/events.rs
// ROLE: Sway event subscription, reading, and extraction.
//
// LLM CONTEXT:
//   TWO extraction APIs:
//
//   FOR MONITOR MODE (used by main.rs debounce):
//     • extract_hint(event) → Option<(i64, HintType)>
//       Returns con_id + New/Move from window events.
//
//   FOR FOCUS-BACK MODE (used by focus_back.rs):
//     • extract_window_info(event) → Option<WindowEventInfo>
//       Returns change type, con_id, app_id, and pid.
//       app_id checks both "app_id" (Wayland) and
//       "window_properties.class" (X11), lowercased.
//       pid is used to filter out non-leaf containers.
//
//   SHARED:
//     • subscribe_events() → UnixStream
//     • read_event(stream) → Value
//
// DEPENDENCIES:
//   • src/config.rs: MSG_SUBSCRIBE
//   • src/ipc.rs: ipc_connect(), ipc_send(), ipc_recv()
// ═══════════════════════════════════════════════════════════════

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

/// Detailed information from a sway window event.
///
/// Used by focus_back.rs to track focus chains and detect
/// window creation/closure.
#[derive(Debug)]
pub struct WindowEventInfo {
    /// Event type: "new", "close", "focus", "title", etc.
    pub change: String,
    /// Sway container ID of the affected window.
    pub con_id: i64,
    /// Application identifier (lowercase).
    /// Wayland: app_id.  X11: window_properties.class.
    /// None if the container has neither.
    pub app_id: Option<String>,
    /// Process ID.  > 0 for leaf windows, 0 for containers.
    /// Used to filter out non-leaf containers.
    pub pid: i64,
}

/// Extract detailed window info from a sway event.
///
/// Returns None for non-window events (workspace events, etc.)
/// that don't have a "container" field.
pub fn extract_window_info(event: &Value) -> Option<WindowEventInfo> {
    let change = event.get("change")?.as_str()?.to_owned();
    let container = event.get("container")?;
    let con_id = container.get("id")?.as_i64()?;
    let pid = container
        .get("pid")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Try Wayland app_id first, fall back to X11 class
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
