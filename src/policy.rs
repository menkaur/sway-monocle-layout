// ═══════════════════════════════════════════════════════════════
// FILE: src/policy.rs
// ROLE: The decision-making engine.
//
// LLM CONTEXT:
//   v3.5 — DEPARTED WINDOW CLEANUP:
//
//   Added restore_departed_windows(): when a window with _auto_fs
//   and border none moves away from the managed output, the daemon
//   restores its original border and removes the _auto_fs mark.
//
//   This eliminates the need for move-to-monitor.sh to check for
//   _auto_fs, undo fullscreen/border, capture con_id, find window
//   center, or do a resize nudge.  The script becomes minimal.
//
//   DETECTION: compare saved_borders keys against current tiled
//   list.  Any key NOT in tiled is a departed window.  Batch-
//   restore borders + unmark, then remove from HashMap.
//
//   prune_saved_borders REMOVED — restore_departed_windows handles
//   all cleanup.  Entries are only cleared after a successful
//   sway_cmd.  Dead entries from closed windows are negligible
//   (~16 bytes each).  This prevents IPC failure from losing
//   saved borders.
//
//   ALL OTHER BEHAVIOR UNCHANGED FROM v3.4.
//
// DEPENDENCIES:
//   • src/ipc.rs: sway_cmd()
//   • src/snapshot.rs: snapshot(), snapshot_stable(),
//     is_on_visible_workspace()
//   • src/tree.rs: Snapshot, WinInfo
// ═══════════════════════════════════════════════════════════════

use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use crate::ipc::sway_cmd;
use crate::snapshot::{is_on_visible_workspace, snapshot, snapshot_stable};
use crate::tree::{Snapshot, WinInfo};

// ── State-change detection ─────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
struct Dp2State {
    windows: Vec<(i64, i64)>,
    float_n: usize,
    float_fs: usize,
    ws_layout: String,
}

impl Dp2State {
    fn from_snapshot(snap: &Snapshot) -> Self {
        let mut windows: Vec<(i64, i64)> = snap
            .tiled
            .iter()
            .map(|w| (w.id, w.fs))
            .collect();
        windows.sort();
        Self {
            windows,
            float_n: snap.float_n,
            float_fs: snap.float_fs,
            ws_layout: snap.ws_layout.clone(),
        }
    }

    fn contains_window(&self, id: i64) -> bool {
        self.windows.iter().any(|(wid, _)| *wid == id)
    }
}

// ── Policy engine ──────────────────────────────────────────────

pub struct Policy {
    prev_focused: Option<i64>,
    cur_focused: Option<i64>,
    saved_focused: Option<i64>,
    floating_fs_active: bool,
    last_state: Option<Dp2State>,
    saved_borders: HashMap<i64, (String, i64)>,
    last_global_focused: Option<i64>,
}

impl Policy {
    pub fn new() -> Self {
        Self {
            prev_focused: None,
            cur_focused: None,
            saved_focused: None,
            floating_fs_active: false,
            last_state: None,
            saved_borders: HashMap::new(),
            last_global_focused: None,
        }
    }

    pub async fn apply(&mut self, hint: Option<i64>) {
        if let Err(e) = self.run(hint).await {
            eprintln!("[smart-borders] policy error: {e}");
        }
    }

    fn needs_action(&self, snap: &Snapshot) -> bool {
        match snap.tiled.len() {
            0 => false,
            1 => {
                let w = &snap.tiled[0];
                if w.is_fs() && !w.has_auto_fs() {
                    return false;
                }
                if snap.float_n > 0 {
                    return false;
                }
                if !w.has_auto_fs() && !w.is_fs() {
                    return true;
                }
                if w.has_auto_fs() && snap.ws_layout != "splith" {
                    return true;
                }
                false
            }
            _ => snap.tiled.iter().any(|w| w.has_auto_fs()),
        }
    }

    // ── Border save / restore ──────────────────────────────────

    fn save_border(&mut self, w: &WinInfo) {
        self.saved_borders
            .entry(w.id)
            .or_insert_with(|| (w.border.clone(), w.border_width));
    }

    fn border_restore_cmd(&self, id: i64) -> String {
        match self.saved_borders.get(&id) {
            Some((style, width)) => match style.as_str() {
                "none" => "border none".to_owned(),
                "pixel" => format!("border pixel {width}"),
                "csd" => "border csd".to_owned(),
                _ => format!("border normal {width}"),
            },
            None => "border normal".to_owned(),
        }
    }

    fn clear_saved_border(&mut self, id: i64) {
        self.saved_borders.remove(&id);
    }

    /// Restore borders and remove _auto_fs marks on windows that
    /// left this output (moved to another output or closed).
    ///
    /// For moved windows: commands restore the original border.
    /// For closed windows: sway ignores commands for dead con_ids.
    ///
    /// Entries are ONLY cleared from saved_borders after a
    /// successful sway_cmd.  On IPC failure, entries persist so
    /// the next cycle retries with the correct border.
    ///
    /// This replaces prune_saved_borders — no separate prune step
    /// is needed.  Dead entries from closed windows where sway_cmd
    /// succeeded are cleaned up.  Dead entries from closed windows
    /// where sway_cmd failed persist but are negligible (~16 bytes).
    async fn restore_departed_windows(&mut self, snap: &Snapshot) {
        let current_ids: Vec<i64> = snap.tiled.iter().map(|w| w.id).collect();

        let departed: Vec<(i64, String)> = self
            .saved_borders
            .keys()
            .filter(|id| !current_ids.contains(id))
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|id| {
                let restore = self.border_restore_cmd(id);
                (id, restore)
            })
            .collect();

        if departed.is_empty() {
            return;
        }

        let mut cmd = String::new();
        for (id, restore) in &departed {
            cmd.push_str(&format!(
                "[con_id={id}] {restore}; \
                 [con_id={id}] unmark _auto_fs; "
            ));
        }

        let result = sway_cmd(&cmd).await;

        if result.is_ok() {
            for (id, _) in &departed {
                self.saved_borders.remove(id);
            }
        }
    }

    // ── Core logic ─────────────────────────────────────────────

    async fn run(
        &mut self,
        hint: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let snap = match snapshot_stable(hint).await {
            Some(s) => s,
            None => return Ok(()),
        };

        let relevant_hint = hint.filter(|h| {
            let on_output = snap.tiled.iter().any(|w| w.id == *h);
            let is_new_arrival = self
                .last_state
                .as_ref()
                .map(|ls| !ls.contains_window(*h))
                .unwrap_or(true);
            on_output && is_new_arrival
        });

        let current_state = Dp2State::from_snapshot(&snap);
        if relevant_hint.is_none() {
            if let Some(ref last) = self.last_state {
                if *last == current_state && !self.needs_action(&snap) {
                    self.last_global_focused = snap.global_focused;
                    return Ok(());
                }
            }
        }

        self.last_state = Some(current_state);

        // Restore borders on windows that left this output.
        // Replaces the old prune_saved_borders — entries are only
        // cleared after successful restore, preventing border loss
        // on IPC failure.
        self.restore_departed_windows(&snap).await;

        let tree_focus = snap.tiled.iter().find(|w| w.focused).map(|w| w.id);
        self.prev_focused = self.cur_focused;
        if tree_focus.is_some() {
            self.cur_focused = tree_focus;
        }

        if snap.float_fs > 0 {
            if !self.floating_fs_active {
                self.saved_focused = self.last_global_focused;
                self.floating_fs_active = true;
            }
            self.last_global_focused = snap.global_focused;
            return Ok(());
        }

        if self.floating_fs_active {
            self.floating_fs_active = false;

            if let Some(id) = self.saved_focused.take() {
                if is_on_visible_workspace(id).await {
                    sway_cmd(&format!("[con_id={id}] focus")).await.ok();
                }
            }

            let snap = match snapshot().await {
                Some(s) => s,
                None => return Ok(()),
            };
            self.last_state = Some(Dp2State::from_snapshot(&snap));
            self.last_global_focused = snap.global_focused;
            let result = self.dispatch(snap, relevant_hint).await;
            self.update_post_dispatch_state().await;
            return result;
        }

        let result = self.dispatch(snap, relevant_hint).await;
        self.update_post_dispatch_state().await;
        result
    }

    async fn update_post_dispatch_state(&mut self) {
        if let Some(post_snap) = snapshot().await {
            self.last_state = Some(Dp2State::from_snapshot(&post_snap));
            self.last_global_focused = post_snap.global_focused;
        }
    }

    async fn dispatch(
        &mut self,
        snap: Snapshot,
        hint: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match snap.tiled.len() {
            0 => Ok(()),
            1 => self.handle_single(&snap).await,
            _ => self.handle_multi(snap, hint).await,
        }
    }

    // ── Single-window policy ───────────────────────────────────

    async fn handle_single(
        &mut self,
        snap: &Snapshot,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let w = &snap.tiled[0];

        if w.is_fs() && !w.has_auto_fs() {
            return Ok(());
        }

        if snap.float_n > 0 {
            if w.has_auto_fs() {
                let restore = self.border_restore_cmd(w.id);
                let result = sway_cmd(&format!(
                    "[con_id={0}] {restore}; \
                     [con_id={0}] unmark _auto_fs",
                    w.id
                ))
                .await;

                if result.is_ok() {
                    self.clear_saved_border(w.id);
                }
            }
            return Ok(());
        }

        if !w.has_auto_fs() {
            self.save_border(w);

            sway_cmd(&format!(
                "[con_id={0}] split none; \
                 [workspace={1}] layout splith; \
                 [con_id={0}] mark --add _auto_fs; \
                 [con_id={0}] border none",
                w.id, snap.ws_name
            ))
            .await
            .ok();
        } else if snap.ws_layout != "splith" {
            sway_cmd(&format!(
                "[workspace={}] layout splith",
                snap.ws_name
            ))
            .await
            .ok();
        }

        Ok(())
    }

    // ── Multi-window policy ────────────────────────────────────

    async fn handle_multi(
        &mut self,
        snap: Snapshot,
        hint: Option<i64>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if snap.tiled.iter().any(|w| w.is_fs() && !w.has_auto_fs()) {
            return Ok(());
        }

        let auto_fs_ids: Vec<i64> = snap
            .tiled
            .iter()
            .filter(|w| w.has_auto_fs())
            .map(|w| w.id)
            .collect();
        let transitioning = !auto_fs_ids.is_empty();

        if transitioning {
            let mut cmd = String::new();

            for id in &auto_fs_ids {
                let restore = self.border_restore_cmd(*id);
                cmd.push_str(&format!(
                    "[con_id={id}] {restore}; \
                     [con_id={id}] unmark _auto_fs; "
                ));
            }

            for w in &snap.tiled {
                cmd.push_str(&format!(
                    "[con_id={}] split none; ", w.id
                ));
            }

            cmd.push_str(&format!(
                "[workspace={}] layout tabbed",
                snap.ws_name
            ));

            let cmd_result = sway_cmd(&cmd).await;

            if cmd_result.is_ok() {
                for id in &auto_fs_ids {
                    self.clear_saved_border(*id);
                }
            }

            if snap.any_focused {
                let focus_target = hint
                    .or_else(|| {
                        snap.tiled
                            .iter()
                            .find(|w| !w.has_auto_fs())
                            .map(|w| w.id)
                    })
                    .or_else(|| snap.tiled.first().map(|w| w.id));

                if let Some(fid) = focus_target {
                    sway_cmd(&format!("[con_id={fid}] focus")).await.ok();
                    self.verify_focus(fid).await;
                }
            }
        } else if snap.ws_layout == "tabbed" {
            if snap.any_focused {
                if let Some(fid) = hint {
                    sway_cmd(&format!("[con_id={fid}] focus")).await.ok();
                    self.verify_focus(fid).await;
                }
            }
        }

        Ok(())
    }

    // ── Post-command verification ──────────────────────────────

    async fn verify_focus(&self, expected_id: i64) {
        for _ in 0..2 {
            sleep(Duration::from_millis(2)).await;
            let snap = match snapshot().await {
                Some(s) => s,
                None => continue,
            };
            if !snap.any_focused {
                return;
            }
            let actual = snap
                .tiled
                .iter()
                .find(|w| w.focused)
                .map(|w| w.id);
            if actual == Some(expected_id) {
                return;
            }
            sway_cmd(&format!("[con_id={expected_id}] focus"))
                .await
                .ok();
        }
    }
}
