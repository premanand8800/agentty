//! One tab: a PTY running a program, the terminal state, and agent-status tracking.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::tty::{self, Options, Shell};

/// Delivers terminal events to whoever owns the session (the window or a headless snapshot).
pub type Sink = Arc<dyn Fn(u32, Event) + Send + Sync>;

#[derive(Clone)]
pub struct Proxy {
    id: u32,
    sink: Sink,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        (self.sink)(self.id, event);
    }
}

#[derive(Clone, Copy)]
pub struct Dims {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Output arrived in the last moment: the agent is working.
    Working,
    /// Quiet, nothing asked of you.
    Idle,
    /// Rang the bell or shows a prompt that waits for you.
    NeedsYou,
    /// The program exited.
    Exited(i32),
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Working => "working",
            Status::Idle => "idle",
            Status::NeedsYou => "needs you",
            Status::Exited(_) => "exited",
        }
    }
}

const WORKING_WINDOW: Duration = Duration::from_millis(900);
/// How long output must be quiet before the screen is checked for a waiting prompt.
pub const ATTENTION_QUIET: Duration = Duration::from_millis(1200);

pub struct Session {
    pub id: u32,
    pub profile: String,
    pub title: String,
    pub term: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    pub dims: Dims,
    pub last_output: Option<Instant>,
    pub bell: bool,
    pub exit_code: Option<i32>,
    pub prompt_detected: bool,
    pub attention_checked: bool,
    pub notified: bool,
}

pub struct Spawn<'a> {
    pub id: u32,
    pub profile: &'a str,
    pub command: &'a [String],
    pub cwd: Option<PathBuf>,
    pub dims: Dims,
    pub cell_w: u16,
    pub cell_h: u16,
    pub scrollback: usize,
    pub sink: Sink,
    pub extra_env: Vec<(String, String)>,
}

impl Session {
    pub fn spawn(s: Spawn) -> std::io::Result<Session> {
        let (program, args) = s
            .command
            .split_first()
            .map(|(p, a)| (p.clone(), a.to_vec()))
            .ok_or_else(|| std::io::Error::other("empty command"))?;
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "agentty".to_string());
        env.insert("TERM_PROGRAM_VERSION".to_string(), env!("CARGO_PKG_VERSION").to_string());
        env.insert("AGENTTY_TAB_ID".to_string(), s.id.to_string());
        env.extend(s.extra_env);
        let options = Options {
            shell: Some(Shell::new(program, args)),
            working_directory: s.cwd.or_else(|| std::env::current_dir().ok()),
            drain_on_exit: true,
            env,
        };
        let size = window_size(s.dims, s.cell_w, s.cell_h);
        let pty = tty::new(&options, size, s.id as u64)?;
        let proxy = Proxy { id: s.id, sink: s.sink };
        let config = TermConfig { scrolling_history: s.scrollback, ..TermConfig::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &s.dims, proxy.clone())));
        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Session {
            id: s.id,
            profile: s.profile.to_string(),
            title: s.profile.to_string(),
            term,
            sender,
            dims: s.dims,
            last_output: None,
            bell: false,
            exit_code: None,
            prompt_detected: false,
            attention_checked: false,
            notified: false,
        })
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    pub fn resize(&mut self, dims: Dims, cell_w: u16, cell_h: u16) {
        if dims.cols == self.dims.cols && dims.rows == self.dims.rows {
            return;
        }
        self.dims = dims;
        self.term.lock().resize(dims);
        let _ = self.sender.send(Msg::Resize(window_size(dims, cell_w, cell_h)));
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(Msg::Shutdown);
    }

    pub fn scroll(&self, scroll: Scroll) {
        self.term.lock().scroll_display(scroll);
    }

    pub fn on_output(&mut self) {
        self.last_output = Some(Instant::now());
        self.attention_checked = false;
        self.prompt_detected = false;
        self.notified = false;
    }

    pub fn status(&self, now: Instant) -> Status {
        if let Some(code) = self.exit_code {
            return Status::Exited(code);
        }
        if self.bell || self.prompt_detected {
            return Status::NeedsYou;
        }
        match self.last_output {
            Some(t) if now.duration_since(t) < WORKING_WINDOW => Status::Working,
            _ => Status::Idle,
        }
    }

    /// Clear the "needs you" flag once the user has seen the tab and typed into it.
    pub fn acknowledge(&mut self) {
        self.bell = false;
        self.prompt_detected = false;
        self.notified = false;
    }

    /// After output goes quiet, look for a prompt that waits for the user. Runs once per quiet period.
    pub fn check_attention(&mut self, now: Instant, phrases: &[String]) {
        if self.attention_checked || self.exit_code.is_some() {
            return;
        }
        let Some(t) = self.last_output else { return };
        if now.duration_since(t) < ATTENTION_QUIET {
            return;
        }
        self.attention_checked = true;
        let screen = self.screen_text(12).to_lowercase();
        self.prompt_detected = phrases.iter().any(|p| screen.contains(&p.to_lowercase()));
    }

    /// The last `lines` lines (scrollback + screen) as plain text: no escape codes, trailing spaces trimmed.
    pub fn text_tail(&self, lines: usize) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let cols = grid.columns();
        let screen = grid.screen_lines() as i32;
        let history = grid.history_size() as i32;
        // End at the last screen line with content (or the cursor), not the bottom of the window:
        // a short session leaves the lower rows blank.
        let blank = |line: i32| {
            let row = &grid[Line(line)];
            (0..cols).all(|c| matches!(row[Column(c)].c, ' ' | '\0'))
        };
        let mut end = screen;
        let cursor_line = grid.cursor.point.line.0;
        while end - 1 > cursor_line && blank(end - 1) {
            end -= 1;
        }
        let start = (end - lines as i32).max(-history);
        let mut out = String::new();
        for line in start..end {
            let row = &grid[Line(line)];
            let mut s = String::with_capacity(cols);
            for col in 0..cols {
                let cell = &row[Column(col)];
                if cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                s.push(cell.c);
            }
            out.push_str(s.trim_end());
            out.push('\n');
        }
        let trimmed = out.trim_end_matches('\n').len();
        out.truncate(trimmed);
        out
    }

    fn screen_text(&self, last_lines: usize) -> String {
        let text = self.text_tail(self.dims.rows);
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(last_lines)..].join("\n")
    }
}

pub fn window_size(dims: Dims, cell_w: u16, cell_h: u16) -> WindowSize {
    WindowSize { num_lines: dims.rows as u16, num_cols: dims.cols as u16, cell_width: cell_w, cell_height: cell_h }
}
