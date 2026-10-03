//! The window: tabs, input, agent status and notifications, control requests.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::Event;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use serde_json::{json, Value};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use crate::config::{Config, Profile};
use crate::ctl::Request;
use crate::font::Fonts;
use crate::input;
use crate::render::{self, Chrome, Frame, Hover, Layout, TabInfo};
use crate::session::{self, Session, Spawn, Status};
use crate::theme::{self, THEMES};

pub enum UserEvent {
    Term(u32, Event),
    Ctl(Request, mpsc::Sender<Value>),
}

const PX_PER_PT: f32 = 1.25;
const UI_PX: f32 = 13.0;

pub struct App {
    cfg: Config,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    fonts: Fonts,
    font_pt: f32,
    scale: f32,
    theme: usize,
    sessions: Vec<Session>,
    active: usize,
    next_id: u32,
    mods: ModifiersState,
    focused: bool,
    cursor: PhysicalPosition<f64>,
    hover: Hover,
    selecting: bool,
    clipboard: Option<arboard::Clipboard>,
    mem_mb: f32,
    mem_at: Instant,
    started: Instant,
    last_status: Vec<Status>,
    ctl_socket: Option<PathBuf>,
    startup_profile: Option<String>,
    os_title: String,
}

fn rss_mb() -> f32 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<f32>().ok())
        .map(|pages| pages * 4096.0 / 1_048_576.0)
        .unwrap_or(0.0)
}

fn notify(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "agentty", "-u", "normal", title, body])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

impl App {
    pub fn new(cfg: Config, proxy: EventLoopProxy<UserEvent>, ctl_socket: Option<PathBuf>, startup_profile: Option<String>) -> Result<App, String> {
        let theme = theme::by_name(&cfg.theme).unwrap_or(0);
        let font_pt = cfg.font_size;
        let fonts = Fonts::new(cfg.font.as_deref(), font_pt * PX_PER_PT, UI_PX)?;
        Ok(App {
            cfg,
            proxy,
            window: None,
            surface: None,
            fonts,
            font_pt,
            scale: 1.0,
            theme,
            sessions: Vec::new(),
            active: 0,
            next_id: 1,
            mods: ModifiersState::empty(),
            focused: true,
            cursor: PhysicalPosition::new(0.0, 0.0),
            hover: Hover::None,
            selecting: false,
            clipboard: arboard::Clipboard::new().ok(),
            mem_mb: 0.0,
            mem_at: Instant::now() - Duration::from_secs(10),
            started: Instant::now(),
            last_status: Vec::new(),
            ctl_socket,
            startup_profile,
            os_title: String::new(),
        })
    }

    fn layout(&self) -> Layout {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        Layout::new(size.width as usize, size.height as usize, self.scale, self.sessions.len() > 1)
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn apply_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.fonts.set_size(self.font_pt * PX_PER_PT * scale, UI_PX * scale);
        self.resize_sessions();
    }

    fn resize_sessions(&mut self) {
        let dims = self.layout().grid_dims(&self.fonts);
        let (cw, ch) = (self.fonts.cell_w as u16, self.fonts.cell_h as u16);
        for s in &mut self.sessions {
            s.resize(dims, cw, ch);
        }
    }

    fn sink(&self) -> session::Sink {
        let proxy = self.proxy.clone();
        Arc::new(move |id, event| {
            let _ = proxy.send_event(UserEvent::Term(id, event));
        })
    }

    pub fn open(&mut self, profile: &Profile, title: Option<String>, focus: bool) -> Result<u32, String> {
        let id = self.next_id;
        self.next_id += 1;
        // Size for the layout this tab will create (the tab strip appears at two tabs).
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        let layout = Layout::new(size.width as usize, size.height as usize, self.scale, !self.sessions.is_empty());
        let dims = layout.grid_dims(&self.fonts);
        let mut extra_env = Vec::new();
        if let Some(sock) = &self.ctl_socket {
            extra_env.push(("AGENTTY_SOCKET".to_string(), sock.display().to_string()));
        }
        let mut s = Session::spawn(Spawn {
            id,
            profile: &profile.name,
            command: &profile.command,
            cwd: profile.cwd.clone(),
            dims,
            cell_w: self.fonts.cell_w as u16,
            cell_h: self.fonts.cell_h as u16,
            scrollback: self.cfg.scrollback,
            sink: self.sink(),
            extra_env,
        })
        .map_err(|e| format!("cannot start {}: {e}", profile.command.join(" ")))?;
        if let Some(t) = title {
            s.title = t;
        }
        self.sessions.push(s);
        if focus || self.sessions.len() == 1 {
            self.active = self.sessions.len() - 1;
        }
        self.resize_sessions();
        self.redraw();
        Ok(id)
    }

    fn open_profile_index(&mut self, i: usize) {
        if let Some(p) = self.cfg.profiles.get(i).cloned() {
            if let Err(e) = self.open(&p, None, true) {
                eprintln!("agentty: {e}");
            }
        }
    }

    fn close(&mut self, index: usize, el: &ActiveEventLoop) {
        if index >= self.sessions.len() {
            return;
        }
        let s = self.sessions.remove(index);
        s.shutdown();
        if self.sessions.is_empty() {
            el.exit();
            return;
        }
        if self.active >= self.sessions.len() || self.active > index {
            self.active = self.active.saturating_sub(1).min(self.sessions.len() - 1);
        }
        self.resize_sessions();
        self.redraw();
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.sessions.iter().position(|s| s.id == id)
    }

    fn activate(&mut self, i: usize) {
        if i < self.sessions.len() {
            self.active = i;
            self.redraw();
        }
    }

    // ---- terminal events -------------------------------------------------------------

    fn on_term_event(&mut self, id: u32, event: Event) {
        let Some(i) = self.index_of(id) else { return };
        let background = !self.focused || i != self.active;
        match event {
            Event::Wakeup => {
                self.sessions[i].on_output();
                self.redraw();
            }
            Event::Title(t) => {
                self.sessions[i].title = t;
                self.redraw();
            }
            Event::ResetTitle => {
                self.sessions[i].title = self.sessions[i].profile.clone();
                self.redraw();
            }
            Event::Bell => {
                self.sessions[i].bell = true;
                if background {
                    self.notify_attention(i);
                }
                self.redraw();
            }
            Event::PtyWrite(text) => self.sessions[i].write(text.into_bytes()),
            Event::ColorRequest(index, reply) => {
                let theme = &THEMES[self.theme];
                let c = {
                    let term = self.sessions[i].term.lock();
                    term.colors()[index].map(|c| theme::Rgb(c.r, c.g, c.b)).unwrap_or_else(|| match index {
                        256 => theme.fg,
                        257 => theme.bg,
                        258 => theme.cursor,
                        i => theme.palette(i),
                    })
                };
                let answer = reply(alacritty_terminal::vte::ansi::Rgb { r: c.0, g: c.1, b: c.2 });
                self.sessions[i].write(answer.into_bytes());
            }
            Event::TextAreaSizeRequest(reply) => {
                let s = &self.sessions[i];
                let answer = reply(session::window_size(s.dims, self.fonts.cell_w as u16, self.fonts.cell_h as u16));
                s.write(answer.into_bytes());
            }
            Event::ClipboardStore(_, text) => {
                if let Some(cb) = &mut self.clipboard {
                    let _ = cb.set_text(text);
                }
            }
            Event::ClipboardLoad(_, reply) => {
                let text = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok()).unwrap_or_default();
                self.sessions[i].write(reply(&text).into_bytes());
            }
            Event::ChildExit(status) => {
                let code = status.code().unwrap_or(-1);
                self.sessions[i].exit_code = Some(code);
                if background && self.cfg.notifications {
                    let s = &self.sessions[i];
                    notify(&format!("{} finished", s.title), &format!("exited with code {code}"));
                }
                self.redraw();
            }
            _ => {}
        }
    }

    fn notify_attention(&mut self, i: usize) {
        let s = &mut self.sessions[i];
        if self.cfg.notifications && !s.notified {
            s.notified = true;
            notify(&format!("{} needs you", s.title), "An agent is waiting for your input.");
        }
    }

    /// Periodic work: detect prompts that wait for the user, refresh status dots.
    fn tick(&mut self) {
        let now = Instant::now();
        let mut changed = false;
        for i in 0..self.sessions.len() {
            self.sessions[i].check_attention(now, &self.cfg.attention_phrases);
            let st = self.sessions[i].status(now);
            if self.last_status.get(i) != Some(&st) {
                changed = true;
                if st == Status::NeedsYou && (!self.focused || i != self.active) {
                    self.notify_attention(i);
                }
            }
        }
        self.last_status = self.sessions.iter().map(|s| s.status(now)).collect();
        if changed || self.last_status.contains(&Status::NeedsYou) {
            self.redraw();
        }
    }

    // ---- drawing -------------------------------------------------------------------

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if self.mem_at.elapsed() > Duration::from_secs(2) {
            self.mem_mb = rss_mb();
            self.mem_at = Instant::now();
        }
        let layout = self.layout();
        let now = Instant::now();
        let tabs: Vec<TabInfo> = self.sessions.iter().map(|s| TabInfo { title: s.title.clone(), status: s.status(now) }).collect();
        let theme = &THEMES[self.theme];
        let pulse = ((self.started.elapsed().as_secs_f32() * 3.0).sin() + 1.0) / 2.0;
        let (window_title, status_left, status_right) = match self.sessions.get(self.active) {
            Some(s) => {
                let st = s.status(now);
                let hint = match st {
                    Status::NeedsYou => " · waiting for your input",
                    Status::Exited(_) => " · Ctrl+Shift+W to close",
                    _ => "",
                };
                let st_label = match st {
                    Status::Exited(c) => format!("exited ({c})"),
                    other => other.label().to_string(),
                };
                (
                    format!("{} — {}×{}", s.title, s.dims.cols, s.dims.rows),
                    format!("{} · {}{}", s.profile, st_label, hint),
                    format!("{} · {:.0} MB · Ctrl+Shift+P theme · Ctrl+Shift+T tab", theme.name, self.mem_mb),
                )
            }
            None => ("agentty".into(), String::new(), String::new()),
        };
        // Keep the OS-level title in sync: the dock's window list and Alt+Tab show it.
        if window_title != self.os_title {
            window.set_title(&window_title);
            self.os_title = window_title.clone();
        }
        let chrome = Chrome {
            window_title,
            tabs: &tabs,
            active: self.active,
            hover: self.hover,
            focused: self.focused,
            status_left,
            status_right,
            pulse,
        };
        let Some(surface) = self.surface.as_mut() else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else { return };
        {
            let mut frame = Frame { buf: &mut buffer, w: size.width as usize, h: size.height as usize };
            frame.fill(0, 0, frame.w, frame.h, theme.bg);
            if let Some(s) = self.sessions.get(self.active) {
                let term = s.term.lock();
                render::draw_terminal(&mut frame, &mut self.fonts, theme, &layout, &term, self.focused);
            }
            render::draw_chrome(&mut frame, &mut self.fonts, theme, &layout, &chrome);
        }
        let _ = buffer.present();
    }

    // ---- mouse -----------------------------------------------------------------------

    fn hit_hover(&self) -> Hover {
        let l = self.layout();
        let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
        if y < l.title_h as f32 {
            if x < 72.0 * self.scale {
                return Hover::Lights;
            }
            let (bx, by, bs) = l.new_tab_button();
            if x >= bx as f32 && x < (bx + bs) as f32 && y >= by as f32 && y < (by + bs) as f32 {
                return Hover::NewTab;
            }
            return Hover::None;
        }
        if l.tabs_h > 0 && y < (l.title_h + l.tabs_h) as f32 {
            for (i, (tx, tw)) in l.tab_rects(self.sessions.len()).into_iter().enumerate() {
                if x >= tx as f32 && x < (tx + tw) as f32 {
                    return Hover::Tab(i);
                }
            }
        }
        Hover::None
    }

    fn resize_edge(&self) -> Option<ResizeDirection> {
        let l = self.layout();
        let e = 5.0 * self.scale as f64;
        let (x, y, w, h) = (self.cursor.x, self.cursor.y, l.w as f64, l.h as f64);
        let (left, right, top, bottom) = (x < e, x > w - e, y < e, y > h - e);
        Some(match (left, right, top, bottom) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn grid_point(&self) -> Option<(Point, Side)> {
        let s = self.sessions.get(self.active)?;
        let l = self.layout();
        let (ox, oy) = l.grid_origin();
        let (x, y) = (self.cursor.x - ox as f64, self.cursor.y - oy as f64);
        let (cw, ch) = (self.fonts.cell_w as f64, self.fonts.cell_h as f64);
        let col = (x / cw).floor().clamp(0.0, s.dims.cols as f64 - 1.0) as usize;
        let row = (y / ch).floor().clamp(0.0, s.dims.rows as f64 - 1.0) as i32;
        let side = if (x / cw).fract() < 0.5 { Side::Left } else { Side::Right };
        let offset = s.term.lock().grid().display_offset() as i32;
        Some((Point::new(Line(row - offset), Column(col)), side))
    }

    fn on_mouse_press(&mut self, button: MouseButton, el: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else { return };
        let l = self.layout();
        if button == MouseButton::Left {
            if let Some(dir) = self.resize_edge() {
                let _ = window.drag_resize_window(dir);
                return;
            }
        }
        match (self.hit_hover(), button) {
            (Hover::Lights, MouseButton::Left) => {
                let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
                for (i, (cx, cy)) in l.traffic_lights().into_iter().enumerate() {
                    if (x - cx).hypot(y - cy) <= l.light_radius() + 3.0 * self.scale {
                        match i {
                            0 => {
                                for s in &self.sessions {
                                    s.shutdown();
                                }
                                el.exit();
                            }
                            1 => window.set_minimized(true),
                            _ => window.set_maximized(!window.is_maximized()),
                        }
                        return;
                    }
                }
                let _ = window.drag_window();
            }
            (Hover::NewTab, MouseButton::Left) => self.open_profile_index(0),
            (Hover::Tab(i), MouseButton::Left) => self.activate(i),
            (Hover::Tab(i), MouseButton::Middle) => self.close(i, el),
            (_, MouseButton::Left) if self.cursor.y < (l.title_h + l.tabs_h) as f64 => {
                let _ = window.drag_window();
            }
            (_, MouseButton::Left) => {
                if let Some((point, side)) = self.grid_point() {
                    let s = &self.sessions[self.active];
                    s.term.lock().selection = Some(Selection::new(SelectionType::Simple, point, side));
                    self.selecting = true;
                    self.redraw();
                }
            }
            (_, MouseButton::Middle) => self.paste(),
            _ => {}
        }
    }

    fn on_cursor_moved(&mut self) {
        let hover = self.hit_hover();
        if hover != self.hover {
            self.hover = hover;
            self.redraw();
        }
        if let Some(w) = &self.window {
            let icon = match self.resize_edge() {
                Some(ResizeDirection::East | ResizeDirection::West) => CursorIcon::EwResize,
                Some(ResizeDirection::North | ResizeDirection::South) => CursorIcon::NsResize,
                Some(ResizeDirection::NorthEast | ResizeDirection::SouthWest) => CursorIcon::NeswResize,
                Some(ResizeDirection::NorthWest | ResizeDirection::SouthEast) => CursorIcon::NwseResize,
                None if self.hover != Hover::None || self.cursor.y < self.layout().title_h as f64 => CursorIcon::Default,
                None => CursorIcon::Text,
            };
            w.set_cursor(icon);
        }
        if self.selecting {
            if let Some((point, side)) = self.grid_point() {
                if let Some(sel) = self.sessions[self.active].term.lock().selection.as_mut() {
                    sel.update(point, side);
                }
                self.redraw();
            }
        }
    }

    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        let Some(s) = self.sessions.get(self.active) else { return };
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * 3.0).round() as i32,
            MouseScrollDelta::PixelDelta(p) => (p.y / self.fonts.cell_h as f64).round() as i32,
        };
        if lines == 0 {
            return;
        }
        let mode = *s.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            let key: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            let key = if mode.contains(TermMode::APP_CURSOR) { if lines > 0 { b"\x1bOA" as &[u8] } else { b"\x1bOB" } } else { key };
            s.write(key.repeat(lines.unsigned_abs() as usize));
        } else {
            s.scroll(Scroll::Delta(lines));
        }
        self.redraw();
    }

    // ---- keyboard --------------------------------------------------------------------

    fn copy(&mut self) {
        let Some(s) = self.sessions.get(self.active) else { return };
        if let Some(text) = s.term.lock().selection_to_string() {
            if let Some(cb) = &mut self.clipboard {
                let _ = cb.set_text(text);
            }
        }
    }

    fn paste(&mut self) {
        let text = self.clipboard.as_mut().and_then(|cb| cb.get_text().ok());
        let Some(s) = self.sessions.get_mut(self.active) else { return };
        if let Some(text) = text {
            let mode = *s.term.lock().mode();
            s.write(input::paste(&text, mode));
            s.scroll(Scroll::Bottom);
            s.acknowledge();
        }
    }

    fn change_font(&mut self, delta: f32) {
        self.font_pt = if delta == 0.0 { self.cfg.font_size } else { (self.font_pt + delta).clamp(6.0, 48.0) };
        self.apply_scale(self.scale);
        self.redraw();
    }

    fn cycle_tab(&mut self, forward: bool) {
        let n = self.sessions.len();
        if n > 1 {
            self.active = if forward { (self.active + 1) % n } else { (self.active + n - 1) % n };
            self.redraw();
        }
    }

    /// App shortcuts. Returns true if the key was handled here and must not reach the program.
    fn shortcut(&mut self, code: KeyCode, el: &ActiveEventLoop) -> bool {
        let m = self.mods;
        let ctrl_shift = m.control_key() && m.shift_key() && !m.alt_key();
        if ctrl_shift {
            match code {
                KeyCode::KeyT => self.open_profile_index(0),
                KeyCode::KeyW => self.close(self.active, el),
                KeyCode::KeyC => self.copy(),
                KeyCode::KeyV => self.paste(),
                KeyCode::KeyP => {
                    self.theme = (self.theme + 1) % THEMES.len();
                    self.redraw();
                }
                KeyCode::Equal => self.change_font(1.0),
                KeyCode::Minus => self.change_font(-1.0),
                KeyCode::Digit0 => self.change_font(0.0),
                KeyCode::Tab => self.cycle_tab(false),
                KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6
                | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9 => {
                    let n = code as usize - KeyCode::Digit1 as usize;
                    self.open_profile_index(n);
                }
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End => return false,
                _ => return false,
            }
            return true;
        }
        if m.control_key() && !m.shift_key() && !m.alt_key() {
            match code {
                KeyCode::Tab => self.cycle_tab(true),
                KeyCode::PageDown => self.cycle_tab(true),
                KeyCode::PageUp => self.cycle_tab(false),
                _ => return false,
            }
            return true;
        }
        if m.alt_key() && !m.control_key() && !m.shift_key() {
            let digits = [
                KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5,
                KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8, KeyCode::Digit9,
            ];
            if let Some(n) = digits.iter().position(|d| *d == code) {
                self.activate(n);
                return true;
            }
        }
        if m.shift_key() && !m.control_key() && !m.alt_key() {
            let scroll = match code {
                KeyCode::PageUp => Scroll::PageUp,
                KeyCode::PageDown => Scroll::PageDown,
                KeyCode::Home => Scroll::Top,
                KeyCode::End => Scroll::Bottom,
                _ => return false,
            };
            if let Some(s) = self.sessions.get(self.active) {
                s.scroll(scroll);
                self.redraw();
            }
            return true;
        }
        false
    }

    // ---- control API -----------------------------------------------------------------

    fn tab_json(&self, i: usize) -> Value {
        let s = &self.sessions[i];
        let st = s.status(Instant::now());
        json!({
            "id": s.id, "title": s.title, "profile": s.profile, "status": st.label(),
            "exit_code": s.exit_code, "cols": s.dims.cols, "rows": s.dims.rows, "active": i == self.active,
        })
    }

    fn on_ctl(&mut self, req: Request, el: &ActiveEventLoop) -> Value {
        let missing = |id: u32| json!({"ok": false, "error": format!("no tab with id {id}")});
        match req {
            Request::List => json!({"ok": true, "tabs": (0..self.sessions.len()).map(|i| self.tab_json(i)).collect::<Vec<_>>()}),
            Request::Open { profile, command, cwd, title, focus } => {
                let mut p = match (profile, command) {
                    (_, Some(cmd)) if !cmd.is_empty() => Profile { name: cmd[0].clone(), command: cmd, cwd: None },
                    (Some(name), _) => match self.cfg.profile(&name) {
                        Some(p) => p.clone(),
                        None => {
                            let names: Vec<&str> = self.cfg.profiles.iter().map(|p| p.name.as_str()).collect();
                            return json!({"ok": false, "error": format!("unknown profile '{name}'"), "profiles": names});
                        }
                    },
                    _ => return json!({"ok": false, "error": "give a profile or a command"}),
                };
                if cwd.is_some() {
                    p.cwd = cwd;
                }
                match self.open(&p, title, focus) {
                    Ok(id) => json!({"ok": true, "id": id}),
                    Err(e) => json!({"ok": false, "error": e}),
                }
            }
            Request::Send { id, text, enter, paste } => {
                let Some(i) = self.index_of(id) else { return missing(id) };
                let s = &mut self.sessions[i];
                let mode = *s.term.lock().mode();
                let mut bytes = if paste { input::paste(&text, mode) } else { text.into_bytes() };
                if enter {
                    bytes.push(b'\r');
                }
                s.write(bytes);
                s.acknowledge();
                s.on_output(); // count as activity so `wait` sees the agent start
                json!({"ok": true})
            }
            Request::Read { id, lines } => {
                let Some(i) = self.index_of(id) else { return missing(id) };
                let text = self.sessions[i].text_tail(lines.clamp(1, 100_000));
                json!({"ok": true, "text": text, "tab": self.tab_json(i)})
            }
            Request::Status { id } => match self.index_of(id) {
                Some(i) => json!({"ok": true, "status": self.sessions[i].status(Instant::now()).label(), "tab": self.tab_json(i)}),
                None => missing(id),
            },
            Request::Focus { id } => match self.index_of(id) {
                Some(i) => {
                    self.activate(i);
                    json!({"ok": true})
                }
                None => missing(id),
            },
            Request::Close { id } => match self.index_of(id) {
                Some(i) => {
                    self.close(i, el);
                    json!({"ok": true})
                }
                None => missing(id),
            },
            Request::Theme { name } => match theme::by_name(&name) {
                Some(t) => {
                    self.theme = t;
                    self.redraw();
                    json!({"ok": true, "theme": THEMES[t].name})
                }
                None => json!({"ok": false, "error": "unknown theme", "themes": THEMES.iter().map(|t| t.name).collect::<Vec<_>>()}),
            },
            Request::Wait { .. } => json!({"ok": false, "error": "wait is handled by the socket thread"}),
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("agentty")
            .with_inner_size(LogicalSize::new(980.0, 640.0))
            .with_min_inner_size(LogicalSize::new(420.0, 240.0))
            .with_decorations(false);
        // App ID (Wayland) / WM_CLASS (X11) must match agentty.desktop, or the dock shows the
        // window as an unknown app with a generic icon instead of under the pinned agentty icon.
        #[cfg(all(unix, not(target_os = "macos")))]
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(attrs, "agentty", "agentty");
        let window = match el.create_window(attrs) {
            Ok(w) => Rc::new(w),
            Err(e) => {
                eprintln!("agentty: cannot create window: {e}");
                el.exit();
                return;
            }
        };
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("agentty: no display: {e}");
                el.exit();
                return;
            }
        };
        self.surface = softbuffer::Surface::new(&context, window.clone()).ok();
        self.window = Some(window.clone());
        self.apply_scale(window.scale_factor() as f32);
        let start = self
            .startup_profile
            .clone()
            .or(self.cfg.startup_profile.clone())
            .and_then(|n| self.cfg.profile(&n).cloned())
            .unwrap_or_else(|| self.cfg.profiles[0].clone());
        if let Err(e) = self.open(&start, None, true) {
            eprintln!("agentty: {e}");
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                for s in &self.sessions {
                    s.shutdown();
                }
                el.exit();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::Resized(_) => {
                self.resize_sessions();
                self.redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.apply_scale(scale_factor as f32);
                self.redraw();
            }
            WindowEvent::Focused(f) => {
                self.focused = f;
                if let Some(s) = self.sessions.get(self.active) {
                    if s.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                        s.write(if f { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                    }
                }
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if self.shortcut(code, el) {
                        return;
                    }
                }
                let Some(s) = self.sessions.get_mut(self.active) else { return };
                let mode = *s.term.lock().mode();
                if let Some(bytes) = input::encode(&event, self.mods, mode) {
                    s.write(bytes);
                    {
                        let mut term = s.term.lock();
                        term.selection = None;
                        term.scroll_display(Scroll::Bottom);
                    }
                    s.acknowledge();
                    self.redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                self.on_cursor_moved();
            }
            WindowEvent::CursorLeft { .. } => {
                if self.hover != Hover::None {
                    self.hover = Hover::None;
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => self.on_mouse_press(button, el),
                ElementState::Released => {
                    if button == MouseButton::Left && self.selecting {
                        self.selecting = false;
                        if let Some(s) = self.sessions.get(self.active) {
                            let mut term = s.term.lock();
                            if term.selection.as_ref().is_some_and(|sel| sel.is_empty()) {
                                term.selection = None;
                            }
                        }
                        self.redraw();
                    }
                }
            },
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta),
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Term(id, ev) => self.on_term_event(id, ev),
            UserEvent::Ctl(req, reply) => {
                let v = self.on_ctl(req, el);
                let _ = reply.send(v);
            }
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.tick();
        let now = Instant::now();
        // Wake again only while something can still change: a tab is working, waiting for the
        // attention check, or pulsing. Otherwise sleep until an event arrives (0% CPU when idle).
        let busy = self.sessions.iter().any(|s| {
            matches!(s.status(now), Status::Working | Status::NeedsYou)
                || (!s.attention_checked && s.last_output.is_some_and(|t| now.duration_since(t) < session::ATTENTION_QUIET * 2))
        });
        if busy {
            el.set_control_flow(ControlFlow::WaitUntil(now + Duration::from_millis(250)));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
    }
}
