# sway-monocle-layout

A lightweight Rust daemon that gives Sway a monocle layout with
automatic tabbed fallback:

- **1 window** → borderless, fills the workspace (transparency preserved)
- **2+ windows** → tabbed layout, new windows auto-focused
- **User changes layout** → daemon backs off until you switch back

Also includes an independent **focus-back** mode that restores focus
to the window you were using when a launched application closes.

---

## Table of Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Building](#building)
- [Usage](#usage)
- [Sway Configuration](#sway-configuration)
- [Move-to-Monitor Script](#move-to-monitor-script)
- [Behavior Reference](#behavior-reference)
- [FAQ](#faq)
- [Project Structure](#project-structure)
- [License](#license)

---

## Features

### Monitor Mode

- **Single window**: removes borders and tab bar — the window fills
  the entire workspace while keeping compositor effects (transparency,
  blur, rounded corners). No fullscreen — apps stay in their normal
  compositing pipeline.

- **Multiple windows**: switches to tabbed layout, focuses the newest
  window automatically.

- **Respects user choice**: if you manually change the layout away from
  tabbed (e.g., to splith, splitv, or stacking), the daemon stops
  managing that workspace. Management resumes when you switch back to
  tabbed or the window count drops to one.

- **Never steals focus**: if you are working on another output, the
  daemon will not pull keyboard focus to the managed output.

- **Border preservation**: saves each window's original border style
  (`normal`, `pixel`, `none`, `csd` + width) before modifying it.
  Restores it exactly when transitioning to tabbed, when a floating
  dialog appears, or when the window leaves the managed output.

- **Clean window departure**: when a window moves away from the managed
  output (via keybind, script, or drag), the daemon automatically
  restores its original border and removes management marks.

- **Floating fullscreen awareness**: when a floating fullscreen app
  (like mpv) appears, the daemon backs off. When it closes, focus
  returns to the window you were using before — even if that window
  is on a different output.

- **User-fullscreen safe**: if you press F11 (or equivalent) on a
  managed window, the daemon does not interfere. If a second window
  opens while the first is user-fullscreened, the daemon correctly
  disables fullscreen before transitioning to tabbed.

- **Graceful instance replacement**: starting a new instance
  automatically terminates the previous one via PID file — no
  `pkill` needed.

### Focus-Back Mode

An independent mode that tracks window focus globally across all
outputs.

- **Core behavior**: when a window closes and it was the focused
  window, focus is restored to the window that was focused when it
  was created.

- **Launchers are transparent**: wofi, rofi, dmenu, bemenu, fuzzel,
  tofi, kickoff, swaynag, wlogout, nwg-drawer, ulauncher, and albert
  are excluded from the focus chain by default. When an excluded app
  is used to open another app, closing that app returns focus to what
  you had before the launcher, not to the launcher itself.

- **Parent-chain walking**: if the parent window is also gone, the
  engine walks up the chain until it finds a living ancestor on a
  visible workspace.

- **No focus stealing**: only triggers when the focused window closes.
  Background closes (unfocused windows closing) are completely ignored.

- **Runs independently**: separate PID file from monitor mode. Can be
  used standalone or alongside any number of monitor mode instances.

---

## How It Works

### Monitor Mode

The daemon subscribes to sway window and workspace events via IPC.
On each relevant event:

1. Reads the container tree for the target output
2. Counts tiled and floating windows on the visible workspace
3. Checks for state changes (skips if nothing changed)
4. Applies the appropriate policy:

| Tiled | Floats | Condition             | Action                                    |
|-------|--------|-----------------------|-------------------------------------------|
| 0     | any    | —                     | No-op                                     |
| 1     | 0      | not managed yet       | `border none` + `layout splith` + mark    |
| 1     | 0      | managed, layout drift | Reset to `layout splith`                  |
| 1     | >0     | managed               | Restore border (dialog needs context)     |
| 1     | —      | user fullscreened     | Back off                                  |
| 2+    | —      | `_auto_fs` present    | Transition: restore → tabbed → focus new  |
| 2+    | —      | tabbed, no mark       | Focus new arrivals only                   |
| 2+    | —      | not tabbed, no mark   | Back off (user changed layout)            |

### Focus-Back Mode

Processes every window event individually (no debouncing — order
matters for correct tracking):

- **`new` event**: records `parent[new_window] = last_focused_normal`
- **`focus` event**: updates tracking; excluded apps do not update the
  "normal" tracker, making them transparent
- **`close` event**: if the closed window was focused, walks the parent
  chain to find a living ancestor on a visible workspace and focuses it

---

## Building

### Dependencies

- Rust toolchain (1.70+)
- sway (tested with 1.8+)
- jq (only for the move-to-monitor helper script)

### Compile

    git clone https://github.com/menkaur/sway-monocle-layout.git
    cd sway-monocle-layout
    cargo build --release

The binary is at `target/release/sway-monocle-layout`.

### Install

    cp target/release/sway-monocle-layout ~/.local/bin/
    cp move-to-monitor.sh ~/.local/bin/
    chmod +x ~/.local/bin/move-to-monitor.sh

---

## Usage

### Monitor Mode

Manage a single output:

    sway-monocle-layout <OUTPUT>

Where `<OUTPUT>` is the sway output name (e.g., `DP-2`, `HDMI-A-1`).
Find your output names with:

    swaymsg -t get_outputs | jq -r '.[].name'

Options:

| Option              | Description                                                            |
|---------------------|------------------------------------------------------------------------|
| `--pidfile <PATH>`  | Override PID file location (default: `/tmp/sway-monocle-<output>.pid`) |
| `-h`, `--help`      | Show help                                                              |

Examples:

    # Manage DP-2
    sway-monocle-layout DP-2

    # Manage HDMI-A-1 with custom PID file
    sway-monocle-layout HDMI-A-1 --pidfile /run/user/1000/hdmi.pid

    # Manage multiple outputs (each is independent)
    sway-monocle-layout DP-1 &
    sway-monocle-layout DP-2 &

### Focus-Back Mode

Track focus globally and restore it when launched applications close:

    sway-monocle-layout --focus-back

Options:

| Option              | Description                                                     |
|---------------------|-----------------------------------------------------------------|
| `--exclude <APPS>`  | Comma-separated app_ids to add to the built-in exclusion list   |
| `--pidfile <PATH>`  | Override PID file (default: `/tmp/sway-monocle-focus-back.pid`) |

Examples:

    # Basic usage
    sway-monocle-layout --focus-back

    # Add custom exclusions
    sway-monocle-layout --focus-back --exclude myapp,otherapp

    # Custom PID file
    sway-monocle-layout --focus-back --pidfile /run/user/1000/fb.pid

### Running Both Together

Monitor mode and focus-back mode are fully independent — they use
separate PID files and do not interfere with each other:

    sway-monocle-layout DP-2 &
    sway-monocle-layout --focus-back &

Starting a new instance of the same mode gracefully replaces the
previous one. Starting a different mode has no effect on running
instances.

---

## Sway Configuration

Add to your `~/.config/sway/config`:

    # Monocle layout management on DP-2
    exec_always sway-monocle-layout DP-2

    # Focus-back (optional, independent)
    exec_always sway-monocle-layout --focus-back

    # Move window to other monitor (adjust keybinds to taste)
    bindsym $mod+bracketleft  exec move-to-monitor.sh DP-1
    bindsym $mod+bracketright exec move-to-monitor.sh DP-2

`exec_always` ensures the daemon restarts on sway config reload.
The new instance automatically kills the previous one via the PID
file — no `pkill` needed.

### Recommended Companion Settings

    # Remove gaps on managed output for true edge-to-edge single window
    workspace 5 gaps inner 0
    workspace 5 gaps outer 0

    # Hide bar on managed output (optional)
    bar {
        output DP-2
        mode hide
    }

    # Default border style (the daemon saves and restores whatever you set)
    default_border pixel 2

### Multi-Output Example

    # Each output managed independently
    exec_always sway-monocle-layout DP-1
    exec_always sway-monocle-layout DP-2
    exec_always sway-monocle-layout HDMI-A-1

    # Focus-back is global (only one instance needed)
    exec_always sway-monocle-layout --focus-back

---

## Move-to-Monitor Script

A minimal helper script for moving windows between outputs. The daemon
handles all cleanup (border restoration, mark removal) automatically
when a window departs the managed output.

### move-to-monitor.sh

    #!/bin/bash
    TARGET="$1"
    [ -z "$TARGET" ] && exit 1

    # Get target output geometry for cursor warp
    eval $(swaymsg -t get_outputs | jq -r \
      ".[] | select(.name == \"$TARGET\") | \
       \"OX=\(.rect.x) OY=\(.rect.y) OW=\(.rect.width) OH=\(.rect.height)\"")
    [ -z "$OW" ] && exit 1

    CX=$((OX + OW / 2))
    CY=$((OY + OH / 2))

    # Capture the con_id of the focused window BEFORE the move
    CON_ID=$(swaymsg -t get_tree | jq -r \
      '.. | select(.focused? == true and .pid? > 0) | .id' | head -1)
    [ -z "$CON_ID" ] && exit 1

    # Move, explicitly focus on target output, warp cursor
    swaymsg "move container to output $TARGET; \
             [con_id=$CON_ID] focus; \
             seat seat0 cursor set $CX $CY"

### What It Does

1. Looks up the target output geometry
2. Captures the focused window container ID
3. Moves the container, focuses it on the target, warps the cursor

### What the Daemon Handles Automatically

When a window leaves the managed output, the daemon detects the
departure and:

- Restores the original border style of the window
- Removes the `_auto_fs` management mark
- No action needed from the script or the user

---

## Behavior Reference

### Single Window on Managed Output

| Event                        | Daemon Action                                  |
|------------------------------|------------------------------------------------|
| Window opens                 | `border none` + `layout splith` + mark         |
| Floating dialog appears      | Restore original border, remove mark           |
| Dialog closes                | Re-apply `border none` + mark                  |
| User fullscreens (F11)       | Daemon backs off                               |
| Window moves to other output | Restore border, remove mark                    |

### Multiple Windows on Managed Output

| Event                        | Daemon Action                                  |
|------------------------------|------------------------------------------------|
| 2nd window opens (1→2)      | Restore 1st border, `layout tabbed`, focus 2nd |
| 3rd+ window opens            | Focus the new window (already tabbed)          |
| Window closes (2→1)         | Remaining window: `border none` + `splith`     |
| User changes layout          | Daemon backs off completely                    |
| User changes back to tabbed  | Daemon resumes (focuses new arrivals)          |
| User-fs + new window         | Disable fs, restore, tabbed, focus new         |

### Cross-Output Focus (mpv Example)

| Event                        | Daemon Action                                  |
|------------------------------|------------------------------------------------|
| mpv opens fullscreen-float   | Save globally focused window (e.g., terminal)  |
| mpv closes                   | Focus saved window if still on visible ws      |
| Saved window closed          | Sway default focus takes over                  |
| Saved window on hidden ws    | No forced focus                                |

### Focus-Back (Independent Mode)

| Event                        | Engine Action                                  |
|------------------------------|------------------------------------------------|
| New window opens             | Record parent = last non-excluded focused win  |
| Window closes (was focused)  | Walk parent chain, focus first living ancestor |
| Window closes (not focused)  | Ignored — never steals focus                   |
| Excluded app (wofi) closes   | Parent chain skips it transparently            |

Example chain:

    terminal → wofi → firefox → (close firefox) → terminal

Even though wofi had focus between terminal and firefox,
focus returns to terminal, not wofi.

---

## FAQ

### Does this slow down sway or add input latency?

No. The daemon is a separate process that communicates via Unix socket
IPC. It has no access to the sway input or rendering pipeline. Each IPC
call takes less than 0.5ms. The daemon issues zero commands when idle
(state-change detection skips unchanged states). Stability polling uses
adaptive backoff to minimize IPC pressure.

### What happens if the daemon crashes?

Windows keep whatever state they had. On restart, the daemon re-reads
the tree and applies the correct policy. The only artifact is that
windows may retain the `_auto_fs` mark and `border none` from the
previous session — the daemon will adopt them on the next relevant
event.

### Can I use this with gaps?

Yes. `border none` does not affect gaps. If you want true edge-to-edge
for single windows, set gaps to 0 on the managed workspaces.

### Does this work with XWayland apps?

Yes. The daemon operates on the sway container tree, which abstracts
over the client protocol. Focus-back mode reads both `app_id`
(Wayland) and `window_properties.class` (X11) for exclusion matching.

### What if I do not want focus-back?

Only run monitor mode:

    exec_always sway-monocle-layout DP-2

Focus-back is entirely optional and independent.

### What if I only want focus-back?

Only run focus-back mode:

    exec_always sway-monocle-layout --focus-back

No monitor management will occur.

### Can I manage multiple outputs?

Yes. Run one monitor-mode instance per output:

    exec_always sway-monocle-layout DP-1
    exec_always sway-monocle-layout DP-2

Each instance manages exactly one output with its own PID file.

### Does the move-to-monitor script work without the daemon?

Yes. The script moves, focuses, and warps the cursor. Without the
daemon, the window keeps whatever border state it has. No errors.

---

## Project Structure

    src/
    ├── main.rs          Entry point, CLI parsing, mode routing
    ├── config.rs        Constants and runtime configuration
    ├── ipc.rs           Sway IPC transport (Unix socket)
    ├── tree.rs          Data types and JSON tree parsing
    ├── snapshot.rs      State acquisition and stability verification
    ├── events.rs        Event subscription and extraction
    ├── pid.rs           PID file management, single-instance enforcement
    ├── policy.rs        Monitor mode decision engine
    └── focus_back.rs    Focus-back engine

    scripts/
    └── move-to-monitor.sh   Helper for moving windows between outputs

---

## License

MIT
