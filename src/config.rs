// ═══════════════════════════════════════════════════════════════
// FILE: src/config.rs
// ROLE: Central configuration — constants and runtime parameters.
//
// LLM CONTEXT:
//   TWO categories:
//
//   COMPILE-TIME CONSTANTS:
//     • IPC protocol (IPC_MAGIC, MSG_COMMAND, MSG_SUBSCRIBE, MSG_TREE)
//     • Timing (DEBOUNCE, STABLE_INTERVAL, STABLE_MAX_TRIES)
//     • DEFAULT_EXCLUDES: app_ids that focus-back mode treats as
//       transparent in the focus chain (launchers, menus, etc.)
//
//   RUNTIME CONFIGURATION (OnceLock<RuntimeConfig>):
//     Only initialized in monitor mode via config::init().
//     Focus-back mode does NOT call init() — it manages its own
//     config as function parameters.
//
//     target_output(): sway output name (monitor mode only)
//     pidfile(): PID file path (monitor mode only)
//
// v3.5 ADDITION:
//   DEFAULT_EXCLUDES — app_ids excluded from focus-back tracking.
//   When an excluded app has focus and a new window opens, the
//   new window's parent is set to whatever was focused BEFORE the
//   excluded app.  This makes launchers (wofi, rofi) transparent:
//     terminal → wofi → app  →  app's parent = terminal
// ═══════════════════════════════════════════════════════════════

use std::sync::OnceLock;
use std::time::Duration;

// ── Sway IPC protocol ──────────────────────────────────────────

pub const IPC_MAGIC: &[u8; 6] = b"i3-ipc";
pub const MSG_COMMAND: u32 = 0;
pub const MSG_SUBSCRIBE: u32 = 2;
pub const MSG_TREE: u32 = 4;

// ── Timing ─────────────────────────────────────────────────────

pub const DEBOUNCE: Duration = Duration::from_millis(5);
pub const STABLE_INTERVAL: Duration = Duration::from_millis(1);
pub const STABLE_MAX_TRIES: usize = 100;

// ── Focus-back default exclusions ──────────────────────────────

/// App identifiers that are treated as transparent in the focus
/// chain.  When one of these is focused and a new window opens,
/// the new window's "parent" is set to whatever was focused
/// BEFORE the excluded app.
///
/// All comparisons are case-insensitive (lowercased on both sides).
///
/// Users can extend this list with --exclude on the CLI.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    "wofi",
    "rofi",
    "dmenu",
    "bemenu",
    "fuzzel",
    "tofi",
    "kickoff",
    "swaynag",
    "wlogout",
    "nwg-drawer",
    "ulauncher",
    "albert",
];

// ── Runtime configuration (monitor mode only) ──────────────────

#[derive(Debug)]
struct RuntimeConfig {
    target_output: String,
    pidfile: String,
}

static CONFIG: OnceLock<RuntimeConfig> = OnceLock::new();

/// Initialize monitor mode config.  NOT called in focus-back mode.
pub fn init(target_output: String, pidfile: Option<String>) {
    let pidfile = pidfile.unwrap_or_else(|| {
        format!(
            "/tmp/smart-borders-{}.pid",
            target_output.to_lowercase().replace(['/', ' '], "-")
        )
    });
    CONFIG
        .set(RuntimeConfig {
            target_output,
            pidfile,
        })
        .expect("config::init() called more than once");
}

pub fn target_output() -> &'static str {
    &CONFIG
        .get()
        .expect("config not initialized — call config::init() first")
        .target_output
}

pub fn pidfile() -> &'static str {
    &CONFIG
        .get()
        .expect("config not initialized — call config::init() first")
        .pidfile
}
