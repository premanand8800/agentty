//! Dock badge: tab count on the app icon, and an "urgent" highlight when an agent needs you.
//!
//! Linux/BSD: the Unity LauncherEntry D-Bus API, read by Ubuntu Dock, Dash to Dock, KDE Plasma
//! and others. The dock drops the badge when the sender disconnects, so a dedicated thread keeps
//! one session-bus connection open for the window's lifetime. Other platforms: no-op for now.

use std::sync::mpsc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub tabs: usize,
    pub urgent: bool,
}

pub struct Badge {
    tx: Option<mpsc::Sender<State>>,
    last: Option<State>,
}

impl Badge {
    pub fn new() -> Badge {
        Badge { tx: platform::start(), last: None }
    }

    /// Publish the state if it changed. Never blocks the UI thread.
    pub fn set(&mut self, state: State) {
        if self.last == Some(state) {
            return;
        }
        self.last = Some(state);
        if let Some(tx) = &self.tx {
            let _ = tx.send(state);
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::State;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use zbus::zvariant::Value;

    const APP_URI: &str = "application://agentty.desktop";

    pub fn start() -> Option<mpsc::Sender<State>> {
        let (tx, rx) = mpsc::channel::<State>();
        std::thread::Builder::new()
            .name("agentty-badge".into())
            .spawn(move || {
                let Ok(conn) = zbus::blocking::Connection::session() else { return };
                let path = format!("/com/canonical/unity/launcherentry/{}", std::process::id());
                let emit = |s: State| {
                    // A badge for a single tab is noise; show it from two tabs up.
                    let props: HashMap<&str, Value> = HashMap::from([
                        ("count", Value::from(s.tabs as i64)),
                        ("count-visible", Value::from(s.tabs >= 2)),
                        ("urgent", Value::from(s.urgent)),
                    ]);
                    let _ = conn.emit_signal(
                        None::<&str>,
                        path.as_str(),
                        "com.canonical.Unity.LauncherEntry",
                        "Update",
                        &(APP_URI, props),
                    );
                };
                for state in rx {
                    emit(state);
                }
                emit(State::default()); // window closing: clear the badge
            })
            .ok()?;
        Some(tx)
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod platform {
    pub fn start() -> Option<std::sync::mpsc::Sender<super::State>> {
        None
    }
}
