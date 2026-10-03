//! Unit tests for the parts that do not need a display.

use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, NamedColor};

use crate::ctl::{Request, WaitFor};
use crate::render::Frame;
use crate::theme::{self, Rgb, THEMES};

#[test]
fn every_theme_resolves_named_and_indexed_colors() {
    let colors = alacritty_terminal::term::color::Colors::default();
    for t in THEMES.iter() {
        assert_eq!(t.resolve(Color::Named(NamedColor::Foreground), &colors, false), t.fg);
        assert_eq!(t.resolve(Color::Named(NamedColor::Background), &colors, false), t.bg);
        // Bold promotes the 8 basic colors to their bright variants, like Terminal.app.
        assert_eq!(t.resolve(Color::Named(NamedColor::Red), &colors, true), t.palette(9));
        assert_eq!(t.resolve(Color::Indexed(196), &colors, false), Rgb(255, 0, 0));
        assert_eq!(t.resolve(Color::Indexed(232), &colors, false), Rgb(8, 8, 8));
    }
    assert_eq!(theme::by_name("man page"), Some(6));
    assert!(THEMES[theme::by_name("Pro").unwrap()].is_dark());
    assert!(!THEMES[theme::by_name("Basic").unwrap()].is_dark());
}

#[test]
fn bracketed_paste_wraps_and_strips_escapes() {
    let out = crate::input::paste("a\nb\x1b[31m", TermMode::BRACKETED_PASTE);
    assert_eq!(out, b"\x1b[200~a\rb[31m\x1b[201~");
    assert_eq!(crate::input::paste("x\r\ny", TermMode::empty()), b"x\ry");
}

#[test]
fn box_lines_join_across_cells() {
    let (w, h) = (10usize, 20usize);
    let mut buf = vec![0u32; w * 2 * h];
    let mut f = Frame { buf: &mut buf, w: w * 2, h };
    let white = Rgb(255, 255, 255);
    assert!(crate::boxdraw::draw(&mut f, '─', 0, 0, w, h, white, 1.0));
    assert!(crate::boxdraw::draw(&mut f, '─', w, 0, w, h, white, 1.0));
    // The horizontal stroke is continuous across both cells on the center row.
    let row = h / 2;
    assert!((0..2 * w).all(|x| buf[row * 2 * w + x] == white.to_u32()));
    let mut buf2 = vec![0u32; w * h];
    let mut f2 = Frame { buf: &mut buf2, w, h };
    assert!(!crate::boxdraw::draw(&mut f2, 'a', 0, 0, w, h, white, 1.0));
    assert!(crate::boxdraw::draw(&mut f2, '╭', 0, 0, w, h, white, 1.0));
    assert!(buf2.iter().any(|&p| p != 0));
}

#[test]
fn control_requests_parse_with_defaults() {
    let r: Request = serde_json::from_str(r#"{"op":"send","id":3,"text":"hi"}"#).unwrap();
    assert!(matches!(r, Request::Send { id: 3, enter: false, paste: false, .. }));
    let r: Request = serde_json::from_str(r#"{"op":"wait","id":1}"#).unwrap();
    assert!(matches!(r, Request::Wait { id: 1, until: WaitFor::Settled, timeout_ms: None }));
    let r: Request = serde_json::from_str(r#"{"op":"read","id":1}"#).unwrap();
    assert!(matches!(r, Request::Read { lines: 200, .. }));
    assert!(serde_json::from_str::<Request>(r#"{"op":"send","id":1,"text":"x","rm":true}"#).is_err());
    assert!(serde_json::from_str::<Request>(r#"{"op":"format_disk"}"#).is_err());
}

#[test]
fn config_rejects_unknown_keys_and_has_sane_defaults() {
    let cfg = crate::config::Config::default();
    assert_eq!(cfg.theme, "Pro");
    assert!(cfg.scrollback > 0 && cfg.scrollback <= 200_000);
    assert!(toml::from_str::<crate::config::Config>("thme = \"Pro\"").is_err());
    let custom: crate::config::Config =
        toml::from_str("theme = \"Ocean\"\n[[profiles]]\nname = \"Claude\"\ncommand = [\"claude\"]\n").unwrap();
    assert_eq!(custom.profiles[0].command, vec!["claude"]);
}

#[cfg(unix)] // uses printf
#[test]
fn snapshot_renders_a_real_command() {
    let cfg = crate::config::Config::default();
    let out = std::env::temp_dir().join(format!("agentty-test-{}.png", std::process::id()));
    crate::snapshot::run(
        &cfg,
        crate::snapshot::Options {
            out: out.to_string_lossy().into(),
            theme: Some("Basic".into()),
            cols: 30,
            rows: 4,
            scale: 1.0,
            wait_ms: 300,
            max_ms: 5000,
            title: None,
            tabs: Vec::new(),
            command: vec!["printf".into(), "\\033[31mred\\033[0m ok".into()],
        },
    )
    .unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let _ = std::fs::remove_file(&out);
    assert_eq!(&bytes[1..4], b"PNG");
}

#[test]
fn wheel_goes_to_scrollback_arrows_or_mouse_reports() {
    use crate::input::{wheel, Wheel};
    // Normal screen: scrollback.
    assert_eq!(wheel(3, TermMode::empty(), 0, 0), Wheel::Scrollback(3));
    // Full-screen program without mouse reporting: arrow keys, even without ALTERNATE_SCROLL.
    assert_eq!(wheel(2, TermMode::ALT_SCREEN, 0, 0), Wheel::Bytes(b"\x1b[A\x1b[A".to_vec()));
    assert_eq!(wheel(-1, TermMode::ALT_SCREEN | TermMode::APP_CURSOR, 0, 0), Wheel::Bytes(b"\x1bOB".to_vec()));
    // Program asked for SGR mouse reporting: wheel-up is button 64 at the pointer cell (1-based).
    assert_eq!(
        wheel(1, TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE, 4, 9),
        Wheel::Bytes(b"\x1b[<64;5;10M".to_vec())
    );
    assert_eq!(wheel(-1, TermMode::MOUSE_REPORT_CLICK, 0, 0), Wheel::Bytes(vec![0x1b, b'[', b'M', 32 + 65, 33, 33]));
}
