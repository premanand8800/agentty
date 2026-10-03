//! Agent control API: newline-delimited JSON over a Unix socket only the current user can open.
//!
//!   {"op":"list"}
//!   {"op":"open","profile":"Claude"}            or  {"op":"open","command":["bash","-lc","make test"],"cwd":"/repo"}
//!   {"op":"send","id":2,"text":"run the tests","enter":true}
//!   {"op":"read","id":2,"lines":200}             plain text, no escape codes
//!   {"op":"wait","id":2,"until":"settled","timeout_ms":120000}
//!   {"op":"status","id":2} / {"op":"focus","id":2} / {"op":"close","id":2} / {"op":"theme","name":"Ocean"}

use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(unix)]
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    os::unix::net::{UnixListener, UnixStream},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(not(unix), allow(dead_code))] // the socket server is Unix-only for now
pub enum Request {
    List,
    Open {
        profile: Option<String>,
        command: Option<Vec<String>>,
        cwd: Option<PathBuf>,
        title: Option<String>,
        #[serde(default = "yes")]
        focus: bool,
    },
    Send {
        id: u32,
        text: String,
        #[serde(default)]
        enter: bool,
        /// Wrap as a bracketed paste, so multi-line text arrives as one block.
        #[serde(default)]
        paste: bool,
    },
    Read {
        id: u32,
        #[serde(default = "default_lines")]
        lines: usize,
    },
    Status {
        id: u32,
    },
    Wait {
        id: u32,
        #[serde(default)]
        until: WaitFor,
        timeout_ms: Option<u64>,
    },
    Focus {
        id: u32,
    },
    Close {
        id: u32,
    },
    Theme {
        name: String,
    },
    /// Ambient background: off, starfield, rain, snow, fireflies, aurora.
    Background {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(not(unix), allow(dead_code))]
pub enum WaitFor {
    /// Output stopped and nothing is asked of you.
    Idle,
    /// The agent is waiting for input.
    Attention,
    /// The program exited.
    Exit,
    /// Any of the above: the tab is no longer busy.
    #[default]
    Settled,
}

fn yes() -> bool {
    true
}
fn default_lines() -> usize {
    200
}

#[cfg(unix)]
fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) => PathBuf::from(d).join("agentty"),
        None => {
            let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
            std::env::temp_dir().join(format!("agentty-{uid}"))
        }
    }
}

#[cfg(unix)]
/// Where `agentty ctl` connects. Inside a tab, `AGENTTY_SOCKET` names that tab's own window;
/// elsewhere `ctl.sock` points at the most recently opened window.
pub fn socket_path() -> PathBuf {
    match std::env::var_os("AGENTTY_SOCKET") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => runtime_dir().join("ctl.sock"),
    }
}

#[cfg(unix)]
fn is_live(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

#[cfg(unix)]
/// Point `ctl.sock` at `target` atomically (symlink + rename), so clients never see it missing.
fn point_default_at(dir: &Path, target: &Path) -> std::io::Result<()> {
    let tmp = dir.join(format!(".ctl-{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp)?;
    std::fs::rename(&tmp, dir.join("ctl.sock"))
}

#[cfg(unix)]
/// Remove this window's socket. If `ctl.sock` pointed here, repoint it at another live window.
pub fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
    let Some(dir) = path.parent() else { return };
    let link = dir.join("ctl.sock");
    if std::fs::read_link(&link).ok().as_deref() != Some(path) {
        return;
    }
    let other = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("ctl-") && n.ends_with(".sock"))
        })
        .find(|p| is_live(p));
    match other {
        Some(p) => {
            let _ = point_default_at(dir, &p);
        }
        None => {
            let _ = std::fs::remove_file(&link);
        }
    }
}

/// Runs one request on the UI thread and returns its JSON reply.
pub type Dispatch = Arc<dyn Fn(Request) -> Value + Send + Sync>;

#[cfg(unix)]
/// Start the server. Returns the socket path, or an error if another instance owns it.
pub fn serve(dispatch: Dispatch) -> Result<PathBuf, String> {
    // One socket per window, so tabs always control their own window, plus `ctl.sock` pointing
    // at the newest window for callers outside agentty.
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("ctl-") && name.ends_with(".sock") && !is_live(&p) {
            let _ = std::fs::remove_file(&p); // left behind by a crashed window
        }
    }
    let path = dir.join(format!("ctl-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    point_default_at(&dir, &path).map_err(|e| format!("{}: {e}", dir.join("ctl.sock").display()))?;
    std::thread::Builder::new()
        .name("agentty-ctl".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let dispatch = dispatch.clone();
                let _ =
                    std::thread::Builder::new().name("agentty-ctl-conn".into()).spawn(move || handle(stream, dispatch));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(path)
}

#[cfg(unix)]
fn handle(stream: UnixStream, dispatch: Dispatch) {
    let Ok(mut writer) = stream.try_clone() else { return };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Wait { id, until, timeout_ms }) => wait(&dispatch, id, until, timeout_ms),
            Ok(req) => dispatch(req),
            Err(e) => json!({"ok": false, "error": format!("bad request: {e}")}),
        };
        if writeln!(writer, "{reply}").is_err() {
            return;
        }
    }
}

#[cfg(unix)]
/// Poll the tab's status until the condition holds. Runs on the connection thread, not the UI.
fn wait(dispatch: &Dispatch, id: u32, until: WaitFor, timeout_ms: Option<u64>) -> Value {
    let start = Instant::now();
    let deadline = start + Duration::from_millis(timeout_ms.unwrap_or(300_000).min(3_600_000));
    let mut seen_working = false;
    loop {
        let st = dispatch(Request::Status { id });
        if st["ok"] != json!(true) {
            return st;
        }
        let status = st["status"].as_str().unwrap_or("");
        seen_working |= status == "working";
        // Right after `send`, the agent may not have printed anything yet. Only treat "quiet" as
        // done once it has been seen working, or after a short grace period.
        let quiet_counts = seen_working || start.elapsed() > Duration::from_secs(2);
        let done = match until {
            WaitFor::Idle => status == "idle" && quiet_counts,
            WaitFor::Attention => status == "needs you",
            WaitFor::Exit => status == "exited",
            WaitFor::Settled => status != "working" && (quiet_counts || status != "idle"),
        };
        if done {
            return json!({"ok": true, "waited": true, "status": status, "tab": st["tab"]});
        }
        if Instant::now() >= deadline {
            return json!({"ok": false, "error": "timeout", "status": status, "tab": st["tab"]});
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(unix)]
/// `agentty ctl ...`: build a request from CLI arguments, send it, print the reply.
pub fn client(args: &[String]) -> i32 {
    let usage = "usage: agentty ctl list | open <profile> | open -- <cmd...> [--cwd DIR] | send <id> <text...> [--no-enter] [--paste]\n\
                 \x20                | read <id> [--lines N] | status <id> | wait <id> [--until idle|attention|exit|settled] [--timeout SECS]\n\
                 \x20                | focus <id> | close <id> | theme <name> | background <name> | raw '<json>'";
    let req: Value = match build(args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}\n{usage}");
            return 2;
        }
    };
    let path = socket_path();
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot reach agentty at {}: {e}\nIs agentty running?", path.display());
            return 1;
        }
    };
    if writeln!(stream, "{req}").is_err() {
        eprintln!("write failed");
        return 1;
    }
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line).is_err() || line.is_empty() {
        eprintln!("no reply");
        return 1;
    }
    let reply: Value = serde_json::from_str(&line).unwrap_or(json!({"ok": false, "error": "invalid reply"}));
    if reply.get("text").is_some() && reply["ok"] == json!(true) && !args.iter().any(|a| a == "--json") {
        println!("{}", reply["text"].as_str().unwrap_or(""));
    } else {
        println!("{}", serde_json::to_string_pretty(&reply).unwrap_or_default());
    }
    if reply["ok"] == json!(true) {
        0
    } else {
        1
    }
}

#[cfg(not(unix))]
const NO_CTL: &str = "the control API is not available on Windows yet";

#[cfg(not(unix))]
pub fn serve(_dispatch: Dispatch) -> Result<PathBuf, String> {
    Err(NO_CTL.into())
}

#[cfg(not(unix))]
pub fn cleanup(_path: &Path) {}

#[cfg(not(unix))]
pub fn client(args: &[String]) -> i32 {
    let _ = build(args);
    eprintln!("agentty: {NO_CTL}");
    1
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn id_arg(args: &[String]) -> Result<u32, String> {
    args.get(1).and_then(|s| s.parse().ok()).ok_or_else(|| "missing or invalid tab id".to_string())
}

fn build(args: &[String]) -> Result<Value, String> {
    let op = args.first().map(String::as_str).ok_or("missing command")?;
    Ok(match op {
        "list" => json!({"op": "list"}),
        "raw" => serde_json::from_str(args.get(1).ok_or("missing json")?).map_err(|e| e.to_string())?,
        "open" => {
            let cwd = flag(args, "--cwd");
            if let Some(i) = args.iter().position(|a| a == "--") {
                let cmd: Vec<&String> = args[i + 1..].iter().collect();
                if cmd.is_empty() {
                    return Err("missing command after --".into());
                }
                json!({"op": "open", "command": cmd, "cwd": cwd})
            } else {
                json!({"op": "open", "profile": args.get(1).ok_or("missing profile")?, "cwd": cwd})
            }
        }
        "send" => {
            let id = id_arg(args)?;
            let words: Vec<&str> = args[2..].iter().map(String::as_str).filter(|a| !a.starts_with("--")).collect();
            json!({"op": "send", "id": id, "text": words.join(" "),
                   "enter": !args.iter().any(|a| a == "--no-enter"), "paste": args.iter().any(|a| a == "--paste")})
        }
        "read" => {
            let lines: usize = flag(args, "--lines").and_then(|s| s.parse().ok()).unwrap_or(200);
            json!({"op": "read", "id": id_arg(args)?, "lines": lines})
        }
        "wait" => {
            let timeout = flag(args, "--timeout").and_then(|s| s.parse::<u64>().ok()).map(|s| s * 1000);
            json!({"op": "wait", "id": id_arg(args)?, "until": flag(args, "--until").unwrap_or("settled".into()), "timeout_ms": timeout})
        }
        "status" | "focus" | "close" => json!({"op": op, "id": id_arg(args)?}),
        "theme" => json!({"op": "theme", "name": args.get(1).ok_or("missing theme name")?}),
        "background" => json!({"op": "background", "name": args.get(1).ok_or("missing background name")?}),
        other => return Err(format!("unknown command '{other}'")),
    })
}
