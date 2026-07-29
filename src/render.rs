//! Rasterises the crosshair.
//!
//! Deliberately platform-independent: it takes a config and a surface size and
//! hands back pixels. Both the Wayland and Windows backends present the exact
//! same buffer, so the crosshair looks identical everywhere and this file is the
//! one part of the drawing path that can be unit-tested.

use crate::config::{Config, Rgba, Style, parse_color};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Scales a colour's alpha by the overall opacity setting.
fn paint_for(color: Rgba, opacity: f32, anti_alias: bool) -> Paint<'static> {
    let mut paint = Paint::default();
    let a = (color[3] as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
    paint.set_color_rgba8(color[0], color[1], color[2], a);
    paint.anti_alias = anti_alias;
    paint
}

/// The four arm directions, as (dx, dy) unit vectors.
const UP: (f32, f32) = (0.0, -1.0);
const RIGHT: (f32, f32) = (1.0, 0.0);
const DOWN: (f32, f32) = (0.0, 1.0);
const LEFT: (f32, f32) = (-1.0, 0.0);

fn arms_for(style: Style) -> &'static [(f32, f32)] {
    match style {
        Style::Cross => &[UP, RIGHT, DOWN, LEFT],
        // A T: no upper arm, so the shot line isn't blocked from above.
        Style::TCross => &[RIGHT, DOWN, LEFT],
        Style::Circle | Style::Dot => &[],
    }
}

/// Rect for one arm, inflated by `grow` on every side (used for the outline).
fn arm_rect(
    dir: (f32, f32),
    cx: f32,
    cy: f32,
    len: f32,
    th: f32,
    gap: f32,
    grow: f32,
) -> Option<Rect> {
    let vertical = dir.0 == 0.0;
    let (w, h) = if vertical { (th, len) } else { (len, th) };
    let x = match dir.0 {
        d if d > 0.0 => cx + gap,
        d if d < 0.0 => cx - gap - len,
        _ => cx - (th / 2.0).round(),
    };
    let y = match dir.1 {
        d if d > 0.0 => cy + gap,
        d if d < 0.0 => cy - gap - len,
        _ => cy - (th / 2.0).round(),
    };
    Rect::from_xywh(x - grow, y - grow, w + grow * 2.0, h + grow * 2.0)
}

/// Draws the crosshair centred in a `width` x `height` surface.
///
/// Returns a transparent pixmap when the crosshair is disabled, so toggling
/// costs one redraw rather than tearing the window down and rebuilding it.
pub fn draw(cfg: &Config, width: u32, height: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(width.max(1), height.max(1)).expect("non-zero pixmap");
    if !cfg.enabled {
        return pixmap;
    }

    let color = parse_color(&cfg.color).unwrap_or([0, 255, 0, 255]);
    let outline_color = parse_color(&cfg.outline_color).unwrap_or([0, 0, 0, 255]);

    let cx = (width as f32 / 2.0 + cfg.offset_x).round();
    let cy = (height as f32 / 2.0 + cfg.offset_y).round();
    let len = cfg.size.max(0.0).round();
    let th = cfg.thickness.max(1.0).round();
    let gap = cfg.gap.max(0.0).round();
    let ol = cfg.outline.max(0.0).round();

    let main = paint_for(color, cfg.opacity, false);
    let edge = paint_for(outline_color, cfg.opacity, false);
    let main_aa = paint_for(color, cfg.opacity, true);
    let edge_aa = paint_for(outline_color, cfg.opacity, true);
    let id = Transform::identity();

    // Arms. Every shape is drawn outline-first so the border sits behind.
    if len > 0.0 {
        for &dir in arms_for(cfg.style) {
            if ol > 0.0
                && let Some(r) = arm_rect(dir, cx, cy, len, th, gap, ol)
            {
                pixmap.fill_rect(r, &edge, id, None);
            }
            if let Some(r) = arm_rect(dir, cx, cy, len, th, gap, 0.0) {
                pixmap.fill_rect(r, &main, id, None);
            }
        }
    }

    // Ring.
    if cfg.style == Style::Circle
        && len > 0.0
        && let Some(path) = PathBuilder::from_circle(cx, cy, len)
    {
        if ol > 0.0 {
            let stroke = Stroke {
                width: th + ol * 2.0,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &edge_aa, &stroke, id, None);
        }
        let stroke = Stroke {
            width: th,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &main_aa, &stroke, id, None);
    }

    // Centre dot. Always present for the dot style, optional for the others.
    let dot_r = if cfg.style == Style::Dot {
        cfg.dot.max(th)
    } else {
        cfg.dot
    };
    if dot_r > 0.0 {
        if ol > 0.0
            && let Some(path) = PathBuilder::from_circle(cx, cy, dot_r + ol)
        {
            pixmap.fill_path(&path, &edge_aa, FillRule::Winding, id, None);
        }
        if let Some(path) = PathBuilder::from_circle(cx, cy, dot_r) {
            pixmap.fill_path(&path, &main_aa, FillRule::Winding, id, None);
        }
    }

    pixmap
}

// ------------------------------------------------------------------- hints --

const HINT_TEXT_PX: f32 = 14.0;
const HINT_PAD_X: f32 = 16.0;
const HINT_PAD_Y: f32 = 11.0;
const HINT_BG: [u8; 4] = [18, 21, 27, 238];
const HINT_BORDER: [u8; 4] = [255, 255, 255, 51];
const HINT_HEADING: [u8; 4] = [255, 255, 255, 255];
const HINT_BODY: [u8; 4] = [168, 178, 193, 255];

/// Draws the hint panel below the crosshair.
///
/// Deliberately drawn at full strength rather than through `cfg.opacity` — the
/// panel is what tells you how to raise the opacity again, so fading it with the
/// crosshair would make a dimmed crosshair impossible to recover from.
pub fn draw_hint(pixmap: &mut Pixmap, cfg: &Config, text: &str) {
    if text.is_empty() {
        return;
    }

    let lines: Vec<&str> = text.lines().collect();
    let line_h = crate::text::line_height(HINT_TEXT_PX);
    let widest = lines
        .iter()
        .map(|l| crate::text::measure(l, HINT_TEXT_PX))
        .fold(0.0_f32, f32::max);

    let box_w = widest + HINT_PAD_X * 2.0;
    let box_h = line_h * lines.len() as f32 + HINT_PAD_Y * 2.0;

    let cx = (pixmap.width() as f32 / 2.0 + cfg.offset_x).round();
    let cy = (pixmap.height() as f32 / 2.0 + cfg.offset_y).round();

    // Clear of the crosshair, and clamped so it stays on screen.
    let below = cfg.size.max(0.0) + cfg.gap.max(0.0) + 34.0;
    let box_x = (cx - box_w / 2.0)
        .max(8.0)
        .min(pixmap.width() as f32 - box_w - 8.0);
    let box_y = (cy + below.max(48.0)).min(pixmap.height() as f32 - box_h - 8.0);

    let id = Transform::identity();
    if let Some(rect) = Rect::from_xywh(box_x, box_y, box_w, box_h) {
        let mut bg = Paint::default();
        bg.set_color_rgba8(HINT_BG[0], HINT_BG[1], HINT_BG[2], HINT_BG[3]);
        bg.anti_alias = false;
        pixmap.fill_rect(rect, &bg, id, None);

        let mut border = Paint::default();
        border.set_color_rgba8(
            HINT_BORDER[0],
            HINT_BORDER[1],
            HINT_BORDER[2],
            HINT_BORDER[3],
        );
        border.anti_alias = false;
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&PathBuilder::from_rect(rect), &border, &stroke, id, None);
    }

    let ascent = crate::text::ascent(HINT_TEXT_PX);
    for (i, line) in lines.iter().enumerate() {
        // The first line is the heading — "Crosshair on", or the setting that
        // just changed.
        let color = if i == 0 { HINT_HEADING } else { HINT_BODY };
        let baseline = box_y + HINT_PAD_Y + line_h * i as f32 + ascent;
        crate::text::draw_on(
            pixmap,
            line,
            box_x + HINT_PAD_X,
            baseline,
            HINT_TEXT_PX,
            color,
        );
    }
}

/// tiny-skia hands back premultiplied RGBA; Wayland's ARGB8888 and Windows'
/// `UpdateLayeredWindow` both want premultiplied BGRA. One swap serves both.
pub fn to_bgra(pixmap: &Pixmap) -> Vec<u8> {
    let mut out = pixmap.data().to_vec();
    for px in out.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config {
            size: 10.0,
            thickness: 2.0,
            gap: 4.0,
            outline: 1.0,
            ..Default::default()
        }
    }

    /// Premultiplied alpha means a transparent pixel is all zeroes.
    fn opaque_pixels(p: &Pixmap) -> usize {
        p.data().chunks_exact(4).filter(|px| px[3] > 0).count()
    }

    #[test]
    fn disabled_draws_nothing() {
        let c = Config {
            enabled: false,
            ..cfg()
        };
        assert_eq!(opaque_pixels(&draw(&c, 200, 200)), 0);
    }

    #[test]
    fn centre_gap_stays_empty() {
        let p = draw(&cfg(), 200, 200);
        // Dead centre falls inside the gap, so nothing should be drawn there.
        let i = (100 * 200 + 100) * 4;
        assert_eq!(
            p.data()[i + 3],
            0,
            "centre of a gapped crosshair must be clear"
        );
    }

    #[test]
    fn dot_style_fills_the_centre() {
        let c = Config {
            style: Style::Dot,
            dot: 3.0,
            ..cfg()
        };
        let p = draw(&c, 200, 200);
        let i = (100 * 200 + 100) * 4;
        assert!(p.data()[i + 3] > 0, "dot style must paint the centre");
    }

    #[test]
    fn tcross_omits_the_upper_arm() {
        let full = draw(&cfg(), 200, 200);
        let t = draw(
            &Config {
                style: Style::TCross,
                ..cfg()
            },
            200,
            200,
        );
        assert!(opaque_pixels(&t) < opaque_pixels(&full));
        // Directly above the centre is clear for a T but painted for a cross.
        let above = (88 * 200 + 100) * 4;
        assert_eq!(t.data()[above + 3], 0);
        assert!(full.data()[above + 3] > 0);
    }

    #[test]
    fn opacity_scales_alpha() {
        let solid = draw(&cfg(), 200, 200);
        let faint = draw(
            &Config {
                opacity: 0.5,
                ..cfg()
            },
            200,
            200,
        );
        let peak = |p: &Pixmap| p.data().chunks_exact(4).map(|px| px[3]).max().unwrap();
        assert!(peak(&faint) < peak(&solid));
    }

    #[test]
    fn offset_moves_the_centre() {
        let p = draw(
            &Config {
                offset_y: -20.0,
                ..cfg()
            },
            200,
            200,
        );
        let at = |x: usize, y: usize| p.data()[(y * 200 + x) * 4 + 3];
        // The gap has moved up with the crosshair.
        assert_eq!(at(100, 80), 0);
        assert!(at(100, 68) > 0, "arm should now sit above the old centre");
    }

    #[test]
    fn bgra_swaps_red_and_blue() {
        let c = Config {
            color: "#ff0000".into(),
            outline: 0.0,
            ..cfg()
        };
        let p = draw(&c, 64, 64);
        let rgba = p.data();
        let bgra = to_bgra(&p);
        let i = rgba.chunks_exact(4).position(|px| px[3] > 0).unwrap() * 4;
        assert_eq!(rgba[i], bgra[i + 2]);
        assert_eq!(rgba[i + 2], bgra[i]);
    }
}
