// ═══════════════════════════════════════════════════════════════
// FILE: src/pid.rs
// ROLE: PID file management and single-instance enforcement.
//
// LLM CONTEXT:
//   Both functions take the PID file path as a parameter.
//   This allows different modes (monitor, focus-back) to use
//   separate PID files and run simultaneously.
//
//   enforce_single_instance(pidfile):
//     Kills any existing instance, writes our PID.
//
//   cleanup_pidfile(pidfile):
//     Removes the PID file only if it contains our PID.
//
// DEPENDENCIES:
//   • libc crate: kill() syscall
// ═══════════════════════════════════════════════════════════════

use std::fs;
use std::process;
use std::time::{Duration, Instant};

fn process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn send_signal(pid: u32, sig: i32) {
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

pub fn enforce_single_instance(pidfile: &str) {
    if let Ok(contents) = fs::read_to_string(pidfile) {
        if let Ok(old_pid) = contents.trim().parse::<u32>() {
            let our_pid = process::id();
            if old_pid != our_pid && process_alive(old_pid) {
                eprintln!(
                    "[smart-borders] sending SIGTERM to previous instance (pid {old_pid})"
                );
                send_signal(old_pid, libc::SIGTERM);

                let deadline = Instant::now() + Duration::from_secs(2);
                let poll_interval = Duration::from_millis(50);

                while process_alive(old_pid) {
                    if Instant::now() >= deadline {
                        eprintln!(
                            "[smart-borders] previous instance (pid {old_pid}) \
                             did not exit after 2s, sending SIGKILL"
                        );
                        send_signal(old_pid, libc::SIGKILL);
                        std::thread::sleep(Duration::from_millis(100));
                        break;
                    }
                    std::thread::sleep(poll_interval);
                }

                if !process_alive(old_pid) {
                    eprintln!(
                        "[smart-borders] previous instance (pid {old_pid}) exited"
                    );
                }
            }
        }
    }
    fs::write(pidfile, format!("{}", process::id())).ok();
}

pub fn cleanup_pidfile(pidfile: &str) {
    if let Ok(contents) = fs::read_to_string(pidfile) {
        if let Ok(pid) = contents.trim().parse::<u32>() {
            if pid == process::id() {
                fs::remove_file(pidfile).ok();
            }
        }
    }
}
