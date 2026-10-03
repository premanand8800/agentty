//! agentty: a lightweight terminal for running and supervising AI coding agents.

mod app;
mod boxdraw;
mod config;
mod ctl;
mod font;
mod input;
mod render;
mod session;
mod snapshot;
mod theme;
#[cfg(test)]
mod tests;

use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::json;
use winit::event_loop::EventLoop;

const HELP: &str = "agentty: a lightweight terminal for AI coding agents

usage:
  agentty [--profile NAME]          open the window (first tab: NAME, default: your shell)
  agentty ctl <command>             control a running agentty (see `agentty ctl help`)
  agentty snapshot --out FILE.png [--theme NAME] [--cols N] [--rows N] [--scale F] [--wait-ms MS] -- <command...>
  agentty themes | profiles | --version

shortcuts:
  Ctrl+Shift+T new tab        Ctrl+Shift+1..9 open profile N     Ctrl+Shift+W close tab
  Ctrl+Tab / Alt+1..9 switch  Ctrl+Shift+C / V copy / paste       Ctrl+Shift+P next theme
  Ctrl+Shift+= / - / 0 font   Shift+PageUp/PageDown scrollback

config: ~/.config/agentty/config.toml";

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("agentty: config error: {e}");
            std::process::exit(2);
        }
    };
    match args.first().map(String::as_str) {
        Some("-h" | "--help" | "help") => println!("{HELP}"),
        Some("-V" | "--version") => println!("agentty {}", env!("CARGO_PKG_VERSION")),
        Some("themes") => {
            for t in theme::THEMES.iter() {
                println!("{}{}", t.name, if t.name.eq_ignore_ascii_case(&cfg.theme) { "  (current)" } else { "" });
            }
        }
        Some("profiles") => {
            for (i, p) in cfg.profiles.iter().enumerate() {
                println!("Ctrl+Shift+{}  {:<12} {}", i + 1, p.name, p.command.join(" "));
            }
        }
        Some("ctl") => std::process::exit(ctl::client(&args[1..])),
        Some("snapshot") => {
            let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
            let (opts, cmd) = (&args[..split], args.get(split + 1..).unwrap_or_default());
            let tabs = arg_value(opts, "--tabs")
                .map(|v| {
                    v.split(',')
                        .filter_map(|t| {
                            let (name, st) = t.split_once(':')?;
                            let st = match st {
                                "working" => session::Status::Working,
                                "needs" => session::Status::NeedsYou,
                                "idle" => session::Status::Idle,
                                _ => session::Status::Exited(0),
                            };
                            Some((name.to_string(), st))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let o = snapshot::Options {
                out: arg_value(opts, "--out").unwrap_or_else(|| "agentty.png".into()),
                theme: arg_value(opts, "--theme"),
                cols: arg_value(opts, "--cols").and_then(|v| v.parse().ok()).unwrap_or(90),
                rows: arg_value(opts, "--rows").and_then(|v| v.parse().ok()).unwrap_or(26),
                scale: arg_value(opts, "--scale").and_then(|v| v.parse().ok()).unwrap_or(1.0),
                wait_ms: arg_value(opts, "--wait-ms").and_then(|v| v.parse().ok()).unwrap_or(800),
                max_ms: arg_value(opts, "--max-ms").and_then(|v| v.parse().ok()).unwrap_or(15_000),
                title: arg_value(opts, "--title"),
                tabs,
                command: if cmd.is_empty() { vec![std::env::var("SHELL").unwrap_or("/bin/sh".into())] } else { cmd.to_vec() },
            };
            if let Err(e) = snapshot::run(&cfg, o) {
                eprintln!("agentty: {e}");
                std::process::exit(1);
            }
        }
        Some("--profile") | None => {
            let profile = arg_value(&args, "--profile");
            if let Err(e) = run_window(cfg, profile) {
                eprintln!("agentty: {e}");
                std::process::exit(1);
            }
        }
        Some(other) => {
            eprintln!("agentty: unknown command '{other}'\n\n{HELP}");
            std::process::exit(2);
        }
    }
}

fn run_window(cfg: config::Config, profile: Option<String>) -> Result<(), String> {
    let event_loop = EventLoop::<app::UserEvent>::with_user_event().build().map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let ctl_proxy = std::sync::Mutex::new(proxy.clone());
    let dispatch: ctl::Dispatch = Arc::new(move |req| {
        let (tx, rx) = mpsc::channel();
        if ctl_proxy.lock().unwrap().send_event(app::UserEvent::Ctl(req, tx)).is_err() {
            return json!({"ok": false, "error": "agentty is shutting down"});
        }
        rx.recv_timeout(Duration::from_secs(10)).unwrap_or(json!({"ok": false, "error": "timeout"}))
    });
    let socket = match ctl::serve(dispatch) {
        Ok(path) => Some(path),
        Err(e) => {
            eprintln!("agentty: control API disabled: {e}");
            None
        }
    };
    let mut app = app::App::new(cfg, proxy, socket.clone(), profile)?;
    let result = event_loop.run_app(&mut app).map_err(|e| e.to_string());
    if let Some(p) = socket {
        let _ = std::fs::remove_file(p);
    }
    result
}
