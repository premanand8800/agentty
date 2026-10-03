#![allow(clippy::too_many_arguments)] // drawing helpers take a rect, a color and a scale
//! Box drawing (U+2500–U+257F) and block elements (U+2580–U+259F) drawn as shapes, not glyphs,
//! so lines join seamlessly across cells. Agent UIs (Claude Code's ╭─╮ panels) rely on this.

use crate::render::Frame;
use crate::theme::Rgb;

/// Segment weights for (up, right, down, left): 0 none, 1 light, 2 heavy, 3 double.
fn segments(c: char) -> Option<[u8; 4]> {
    let s = match c as u32 {
        0x2500 | 0x2504 | 0x2508 | 0x254C => [0, 1, 0, 1],
        0x2501 | 0x2505 | 0x2509 | 0x254D => [0, 2, 0, 2],
        0x2502 | 0x2506 | 0x250A | 0x254E => [1, 0, 1, 0],
        0x2503 | 0x2507 | 0x250B | 0x254F => [2, 0, 2, 0],
        0x250C => [0, 1, 1, 0],
        0x250D => [0, 2, 1, 0],
        0x250E => [0, 1, 2, 0],
        0x250F => [0, 2, 2, 0],
        0x2510 => [0, 0, 1, 1],
        0x2511 => [0, 0, 1, 2],
        0x2512 => [0, 0, 2, 1],
        0x2513 => [0, 0, 2, 2],
        0x2514 => [1, 1, 0, 0],
        0x2515 => [1, 2, 0, 0],
        0x2516 => [2, 1, 0, 0],
        0x2517 => [2, 2, 0, 0],
        0x2518 => [1, 0, 0, 1],
        0x2519 => [1, 0, 0, 2],
        0x251A => [2, 0, 0, 1],
        0x251B => [2, 0, 0, 2],
        0x251C => [1, 1, 1, 0],
        0x251D => [1, 2, 1, 0],
        0x251E => [2, 1, 1, 0],
        0x251F => [1, 1, 2, 0],
        0x2520 => [2, 1, 2, 0],
        0x2521 => [2, 2, 1, 0],
        0x2522 => [1, 2, 2, 0],
        0x2523 => [2, 2, 2, 0],
        0x2524 => [1, 0, 1, 1],
        0x2525 => [1, 0, 1, 2],
        0x2526 => [2, 0, 1, 1],
        0x2527 => [1, 0, 2, 1],
        0x2528 => [2, 0, 2, 1],
        0x2529 => [2, 0, 1, 2],
        0x252A => [1, 0, 2, 2],
        0x252B => [2, 0, 2, 2],
        0x252C => [0, 1, 1, 1],
        0x252D => [0, 1, 1, 2],
        0x252E => [0, 2, 1, 1],
        0x252F => [0, 2, 1, 2],
        0x2530 => [0, 1, 2, 1],
        0x2531 => [0, 1, 2, 2],
        0x2532 => [0, 2, 2, 1],
        0x2533 => [0, 2, 2, 2],
        0x2534 => [1, 1, 0, 1],
        0x2535 => [1, 1, 0, 2],
        0x2536 => [1, 2, 0, 1],
        0x2537 => [1, 2, 0, 2],
        0x2538 => [2, 1, 0, 1],
        0x2539 => [2, 1, 0, 2],
        0x253A => [2, 2, 0, 1],
        0x253B => [2, 2, 0, 2],
        0x253C => [1, 1, 1, 1],
        0x253D => [1, 1, 1, 2],
        0x253E => [1, 2, 1, 1],
        0x253F => [1, 2, 1, 2],
        0x2540 => [2, 1, 1, 1],
        0x2541 => [1, 1, 2, 1],
        0x2542 => [2, 1, 2, 1],
        0x2543 => [2, 1, 1, 2],
        0x2544 => [2, 2, 1, 1],
        0x2545 => [1, 1, 2, 2],
        0x2546 => [1, 2, 2, 1],
        0x2547 => [2, 2, 1, 2],
        0x2548 => [1, 2, 2, 2],
        0x2549 => [2, 1, 2, 2],
        0x254A => [2, 2, 2, 1],
        0x254B => [2, 2, 2, 2],
        0x2550 => [0, 3, 0, 3],
        0x2551 => [3, 0, 3, 0],
        0x2552 => [0, 3, 1, 0],
        0x2553 => [0, 1, 3, 0],
        0x2554 => [0, 3, 3, 0],
        0x2555 => [0, 0, 1, 3],
        0x2556 => [0, 0, 3, 1],
        0x2557 => [0, 0, 3, 3],
        0x2558 => [1, 3, 0, 0],
        0x2559 => [3, 1, 0, 0],
        0x255A => [3, 3, 0, 0],
        0x255B => [1, 0, 0, 3],
        0x255C => [3, 0, 0, 1],
        0x255D => [3, 0, 0, 3],
        0x255E => [1, 3, 1, 0],
        0x255F => [3, 1, 3, 0],
        0x2560 => [3, 3, 3, 0],
        0x2561 => [1, 0, 1, 3],
        0x2562 => [3, 0, 3, 1],
        0x2563 => [3, 0, 3, 3],
        0x2564 => [0, 3, 1, 3],
        0x2565 => [0, 1, 3, 1],
        0x2566 => [0, 3, 3, 3],
        0x2567 => [1, 3, 0, 3],
        0x2568 => [3, 1, 0, 1],
        0x2569 => [3, 3, 0, 3],
        0x256A => [1, 3, 1, 3],
        0x256B => [3, 1, 3, 1],
        0x256C => [3, 3, 3, 3],
        0x2574 => [0, 0, 0, 1],
        0x2575 => [1, 0, 0, 0],
        0x2576 => [0, 1, 0, 0],
        0x2577 => [0, 0, 1, 0],
        0x2578 => [0, 0, 0, 2],
        0x2579 => [2, 0, 0, 0],
        0x257A => [0, 2, 0, 0],
        0x257B => [0, 0, 2, 0],
        0x257C => [0, 2, 0, 1],
        0x257D => [1, 0, 2, 0],
        0x257E => [0, 1, 0, 2],
        0x257F => [2, 0, 1, 0],
        _ => return None,
    };
    Some(s)
}

/// Draw `c` into the cell at (x, y). Returns false if `c` is not a box/block character.
pub fn draw(f: &mut Frame, c: char, x: usize, y: usize, w: usize, h: usize, color: Rgb, scale: f32) -> bool {
    let code = c as u32;
    if !(0x2500..=0x259F).contains(&code) {
        return false;
    }
    let light = (scale.round() as usize).max(1);
    let (x, y) = (x as isize, y as isize);
    if let Some(seg) = segments(c) {
        lines(f, seg, x, y, w, h, light, color);
        return true;
    }
    match code {
        0x256D..=0x2570 => {
            arc(f, code, x, y, w, h, light as f32, color);
            true
        }
        0x2571..=0x2573 => {
            if code != 0x2572 {
                diagonal(f, x, y, w, h, light as f32, color, true);
            }
            if code != 0x2571 {
                diagonal(f, x, y, w, h, light as f32, color, false);
            }
            true
        }
        0x2580..=0x259F => {
            block(f, code, x, y, w, h, color);
            true
        }
        _ => false,
    }
}

fn lines(f: &mut Frame, seg: [u8; 4], x: isize, y: isize, w: usize, h: usize, light: usize, color: Rgb) {
    let (cx, cy) = (x + w as isize / 2, y + h as isize / 2);
    let thick = |weight: u8| if weight == 2 { light * 2 + 1 } else { light };
    let gap = light as isize + 1; // half-distance between the two strokes of a double line
                                  // The widest vertical stroke decides how far horizontal strokes reach into the center, and vice versa.
    let vmax = seg[0].max(seg[2]);
    let hmax = seg[1].max(seg[3]);
    let reach_h = if vmax == 3 { gap + light as isize } else { thick(vmax) as isize / 2 + 1 };
    let reach_v = if hmax == 3 { gap + light as isize } else { thick(hmax) as isize / 2 + 1 };
    let mut horiz = |x0: isize, x1: isize, weight: u8| {
        if weight == 3 {
            for off in [-gap, gap] {
                f.fill(x0, cy + off - light as isize / 2, (x1 - x0) as usize, light, color);
            }
        } else if weight > 0 {
            let t = thick(weight);
            f.fill(x0, cy - t as isize / 2, (x1 - x0) as usize, t, color);
        }
    };
    horiz(x, cx + reach_h, seg[3]);
    horiz(cx - reach_h, x + w as isize, seg[1]);
    let mut vert = |y0: isize, y1: isize, weight: u8| {
        if weight == 3 {
            for off in [-gap, gap] {
                f.fill(cx + off - light as isize / 2, y0, light, (y1 - y0) as usize, color);
            }
        } else if weight > 0 {
            let t = thick(weight);
            f.fill(cx - t as isize / 2, y0, t, (y1 - y0) as usize, color);
        }
    };
    vert(y, cy + reach_v, seg[0]);
    vert(cy - reach_v, y + h as isize, seg[2]);
}

/// Rounded corners ╭ ╮ ╯ ╰.
fn arc(f: &mut Frame, code: u32, x: isize, y: isize, w: usize, h: usize, t: f32, color: Rgb) {
    let (cx, cy) = (x as f32 + (w / 2) as f32 + 0.5 * (t % 2.0), y as f32 + (h / 2) as f32 + 0.5 * (t % 2.0));
    // Direction of the two arms: (horizontal, vertical); +1 = right/down.
    let (hd, vd) = match code {
        0x256D => (1.0, 1.0),   // ╭ right + down
        0x256E => (-1.0, 1.0),  // ╮ left + down
        0x256F => (-1.0, -1.0), // ╯ left + up
        _ => (1.0, -1.0),       // ╰ right + up
    };
    let r = (w.min(h) as f32 / 2.0).max(2.0);
    let (ax, ay) = (cx + hd * r, cy + vd * r); // arc center
                                               // Straight arms from the end of the arc to the cell edges.
    let ti = t as usize;
    if hd > 0.0 {
        f.fill(ax as isize, (cy - t / 2.0).round() as isize, (x + w as isize - ax as isize).max(0) as usize, ti, color);
    } else {
        f.fill(x, (cy - t / 2.0).round() as isize, (ax as isize - x).max(0) as usize, ti, color);
    }
    if vd > 0.0 {
        f.fill((cx - t / 2.0).round() as isize, ay as isize, ti, (y + h as isize - ay as isize).max(0) as usize, color);
    } else {
        f.fill((cx - t / 2.0).round() as isize, y, ti, (ay as isize - y).max(0) as usize, color);
    }
    // Quarter circle facing the cell center.
    let (x0, x1) = ((cx.min(ax) - t).floor() as isize, (cx.max(ax) + t).ceil() as isize);
    let (y0, y1) = ((cy.min(ay) - t).floor() as isize, (cy.max(ay) + t).ceil() as isize);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            if (fx - ax) * hd > 0.5 || (fy - ay) * vd > 0.5 {
                continue;
            }
            let d = ((fx - ax).powi(2) + (fy - ay).powi(2)).sqrt();
            let a = (t / 2.0 + 0.5 - (d - r).abs()).clamp(0.0, 1.0);
            f.blend_pub(px, py, color, (a * 255.0) as u8);
        }
    }
}

fn diagonal(f: &mut Frame, x: isize, y: isize, w: usize, h: usize, t: f32, color: Rgb, rising: bool) {
    let (fw, fh) = (w as f32, h as f32);
    let len = (fw * fw + fh * fh).sqrt();
    for py in 0..h {
        for px in 0..w {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let fy = if rising { fh - fy } else { fy };
            let d = (fh * fx - fw * fy).abs() / len;
            let a = (t / 2.0 + 0.5 - d).clamp(0.0, 1.0);
            f.blend_pub(x + px as isize, y + py as isize, color, (a * 255.0) as u8);
        }
    }
}

fn block(f: &mut Frame, code: u32, x: isize, y: isize, w: usize, h: usize, color: Rgb) {
    let eighth_h = |n: usize| h * n / 8;
    let eighth_w = |n: usize| w * n / 8;
    let (hw, hh) = (w / 2, h / 2);
    match code {
        0x2580 => f.fill(x, y, w, hh, color),
        0x2581..=0x2587 => {
            let n = eighth_h((code - 0x2580) as usize);
            f.fill(x, y + (h - n) as isize, w, n, color);
        }
        0x2588 => f.fill(x, y, w, h, color),
        0x2589..=0x258F => f.fill(x, y, eighth_w((0x2590 - code) as usize), h, color),
        0x2590 => f.fill(x + hw as isize, y, w - hw, h, color),
        0x2591..=0x2593 => {
            let a = [64u8, 128, 192][(code - 0x2591) as usize];
            for py in 0..h {
                for px in 0..w {
                    f.blend_pub(x + px as isize, y + py as isize, color, a);
                }
            }
        }
        0x2594 => f.fill(x, y, w, eighth_h(1).max(1), color),
        0x2595 => {
            let n = eighth_w(1).max(1);
            f.fill(x + (w - n) as isize, y, n, h, color);
        }
        0x2596..=0x259F => {
            // Quadrants: upper-left, upper-right, lower-left, lower-right.
            let q: [bool; 4] = match code {
                0x2596 => [false, false, true, false],
                0x2597 => [false, false, false, true],
                0x2598 => [true, false, false, false],
                0x2599 => [true, false, true, true],
                0x259A => [true, false, false, true],
                0x259B => [true, true, true, false],
                0x259C => [true, true, false, true],
                0x259D => [false, true, false, false],
                0x259E => [false, true, true, false],
                _ => [false, true, true, true],
            };
            let rects = [(0, 0, hw, hh), (hw, 0, w - hw, hh), (0, hh, hw, h - hh), (hw, hh, w - hw, h - hh)];
            for (on, (rx, ry, rw, rh)) in q.iter().zip(rects) {
                if *on {
                    f.fill(x + rx as isize, y + ry as isize, rw, rh, color);
                }
            }
        }
        _ => {}
    }
}
