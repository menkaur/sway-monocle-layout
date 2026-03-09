// ═══════════════════════════════════════════════════════════════
// FILE: src/focus_back.rs
// ROLE: Focus-back engine — restores focus to the "launcher"
//       window when a launched application closes.
//
// LLM CONTEXT:
//   This module implements a GLOBAL focus tracker that is
//   completely independent of the monitor management logic in
//   policy.rs.  It can run simultaneously with monitor mode
//   (separate PID file, separate event processing).
//
//   CORE CONCEPT: PARENT TRACKING
//
//   For every window that opens, the engine records "what
//   non-excluded window was focused when this window was created."
//   This is the window's "parent."  When a window closes AND it
//   was the focused window, focus is restored to its parent.
//
//   EXCLUSION LIST:
//   Certain apps (wofi, rofi, dmenu, etc.) are "transparent" in
//   the focus chain.  When an excluded app has focus:
//     • It is NOT recorded as a parent for new windows
//     • Its focus events don't update last_focused_normal
//
//   This means: terminal → wofi → app  →  app's parent = terminal
//   (wofi is skipped in the chain)
//
//   CHAIN WALKING:
//   When a window closes, if its parent is dead (also closed or
//   on a hidden workspace), the engine walks up the parent chain
//   until it finds a living ancestor on a visible workspace.
//
//   Example chain: terminal → app1 → app2
//   If app1 already closed and app2 closes:
//     parent(app2) = app1 → dead → parent(app1) = terminal → alive → focus terminal
//
//   EVENT PROCESSING:
//   Events are processed individually (NO debounce).  Order
//   matters for correct focus tracking.
//
//     "new":   Record parent = last_focused_normal
//     "focus": Update last_focused.  If app_id not excluded,
//              also update last_focused_normal.
//     "close": If closed == last_focused, walk parent chain
//              and focus the first living ancestor.
//
//   BACKGROUND CLOSES:
//   If a window closes but wasn't the focused window, no focus
//   restore is attempted.  This prevents stealing focus from
//   the user's current work.
//
// DEPENDENCIES:
//   • src/config.rs: DEFAULT_EXCLUDES
//   • src/events.rs: subscribe_events(), read_event(),
//     extract_window_info(), WindowEventInfo
//   • src/ipc.rs: sway_cmd()
//   • src/snapshot.rs: is_on_visible_workspace()
//   • src/pid.rs: enforce_single_instance(), cleanup_pidfile()
// ═══════════════════════════════════════════════════════════════

use std::collections::HashMap;

use serde_json::Value;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, Duration};

use crate::config::DEFAULT_EXCLUDES;
use crate::events::{extract_window_info, read_event, subscribe_events};
use crate::ipc::sway_cmd;
use crate::pid::{cleanup_pidfile, enforce_single_instance};
use crate::snapshot::is_on_visible_workspace;

// ── Engine ─────────────────────────────────────────────────────

struct FocusBackEngine {
    /// Maps child con_id → parent con_id.
    /// Parent = the non-excluded window that was focused when the
    /// child was created.
    ///
    /// Entries are NEVER removed — dead windows stay in the map
    /// so chain walking works (terminal → app1 → app2, where app1
    /// already closed).  Memory is negligible (~16 bytes per entry).
    parents: HashMap<i64, i64>,

    /// The most recently focused window (any app_id).
    /// Used to detect whether a closing window was the active one.
    last_focused: Option<i64>,

    /// The most recently focused NON-EXCLUDED window.
    /// Used as the parent when a new window opens.
    /// Excluded apps (wofi, rofi, etc.) don't update this field,
    /// making them transparent in the focus chain.
    last_focused_normal: Option<i64>,

    /// Lowercased app_ids that are transparent in the focus chain.
    excludes: Vec<String>,
}

impl FocusBackEngine {
    fn new(excludes: Vec<String>) -> Self {
        Self {
            parents: HashMap::new(),
            last_focused: None,
            last_focused_normal: None,
            excludes,
        }
    }

    /// Process one sway window event.
    async fn process_event(&mut self, event: &Value) {
        let info = match extract_window_info(event) {
            Some(i) => i,
            None => return,
        };

        // Only track leaf windows (pid > 0).
        // Containers (splits, workspaces) have pid = 0.
        if info.pid <= 0 {
            return;
        }

        match info.change.as_str() {
            "new" => self.on_new(info.con_id),
            "focus" => self.on_focus(info.con_id, &info.app_id),
            "close" => self.on_close(info.con_id).await,
            _ => {}
        }
    }

    /// A new window was created.
    /// Record its parent as the last non-excluded focused window.
    fn on_new(&mut self, con_id: i64) {
        if let Some(parent) = self.last_focused_normal {
            // Don't record self-parent (shouldn't happen, but safe)
            if parent != con_id {
                self.parents.insert(con_id, parent);
            }
        }
    }

    /// A window received focus.
    /// Update tracking.  If the app is excluded (launcher/menu),
    /// don't update last_focused_normal — this makes launchers
    /// transparent in the parent chain.
    fn on_focus(&mut self, con_id: i64, app_id: &Option<String>) {
        self.last_focused = Some(con_id);
        if !self.is_excluded(app_id) {
            self.last_focused_normal = Some(con_id);
        }
    }

    /// A window was closed.
    /// If it was the focused window, walk up the parent chain
    /// to find a living ancestor on a visible workspace and
    /// focus it.
    async fn on_close(&mut self, con_id: i64) {
        // Only act if the closed window was the focused one.
        // Background closes are ignored — don't steal focus.
        if self.last_focused != Some(con_id) {
            return;
        }

        // Walk up the parent chain.
        let mut target = self.parents.get(&con_id).copied();
        let mut visited = 0;

        while let Some(t) = target {
            // Safety: prevent infinite loops (cyclic parent chains)
            visited += 1;
            if visited > 100 {
                break;
            }

            // Don't focus self (shouldn't happen)
            if t == con_id {
                break;
            }

            if is_on_visible_workspace(t).await {
                sway_cmd(&format!("[con_id={t}] focus")).await.ok();
                return;
            }

            // Parent is dead or on hidden workspace — try grandparent
            target = self.parents.get(&t).copied();
        }

        // No living ancestor found — sway's default focus behavior
        // will handle it (focus next window in container).
    }

    /// Check if an app_id is in the exclusion list.
    fn is_excluded(&self, app_id: &Option<String>) -> bool {
        match app_id {
            Some(id) => self.excludes.iter().any(|e| e == id),
            None => false,
        }
    }
}

// ── Public entry point ─────────────────────────────────────────

/// Run the focus-back engine.
///
/// This function blocks forever (until SIGTERM/SIGINT).
/// It manages its own PID file and signal handlers.
///
/// # Arguments
/// * `extra_excludes` — additional app_ids to exclude (from CLI)
/// * `pidfile` — path to the PID file
pub async fn run(extra_excludes: Vec<String>, pidfile: String) {
    enforce_single_instance(&pidfile);

    let our_pid = std::process::id();

    // Build exclude list: defaults + user additions
    let mut excludes: Vec<String> = DEFAULT_EXCLUDES
        .iter()
        .map(|s| s.to_string())
        .collect();
    for e in extra_excludes {
        let lower = e.to_lowercase();
        if !excludes.contains(&lower) {
            excludes.push(lower);
        }
    }

    eprintln!("[focus-back] pid {our_pid}, pidfile '{pidfile}'");
    eprintln!(
        "[focus-back] excluding: {}",
        excludes.join(", ")
    );

    let mut sigterm =
        signal(SignalKind::terminate()).expect("failed to register SIGTERM");
    let mut sigint =
        signal(SignalKind::interrupt()).expect("failed to register SIGINT");

    let mut engine = FocusBackEngine::new(excludes);

    // ── Outer loop: reconnect on sway restart ──────────────────
    'outer: loop {
        let mut stream = match subscribe_events().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "[focus-back] subscription failed: {e}, retrying in 1s"
                );
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        // ── Inner loop: process events one at a time ───────────
        // NO debounce — order matters for correct tracking.
        loop {
            let event = tokio::select! {
                _ = sigterm.recv() => {
                    eprintln!(
                        "[focus-back] pid {our_pid} received SIGTERM"
                    );
                    break 'outer;
                }
                _ = sigint.recv() => {
                    eprintln!(
                        "[focus-back] pid {our_pid} received SIGINT"
                    );
                    break 'outer;
                }
                result = read_event(&mut stream) => {
                    match result {
                        Ok(e) => e,
                        Err(_) => break, // disconnected
                    }
                }
            };

            engine.process_event(&event).await;
        }

        eprintln!(
            "[focus-back] disconnected from sway, reconnecting in 1s"
        );
        sleep(Duration::from_secs(1)).await;
    }

    cleanup_pidfile(&pidfile);
    eprintln!("[focus-back] pid {our_pid} shutdown complete");
}
