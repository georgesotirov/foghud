//! Just enough text rendering for the hint panel.
//!
//! tiny-skia rasterises shapes but has no notion of glyphs, so fontdue turns
//! characters into coverage bitmaps and we alpha-blend them into the pixmap
//! ourselves. The font is embedded rather than looked up on the system: an
//! overlay that silently loses its labels because a machine has no monospace
//! font configured would be worse than 340KB of binary.

use fontdue::{Font, FontSettings};
use std::sync::OnceLock;
use tiny_skia::{Pixmap, PixmapMut};

const FONT_DATA: &[u8] = include_bytes!("../assets/DejaVuSansMono.ttf");

fn font() -> &'static Font {
    static FONT: OnceLock<Font> = OnceLock::new();
    FONT.get_or_init(|| {
        Font::from_bytes(FONT_DATA, FontSettings::default()).expect("embedded font is valid")
    })
}

/// Width in pixels of a single character cell. The font is monospaced, so one
/// measurement describes every column.
pub fn advance(px: f32) -> f32 {
    font().metrics('M', px).advance_width
}

/// Distance between baselines.
pub fn line_height(px: f32) -> f32 {
    let m = font().horizontal_line_metrics(px).expect("scalable font");
    m.ascent - m.descent + m.line_gap
}

pub fn ascent(px: f32) -> f32 {
    font()
        .horizontal_line_metrics(px)
        .expect("scalable font")
        .ascent
}

pub fn measure(text: &str, px: f32) -> f32 {
    text.chars().count() as f32 * advance(px)
}

/// Draws `text` with its left edge at `x` and its baseline at `y`.
pub fn draw(target: &mut PixmapMut, text: &str, x: f32, y: f32, px: f32, color: [u8; 4]) {
    let font = font();
    let mut pen = x;
    let target_width = target.width() as i32;
    let target_height = target.height() as i32;

    for ch in text.chars() {
        let (metrics, coverage) = font.rasterize(ch, px);

        // fontdue gives bitmaps bottom-up relative to the baseline via ymin.
        let glyph_x = (pen + metrics.xmin as f32).round() as i32;
        let glyph_y = (y - metrics.height as f32 - metrics.ymin as f32).round() as i32;

        for row in 0..metrics.height {
            let py = glyph_y + row as i32;
            if py < 0 || py >= target_height {
                continue;
            }
            for col in 0..metrics.width {
                let px_x = glyph_x + col as i32;
                if px_x < 0 || px_x >= target_width {
                    continue;
                }
                let alpha = coverage[row * metrics.width + col];
                if alpha == 0 {
                    continue;
                }
                blend(target, px_x, py, color, alpha);
            }
        }

        pen += metrics.advance_width;
    }
}

/// Source-over blend of one premultiplied pixel.
fn blend(target: &mut PixmapMut, x: i32, y: i32, color: [u8; 4], coverage: u8) {
    let width = target.width() as usize;
    let index = (y as usize * width + x as usize) * 4;
    let data = target.data_mut();

    let a = (color[3] as u32 * coverage as u32) / 255;
    if a == 0 {
        return;
    }
    let inv = 255 - a;

    for (offset, channel) in [color[0], color[1], color[2]].into_iter().enumerate() {
        let src = (channel as u32 * a) / 255; // premultiply
        let dst = data[index + offset] as u32;
        data[index + offset] = (src + (dst * inv) / 255) as u8;
    }
    let dst_a = data[index + 3] as u32;
    data[index + 3] = (a + (dst_a * inv) / 255) as u8;
}

/// Convenience wrapper for drawing straight into an owned pixmap.
pub fn draw_on(target: &mut Pixmap, text: &str, x: f32, y: f32, px: f32, color: [u8; 4]) {
    draw(&mut target.as_mut(), text, x, y, px, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_loads_and_is_monospaced() {
        let px = 14.0;
        assert!(advance(px) > 0.0);
        // Every glyph occupies the same cell in a monospaced face.
        assert_eq!(
            font().metrics('i', px).advance_width,
            font().metrics('W', px).advance_width
        );
    }

    #[test]
    fn measure_scales_with_length() {
        assert!(measure("F1", 14.0) < measure("F1  style", 14.0));
        assert_eq!(measure("", 14.0), 0.0);
    }

    #[test]
    fn drawing_marks_pixels() {
        let mut pixmap = Pixmap::new(120, 40).unwrap();
        draw_on(&mut pixmap, "F1", 4.0, 24.0, 14.0, [255, 255, 255, 255]);
        let painted = pixmap.data().chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(painted > 0, "text should mark pixels");
    }

    #[test]
    fn drawing_off_canvas_does_not_panic() {
        let mut pixmap = Pixmap::new(32, 16).unwrap();
        draw_on(
            &mut pixmap,
            "clipped",
            -50.0,
            8.0,
            14.0,
            [255, 255, 255, 255],
        );
        draw_on(
            &mut pixmap,
            "clipped",
            500.0,
            8.0,
            14.0,
            [255, 255, 255, 255],
        );
        draw_on(
            &mut pixmap,
            "clipped",
            4.0,
            -40.0,
            14.0,
            [255, 255, 255, 255],
        );
    }
}
