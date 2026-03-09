use std::collections::HashMap;

use serde_json::Value;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, Duration};

use crate::config::DEFAULT_EXCLUDES;
use crate::events::{extract_window_info, read_event, subscribe_events};
use crate::ipc::sway_cmd;
use crate::pid::{cleanup_pidfile, enforce_single_instance};
use crate::snapshot::is_on_visible_workspace;

struct FocusBackEngine {
    parents: HashMap<i64, i64>,
    last_focused: Option<i64>,
    last_focused_normal: Option<i64>,
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

    async fn process_event(&mut self, event: &Value) {
        let info = match extract_window_info(event) {
            Some(i) => i,
            None => return,
        };

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

    fn on_new(&mut self, con_id: i64) {
        if let Some(parent) = self.last_focused_normal {
            if parent != con_id {
                self.parents.insert(con_id, parent);
            }
        }
    }

    fn on_focus(&mut self, con_id: i64, app_id: &Option<String>) {
        self.last_focused = Some(con_id);
        if !self.is_excluded(app_id) {
            self.last_focused_normal = Some(con_id);
        }
    }

    async fn on_close(&mut self, con_id: i64) {
        if self.last_focused != Some(con_id) {
            return;
        }

        let mut target = self.parents.get(&con_id).copied();
        let mut visited = 0;

        while let Some(t) = target {
            visited += 1;
            if visited > 100 {
                break;
            }

            if t == con_id {
                break;
            }

            if is_on_visible_workspace(t).await {
                sway_cmd(&format!("[con_id={t}] focus")).await.ok();
                return;
            }

            target = self.parents.get(&t).copied();
        }
    }

    fn is_excluded(&self, app_id: &Option<String>) -> bool {
        match app_id {
            Some(id) => self.excludes.iter().any(|e| e == id),
            None => false,
        }
    }
}

pub async fn run(extra_excludes: Vec<String>, pidfile: String) {
    enforce_single_instance(&pidfile);

    let our_pid = std::process::id();

    let mut excludes: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
    for e in extra_excludes {
        let lower = e.to_lowercase();
        if !excludes.contains(&lower) {
            excludes.push(lower);
        }
    }

    eprintln!("[focus-back] pid {our_pid}, pidfile '{pidfile}'");
    eprintln!("[focus-back] excluding: {}", excludes.join(", "));

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT");

    let mut engine = FocusBackEngine::new(excludes);

    'outer: loop {
        let mut stream = match subscribe_events().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[focus-back] subscription failed: {e}, retrying in 1s");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

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
                        Err(_) => break,
                    }
                }
            };

            engine.process_event(&event).await;
        }

        eprintln!("[focus-back] disconnected from sway, reconnecting in 1s");
        sleep(Duration::from_secs(1)).await;
    }

    cleanup_pidfile(&pidfile);
    eprintln!("[focus-back] pid {our_pid} shutdown complete");
}
