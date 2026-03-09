// ═══════════════════════════════════════════════════════════════
// FILE: src/snapshot.rs
// ROLE: Reads the current state of the target output from sway's
//       tree, with optional stability verification.
//
// LLM CONTEXT:
//   FUNCTIONS:
//     • snapshot() → Option<Snapshot>
//       Single-read from one get_tree call.
//
//     • snapshot_stable(expected_id) → Option<Snapshot>
//       Polls snapshot() until tiled count is stable on two
//       consecutive reads.  If expected_id provided, also waits
//       for that window to appear.
//
//       v3.5 CHANGE: adaptive backoff.  First 10 iterations at
//       STABLE_INTERVAL (1ms) for fast convergence in the typical
//       case (2-3 iterations).  After 10 iterations, backs off to
//       5ms to reduce IPC pressure on sway in pathological cases
//       (many windows opening simultaneously).
//
//     • is_on_visible_workspace(target_id) → bool
//       One get_tree call.  Checks if a con_id exists on any
//       visible workspace across all outputs.
//
// DEPENDENCIES:
//   • src/config.rs: config::target_output(), STABLE_INTERVAL,
//     STABLE_MAX_TRIES
//   • src/ipc.rs: sway_tree()
//   • src/tree.rs: Snapshot, collect_tiled, count_floating,
//     has_focused_descendant, find_focused_window, contains_con_id
// ═══════════════════════════════════════════════════════════════

use tokio::time::{sleep, Duration};

use crate::config::{self, STABLE_INTERVAL, STABLE_MAX_TRIES};
use crate::ipc::sway_tree;
use crate::tree::{
    collect_tiled, contains_con_id, count_floating,
    find_focused_window, has_focused_descendant, Snapshot,
};

pub async fn snapshot() -> Option<Snapshot> {
    let tree = sway_tree().await.ok()?;
    let target = config::target_output();

    let global_focused = find_focused_window(&tree);

    let output = tree
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|n| {
            n.get("name").and_then(|v| v.as_str()) == Some(target)
        })?;

    let focus_id = output
        .get("focus")?
        .as_array()?
        .first()?
        .as_i64()?;

    let ws = output
        .get("nodes")?
        .as_array()?
        .iter()
        .find(|n| {
            n.get("id").and_then(|v| v.as_i64()) == Some(focus_id)
                && n.get("type").and_then(|v| v.as_str()) == Some("workspace")
        })?;

    let ws_name = ws.get("name")?.as_str()?.to_owned();

    let mut tiled = Vec::new();
    for c in ws
        .get("nodes")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        collect_tiled(c, &mut tiled);
    }

    let (fn_, ffs) = count_floating(ws);

    let layout = ws
        .get("layout")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_owned();

    let any_focused = has_focused_descendant(ws);

    Some(Snapshot {
        ws_name,
        ws_layout: layout,
        tiled,
        float_n: fn_,
        float_fs: ffs,
        any_focused,
        global_focused,
    })
}

pub async fn snapshot_stable(expected_id: Option<i64>) -> Option<Snapshot> {
    let mut prev_count: Option<usize> = None;
    let mut last_good: Option<Snapshot> = None;
    let hint_patience: usize = 10;

    for i in 0..STABLE_MAX_TRIES {
        let snap = match snapshot().await {
            Some(s) => s,
            None => {
                sleep(STABLE_INTERVAL).await;
                continue;
            }
        };

        let count = snap.tiled.len();
        let count_stable = prev_count == Some(count);

        match expected_id {
            Some(eid) => {
                let hint_found = snap.tiled.iter().any(|w| w.id == eid);

                if hint_found && count_stable {
                    return Some(snap);
                }

                if !hint_found && count_stable && i >= hint_patience {
                    return Some(snap);
                }
            }
            None => {
                if count_stable {
                    return Some(snap);
                }
            }
        }

        prev_count = Some(count);
        last_good = Some(snap);

        // Adaptive backoff:
        //   First 10 iterations: STABLE_INTERVAL (1ms)
        //     → Fast convergence for the typical case (2-3 reads)
        //   After 10 iterations: 5ms
        //     → Reduces IPC pressure on sway's event loop in
        //       pathological cases (many windows opening at once)
        //
        // Typical case: converges in 2-3ms (2-3 iterations × 1ms)
        // Worst case ceiling: 10ms + 90×5ms = 460ms with 80%
        //   less IPC pressure after the initial fast burst
        if i < 10 {
            sleep(STABLE_INTERVAL).await;
        } else {
            sleep(Duration::from_millis(5)).await;
        }
    }

    last_good
}

/// Check if a window with the given con_id exists on any currently
/// visible workspace across all outputs.
///
/// Makes one get_tree IPC call.
pub async fn is_on_visible_workspace(target_id: i64) -> bool {
    let tree = match sway_tree().await {
        Ok(t) => t,
        Err(_) => return false,
    };

    let outputs = match tree.get("nodes").and_then(|x| x.as_array()) {
        Some(o) => o,
        None => return false,
    };

    for output in outputs {
        let name = output
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name.starts_with("__") {
            continue;
        }

        let focus_id = match output
            .get("focus")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_i64())
        {
            Some(id) => id,
            None => continue,
        };

        let ws = match output
            .get("nodes")
            .and_then(|x| x.as_array())
            .and_then(|nodes| {
                nodes.iter().find(|n| {
                    n.get("id").and_then(|v| v.as_i64()) == Some(focus_id)
                        && n.get("type").and_then(|v| v.as_str())
                            == Some("workspace")
                })
            }) {
            Some(ws) => ws,
            None => continue,
        };

        if contains_con_id(ws, target_id) {
            return true;
        }
    }

    false
}
