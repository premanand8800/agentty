//! `agentty snapshot`: run a command in a hidden terminal and save a PNG of the window.
//! Same renderer as the app. Used for docs and visual tests; needs no display.

use std::io::BufWriter;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use alacritty_terminal::event::Event;

use crate::config::Config;
use crate::font::Fonts;
use crate::render::{self, Chrome, Frame, Hover, Layout, TabInfo};
use crate::session::{Dims, Session, Spawn, Status};
use crate::theme::{self, THEMES};

pub struct Options {
    pub out: String,
    pub theme: Option<String>,
    pub cols: usize,
    pub rows: usize,
    pub scale: f32,
    pub wait_ms: u64,
    pub max_ms: u64,
    pub title: Option<String>,
    pub tabs: Vec<(String, Status)>,
    pub command: Vec<String>,
}

pub fn run(cfg: &Config, o: Options) -> Result<(), String> {
    let theme_idx = o.theme.as_deref().map(|n| theme::by_name(n).ok_or(format!("unknown theme '{n}'"))).transpose()?.unwrap_or_else(|| theme::by_name(&cfg.theme).unwrap_or(0));
    let theme = &THEMES[theme_idx];
    let mut fonts = Fonts::new(cfg.font.as_deref(), cfg.font_size * 1.25 * o.scale, 13.0 * o.scale)?;
    let show_tabs = !o.tabs.is_empty();
    // Size the window so the grid is exactly cols x rows.
    let probe = Layout::new(0, 0, o.scale, show_tabs);
    let w = o.cols * fonts.cell_w + 2 * probe.pad;
    let h = o.rows * fonts.cell_h + probe.title_h + probe.tabs_h + probe.status_h + 2 * probe.pad;
    let layout = Layout::new(w, h, o.scale, show_tabs);
    let dims = Dims { cols: o.cols, rows: o.rows };

    let (tx, rx) = mpsc::channel::<Event>();
    let tx = std::sync::Mutex::new(tx);
    let sink = Arc::new(move |_id: u32, e: Event| {
        let _ = tx.lock().unwrap().send(e);
    });
    let profile = o.command.first().cloned().unwrap_or_default();
    let mut s = Session::spawn(Spawn {
        id: 1,
        profile: &profile,
        command: &o.command,
        cwd: None,
        dims,
        cell_w: fonts.cell_w as u16,
        cell_h: fonts.cell_h as u16,
        scrollback: cfg.scrollback,
        sink,
        extra_env: Vec::new(),
    })
    .map_err(|e| format!("cannot start {}: {e}", o.command.join(" ")))?;

    let start = Instant::now();
    let mut last = Instant::now();
    let mut seen_output = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Wakeup) => {
                seen_output = true;
                last = Instant::now();
            }
            Ok(Event::PtyWrite(text)) => s.write(text.into_bytes()),
            Ok(Event::Title(t)) => s.title = t,
            Ok(Event::ChildExit(st)) => s.exit_code = Some(st.code().unwrap_or(-1)),
            Ok(Event::Exit) => break,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let quiet = last.elapsed() >= Duration::from_millis(o.wait_ms);
        if (seen_output && quiet) || start.elapsed() >= Duration::from_millis(o.max_ms) {
            break;
        }
    }

    let mut buf = vec![0u32; w * h];
    let mut frame = Frame { buf: &mut buf, w, h };
    frame.fill(0, 0, w, h, theme.bg);
    {
        let term = s.term.lock();
        render::draw_terminal(&mut frame, &mut fonts, theme, &layout, &term, true);
    }
    let title = o.title.clone().unwrap_or_else(|| s.title.clone());
    let mut tabs: Vec<TabInfo> = o.tabs.iter().map(|(t, st)| TabInfo { title: t.clone(), status: *st }).collect();
    let status = s.exit_code.map(Status::Exited).unwrap_or(Status::Idle);
    if tabs.is_empty() {
        tabs.push(TabInfo { title: title.clone(), status });
    }
    let active_status = tabs[0].status;
    let chrome = Chrome {
        window_title: format!("{title} — {}×{}", o.cols, o.rows),
        tabs: &tabs,
        active: 0,
        hover: Hover::None,
        focused: true,
        status_left: format!("{} · {}", tabs[0].title, active_status.label()),
        status_right: format!("{} · Ctrl+Shift+P theme · Ctrl+Shift+T tab", theme.name),
        pulse: 0.0,
    };
    render::draw_chrome(&mut frame, &mut fonts, theme, &layout, &chrome);
    s.shutdown();
    write_png(&o.out, &buf, w, h)
}

fn write_png(path: &str, buf: &[u32], w: usize, h: usize) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut enc = png::Encoder::new(BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    let mut data = Vec::with_capacity(w * h * 3);
    for p in buf {
        data.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
    }
    writer.write_image_data(&data).map_err(|e| e.to_string())
}
