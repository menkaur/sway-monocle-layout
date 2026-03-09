# smart-borders — Auto-Tabbed Layout Manager for Sway

smart-borders is a lightweight Rust daemon that enhances the Sway window manager
with intelligent single-monitor window management and an optional global
focus-restoration engine.

With smart-borders, Sway behaves more like a polished tiling WM with
smart fullscreen, automatic tabbing, border cleanup, and smart focus.

---

## Features

### Monitor Mode (per-output management)

On a selected output (e.g., DP-2):

**1 window -> fullscreen-like, but with transparency preserved**

- `border none`
- `layout splith` (removes sway tab/stack bars)
- fills entire workspace
- compositor effects remain (blur, transparency)

**2+ windows -> automatic tabbed layout**

- restores original borders of all windows
- switches workspace to `layout tabbed`
- focuses new windows as they appear

**Respects user behavior**

- If you switch layout away from tabbed, daemon backs off
  until layout returns to tabbed or window count becomes 1.

**Never steals focus**

- If you are working on another output, smart-borders never pulls focus.

**Border save/restore**

- Before manipulating a window, its full border configuration is saved
  (`normal`, `pixel`, `none`, `csd`, width).
- When transitioning to tabbed or when the window moves to another output,
  the original border is restored exactly.

**Departed window cleanup**

- When a window leaves the managed output (e.g., moved by script or keybind),
  smart-borders automatically restores its border and removes management marks.

**mpv / fullscreen float awareness**

- When a floating fullscreen app (like mpv) appears:
  - daemon temporarily backs off
  - tracks the window you were using BEFORE mpv
- When mpv closes:
  - focus returns to that earlier window
    (even if it is on a different output)

---

### Focus-Back Mode (global)

An independent mode that tracks window focus globally across all displays.

When a window closes, smart-borders automatically restores focus to
the window that launched it (terminal -> mpv -> close -> return focus to terminal).

**Launchers are skipped** (transparent):

- wofi, rofi, dmenu, bemenu, fuzzel, tofi, kickoff
- swaynag, wlogout, nwg-drawer, ulauncher, albert

When a launcher is used to open an app, closing that app returns focus to the
window that existed before the launcher opened.

Also includes:

- parent-chain walking (terminal -> app1 -> app2 -> if app1 dead, go to terminal)
- no focus stealing (only triggers when the focused window closes)

Monitor mode and focus-back mode can run together or independently.

---

## Building

Requires:

- Rust 1.70+
- sway 1.8+
- jq (only used by helper script, not by daemon)

Build:

    git clone https://github.com/menkaur/sway-monocle-layout.git
    cd smart-borders
    cargo build --release

Install:

    cp target/release/smart-borders-dp2 ~/.local/bin/
    cp move-to-monitor.sh ~/.local/bin/
    chmod +x ~/.local/bin/move-to-monitor.sh

---

## Usage

### Monitor Mode

Run for a specific output:

    smart-borders-dp2 <OUTPUT>

Examples:

    smart-borders-dp2 DP-2
    smart-borders-dp2 HDMI-A-1 --pidfile /run/user/1000/hdmi.pid

Find output names:

    swaymsg -t get_outputs | jq -r '.[].name'

Options:

    --pidfile <PATH>   Override PID file location
    -h, --help         Show help

### Focus-Back Mode

Global focus restoration:

    smart-borders-dp2 --focus-back

Optional exclusions:

    smart-borders-dp2 --focus-back --exclude myapp,otherapp

Options:

    --exclude <APPS>   Comma-separated app_ids to add to exclusion list
    --pidfile <PATH>   Override PID file

### Running Both Together

    smart-borders-dp2 DP-2 &
    smart-borders-dp2 --focus-back &

Each mode uses a separate PID file and will not kill the other.

---

## Sway Configuration

Add to ~/.config/sway/config:

    # Manage DP-2 with auto-tabbed layout
    exec_always smart-borders-dp2 DP-2

    # Global focus restoration
    exec_always smart-borders-dp2 --focus-back

    # Keybinds to move windows between monitors
    bindsym $mod+bracketleft  exec move-to-monitor.sh DP-1
    bindsym $mod+bracketright exec move-to-monitor.sh DP-2

### Optional enhancements

    # Hide bar on DP-2
    bar {
        output DP-2
        mode hide
    }

    # Edge-to-edge single-window look
    workspace * gaps inner 0
    workspace * gaps outer 0

    # Custom default border style
    default_border pixel 2

### Multi-Output Example

    exec_always smart-borders-dp2 DP-1
    exec_always smart-borders-dp2 DP-2
    exec_always smart-borders-dp2 HDMI-A-1
    exec_always smart-borders-dp2 --focus-back

---

## Move-to-Monitor Script

The companion script for moving windows between outputs. The daemon handles
all cleanup (border restore, mark removal) automatically when a window departs
the managed output.

### move-to-monitor.sh

    #!/bin/bash
    TARGET="$1"
    [ -z "$TARGET" ] && exit 1

    eval $(swaymsg -t get_outputs | jq -r \
      ".[] | select(.name == \"$TARGET\") | \
       \"OX=\(.rect.x) OY=\(.rect.y) OW=\(.rect.width) OH=\(.rect.height)\"")
    [ -z "$OW" ] && exit 1

    CX=$((OX + OW / 2))
    CY=$((OY + OH / 2))

    CON_ID=$(swaymsg -t get_tree | jq -r \
      '.. | select(.focused? == true and .pid? > 0) | .id' | head -1)
    [ -z "$CON_ID" ] && exit 1

    swaymsg "move container to output $TARGET; \
             [con_id=$CON_ID] focus; \
             seat seat0 cursor set $CX $CY"

### What the script does

1. Looks up the target output geometry
2. Captures the focused window container ID
3. Moves the container, focuses it on the target, warps the cursor

### What the daemon handles automatically

When a window leaves the managed output, the daemon detects the departure and:

- Restores the window's original border style
- Removes the _auto_fs management mark
- No action needed from the script or the user

---

## Detailed Behavior Reference

### Single Window on Managed Output

| Event                        | Daemon Action                                      |
|------------------------------|---------------------------------------------------|
| Window opens                 | border none + layout splith + mark _auto_fs       |
| Floating dialog appears      | Restore border, remove mark                        |
| Dialog closes                | Re-apply border none + mark                        |
| User fullscreens manually    | Daemon backs off (no _auto_fs on that window)      |

### Multiple Windows on Managed Output

| Event                        | Daemon Action                                      |
|------------------------------|---------------------------------------------------|
| 2nd window opens             | Restore border on 1st, layout tabbed, focus 2nd   |
| 3rd+ window opens            | Focus the new window (layout already tabbed)       |
| Window closes (2->1)         | Remaining window gets border none + layout splith  |
| User changes layout          | Daemon backs off completely                        |
| User changes back to tabbed  | Daemon resumes (focuses new arrivals)              |

### Cross-Output Focus (mpv Example)

| Event                        | Daemon Action                                      |
|------------------------------|---------------------------------------------------|
| mpv opens fullscreen-float   | Save globally focused window (e.g. terminal on DP-1)|
| mpv closes                   | Restore focus to saved window if still visible     |
| Saved window was closed      | Sway default focus behavior takes over             |
| Saved window on hidden ws    | Same - no forced focus                             |

### Focus-Back (Independent Mode)

| Event                        | Engine Action                                      |
|------------------------------|---------------------------------------------------|
| New window opens             | Record parent = last non-excluded focused window   |
| Window closes (was focused)  | Walk parent chain, focus first living ancestor     |
| Window closes (not focused)  | Ignored - never steals focus                       |
| Excluded app closes          | Parent chain skips it transparently                |

Example chain:

    terminal -> wofi -> firefox -> (close firefox) -> terminal

Even though wofi focused itself and then firefox,
focus returns to terminal, not wofi.

---

## FAQ

**Does this slow down sway or add input latency?**

No. The daemon is a separate process communicating via Unix socket IPC.
It has no access to sway input or rendering pipeline. Each IPC call
takes less than 0.5ms. Zero commands during idle states.

**Does it steal keyboard focus?**

Never, unless a deliberate transition occurs (new window in tabbed mode
on the managed output, mpv close restore, focus-back on close).

**What happens if the daemon crashes?**

Windows keep whatever state they had. On restart, the daemon re-reads the
tree and applies the correct policy. The only artifact is windows may
retain the _auto_fs mark and border none from the previous session.

**Can I use this with gaps?**

Yes. border none does not affect gaps. Set gaps to 0 on the managed
workspace for true edge-to-edge single windows.

**Does this work with XWayland apps?**

Yes. The daemon operates on sway container tree, which abstracts over
the client protocol. Focus-back mode reads both app_id (Wayland) and
window_properties.class (X11) for exclusion matching.

**Can I exclude an app from focus-back by window title?**

Not currently. Exclusion matches on app_id or window_properties.class.

**What if I only want the tabbed layout, no focus-back?**

    exec_always smart-borders-dp2 DP-2

Focus-back is entirely optional and independent.

**What if I only want focus-back, no layout management?**

    exec_always smart-borders-dp2 --focus-back

---

## Project Structure

    src/
    +-- main.rs          Entry point, CLI parsing, mode routing
    +-- config.rs        Constants and runtime configuration
    +-- ipc.rs           Sway IPC transport (Unix socket)
    +-- tree.rs          Data types and JSON tree parsing
    +-- snapshot.rs      State acquisition and stability verification
    +-- events.rs        Event subscription and extraction
    +-- pid.rs           PID file management, single-instance enforcement
    +-- policy.rs        Monitor mode decision engine
    +-- focus_back.rs    Focus-back engine

---

## License

MIT
