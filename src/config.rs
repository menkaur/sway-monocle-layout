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

pub fn init(target_output: String, pidfile: Option<String>) {
    let pidfile = pidfile.unwrap_or_else(|| {
        format!(
            "/tmp/sway-monocle-{}.pid",
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
