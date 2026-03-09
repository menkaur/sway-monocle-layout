mod config;
mod events;
mod focus_back;
mod ipc;
mod pid;
mod policy;
mod snapshot;
mod tree;

use std::env;
use std::process;

use config::DEBOUNCE;
use events::{extract_hint, read_event, subscribe_events, HintType};
use pid::{cleanup_pidfile, enforce_single_instance};
use policy::Policy;

use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{sleep, timeout, Duration};

// ── CLI Parsing ────────────────────────────────────────────────

enum Mode {
    Monitor {
        output: String,
        pidfile: Option<String>,
    },
    FocusBack {
        excludes: Vec<String>,
        pidfile: Option<String>,
    },
}

fn print_usage() -> ! {
    let bin = env::args()
        .next()
        .unwrap_or_else(|| "sway-monocle-layout".to_string());
    eprintln!(
        "Usage:\n\
         \n\
         MONITOR MODE:\n\
           {bin} <OUTPUT> [--pidfile <PATH>]\n\
         \n\
           Manages a sway output: 1 window = borderless monocle,\n\
           2+ windows = tabbed layout.\n\
         \n\
         FOCUS-BACK MODE:\n\
           {bin} --focus-back [--exclude <APPS>] [--pidfile <PATH>]\n\
         \n\
           Tracks focus globally. When a window closes, restores\n\
           focus to the window that was focused when it was created.\n\
         \n\
         Options:\n\
           --pidfile <PATH>      Override PID file location\n\
           --exclude <APPS>      Comma-separated app_ids to exclude\n\
                                 (added to built-in defaults)\n\
           -h, --help            Show this help message\n\
         \n\
         Examples:\n\
           {bin} DP-2\n\
           {bin} HDMI-A-1 --pidfile /run/user/1000/dp2.pid\n\
           {bin} --focus-back\n\
           {bin} --focus-back --exclude myapp,otherapp\n\
         \n\
         Both modes can run simultaneously."
    );
    process::exit(0);
}

fn parse_args() -> Mode {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        eprintln!("Error: missing arguments\n");
        print_usage();
    }

    let mut focus_back = false;
    let mut output: Option<String> = None;
    let mut pidfile: Option<String> = None;
    let mut excludes: Vec<String> = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => print_usage(),
            "--focus-back" => {
                focus_back = true;
            }
            "--pidfile" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --pidfile requires a value\n");
                    print_usage();
                }
                pidfile = Some(args[i].clone());
            }
            "--exclude" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --exclude requires a value\n");
                    print_usage();
                }
                excludes.extend(
                    args[i]
                        .split(',')
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| !s.is_empty()),
                );
            }
            arg if arg.starts_with('-') => {
                eprintln!("Error: unknown option '{arg}'\n");
                print_usage();
            }
            _ => {
                if output.is_some() {
                    eprintln!("Error: unexpected argument '{}'\n", args[i]);
                    print_usage();
                }
                output = Some(args[i].clone());
            }
        }
        i += 1;
    }

    if focus_back {
        if output.is_some() {
            eprintln!("Error: --focus-back and <OUTPUT> are mutually exclusive\n");
            print_usage();
        }
        Mode::FocusBack { excludes, pidfile }
    } else {
        let output = output.unwrap_or_else(|| {
            eprintln!("Error: missing required argument <OUTPUT>\n");
            print_usage();
        });
        Mode::Monitor { output, pidfile }
    }
}

// ── Main ───────────────────────────────────────────────────────

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mode = parse_args();

    match mode {
        Mode::FocusBack { excludes, pidfile } => {
            let pidfile = pidfile.unwrap_or_else(|| "/tmp/sway-monocle-focus-back.pid".to_string());
            focus_back::run(excludes, pidfile).await;
        }
        Mode::Monitor { output, pidfile } => {
            run_monitor(output, pidfile).await;
        }
    }
}

// ── Monitor mode ───────────────────────────────────────────────

async fn run_monitor(output: String, pidfile: Option<String>) {
    config::init(output, pidfile);
    enforce_single_instance(config::pidfile());

    let our_pid = std::process::id();
    eprintln!(
        "[monocle] pid {our_pid} managing output '{}', pidfile '{}'",
        config::target_output(),
        config::pidfile()
    );

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to register SIGTERM");
    let mut sigint = signal(SignalKind::interrupt()).expect("failed to register SIGINT");

    sleep(Duration::from_millis(500)).await;

    let mut policy = Policy::new();

    'outer: loop {
        policy.apply(None).await;

        let mut stream = match subscribe_events().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[monocle] subscription failed: {e}, retrying in 1s");
                sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        loop {
            let event = tokio::select! {
                _ = sigterm.recv() => {
                    eprintln!(
                        "[monocle] pid {our_pid} received SIGTERM"
                    );
                    break 'outer;
                }
                _ = sigint.recv() => {
                    eprintln!(
                        "[monocle] pid {our_pid} received SIGINT"
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

            let mut hint: Option<i64> = None;
            let mut hint_is_new = false;

            if let Some((id, ht)) = extract_hint(&event) {
                hint = Some(id);
                hint_is_new = ht == HintType::New;
            }

            let mut disconnected = false;

            loop {
                match timeout(DEBOUNCE, read_event(&mut stream)).await {
                    Ok(Ok(next)) => {
                        if let Some((id, ht)) = extract_hint(&next) {
                            let is_new = ht == HintType::New;
                            if is_new || !hint_is_new {
                                hint = Some(id);
                                hint_is_new = is_new;
                            }
                        }
                    }
                    Ok(Err(_)) => {
                        disconnected = true;
                        break;
                    }
                    Err(_) => break,
                }
            }

            policy.apply(hint).await;

            if disconnected {
                break;
            }
        }

        eprintln!("[monocle] disconnected from sway, reconnecting in 1s");
        sleep(Duration::from_secs(1)).await;
    }

    cleanup_pidfile(config::pidfile());
    eprintln!("[monocle] pid {our_pid} shutdown complete");
}
