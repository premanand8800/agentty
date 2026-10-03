//! `~/.config/agentty/config.toml`. Every key is optional.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// One of the names in `agentty themes`.
    pub theme: String,
    pub font_size: f32,
    /// A monospace font: family name ("JetBrains Mono") or path to a TTF/OTF/TTC file.
    /// Default: Menlo on macOS, Consolas on Windows, DejaVu Sans Mono (Menlo's ancestor) on Linux.
    pub font: Option<String>,
    /// Ambient background: "off", "starfield", "rain", "snow", "fireflies" or "aurora".
    pub background: String,
    /// How visible the background is, 0.0 to 1.0.
    pub background_intensity: f32,
    /// Animation frame rate while the window is focused (halved when it is not).
    pub background_fps: u32,
    /// Lines kept per tab. Agents print a lot; this bounds memory.
    pub scrollback: usize,
    /// Desktop notification when a background tab needs you.
    pub notifications: bool,
    /// Phrases that mean "an agent is waiting for you" when they appear on screen after output stops.
    pub attention_phrases: Vec<String>,
    /// Profile opened in the first tab.
    pub startup_profile: Option<String>,
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            theme: "Pro".into(),
            font_size: 13.5,
            font: None,
            background: "off".into(),
            background_intensity: 0.6,
            background_fps: 20,
            scrollback: 5000,
            notifications: true,
            attention_phrases: [
                "do you want to",
                "(y/n)",
                "[y/n]",
                "allow once",
                "approve",
                "press enter to",
                "waiting for your",
                "permission",
                "❯ 1. yes",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            startup_profile: None,
            profiles: Vec::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"));
    base.join("agentty").join("config.toml")
}

/// Settings changed with shortcuts, saved automatically so they survive a restart.
/// Kept apart from config.toml so a hand-written config is never rewritten.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiState {
    pub theme: Option<String>,
    pub font: Option<String>,
    pub font_size: Option<f32>,
    pub background: Option<String>,
}

impl UiState {
    pub fn apply(&self, cfg: &mut Config) {
        if let Some(v) = &self.theme {
            cfg.theme = v.clone();
        }
        if let Some(v) = &self.font {
            cfg.font = Some(v.clone());
        }
        if let Some(v) = self.font_size {
            cfg.font_size = v;
        }
        if let Some(v) = &self.background {
            cfg.background = v.clone();
        }
    }

    pub fn load() -> UiState {
        std::fs::read_to_string(ui_state_path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    /// Write atomically (temp file + rename) so a crash never leaves a half-written file.
    pub fn save(&self) -> std::io::Result<()> {
        let path = ui_state_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("# Saved by agentty when you change these with shortcuts.\n{text}"))?;
        std::fs::rename(tmp, path)
    }
}

pub fn ui_state_path() -> PathBuf {
    config_path().with_file_name("ui.toml")
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Full path of `bin` on PATH. On Windows also tries PATHEXT (.exe, .cmd, ...).
fn find_on_path(bin: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(|e| e.to_ascii_lowercase())
            .chain(std::iter::once(String::new()))
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{bin}{ext}"))))
        .find(|p| p.is_file())
}

/// The command to start `bin`. Windows runs npm's .cmd shims through cmd.exe.
fn agent_command(bin: &str) -> Option<Vec<String>> {
    let path = find_on_path(bin)?;
    let is_script = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    Some(if is_script {
        vec!["cmd.exe".into(), "/c".into(), path.to_string_lossy().into_owned()]
    } else {
        vec![bin.to_string()]
    })
}

fn default_shell() -> Vec<String> {
    if cfg!(windows) {
        let ps = if find_on_path("pwsh").is_some() { "pwsh.exe" } else { "powershell.exe" };
        vec![ps.into(), "-NoLogo".into()]
    } else {
        let fallback = if cfg!(target_os = "macos") { "/bin/zsh" } else { "/bin/bash" };
        vec![std::env::var("SHELL").unwrap_or_else(|_| fallback.into())]
    }
}

impl Config {
    pub fn load() -> Result<Config, String> {
        let path = config_path();
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str::<Config>(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(_) => Config::default(),
        };
        // Choices made with shortcuts (theme, font, size, background) live in ui.toml and win.
        if let Ok(text) = std::fs::read_to_string(ui_state_path()) {
            if let Ok(ui) = toml::from_str::<UiState>(&text) {
                ui.apply(&mut cfg);
            }
        }
        cfg.font_size = cfg.font_size.clamp(6.0, 48.0);
        cfg.background_intensity = cfg.background_intensity.clamp(0.0, 1.0);
        cfg.background_fps = cfg.background_fps.clamp(1, 60);
        cfg.scrollback = cfg.scrollback.min(200_000);
        if cfg.profiles.is_empty() {
            cfg.profiles = default_profiles();
        }
        Ok(cfg)
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
}

/// The user's shell, then every agent CLI found on PATH.
pub fn default_profiles() -> Vec<Profile> {
    let mut profiles = vec![Profile { name: "Shell".into(), command: default_shell(), cwd: None }];
    for (name, bin) in [
        ("Claude", "claude"),
        ("Codex", "codex"),
        ("Antigravity", "agy"),
        ("Gemini", "gemini"),
        ("Aider", "aider"),
        ("Goose", "goose"),
    ] {
        if let Some(command) = agent_command(bin) {
            profiles.push(Profile { name: name.into(), command, cwd: None });
        }
    }
    profiles
}
