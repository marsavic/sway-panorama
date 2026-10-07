use cosmic_text::{Align, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use tiny_skia::PixmapMut;

use crate::{color::Color, config::Config, rect::Rect};

/// Fonts and rasterized glyphs, for laying out and drawing text.
pub struct Text {
    fonts: FontSystem,
    glyphs: SwashCache,
}

impl Text {
    pub fn new() -> Self {
        Text { fonts: FontSystem::new(), glyphs: SwashCache::new() }
    }

    /// Lays out `s` in font family `font`, else the system sans-serif font, at font size `size`.
    /// With a width, lines are centered in it, and wrapped to it if `wrap`. Returns the buffer and
    /// the width and height of the text.
    pub fn layout(
        &mut self,
        s: &str,
        font: Option<&str>,
        size: f32,
        width: Option<f64>,
        wrap: bool,
    ) -> (Buffer, f64, f64) {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.2));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(width.map(|w| w as f32), None);
        let attrs = match font {
            Some(f) => Attrs::new().family(Family::Name(f)),
            None => Attrs::new(),
        };
        buffer.set_text(s, &attrs, Shaping::Advanced, width.map(|_| Align::Center));
        buffer.shape_until_scroll(&mut self.fonts, false);
        let w = buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max) as f64;
        let h = buffer.layout_runs().count() as f64 * size as f64 * 1.2;
        (buffer, w, h)
    }

    /// Draws laid out text with its top left corner at (`x`, `y`), clipped to `clip` at whole
    /// pixels. Glyphs are placed at quarter pixels horizontally; cosmic-text places them at whole
    /// pixels vertically.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pm: &mut PixmapMut,
        buffer: &mut Buffer,
        x: f64,
        y: f64,
        config: &Config,
        color: Color,
        clip: Rect,
    ) {
        let [r, g, b, a] = color.0;
        let color = cosmic_text::Color::rgba(r, g, b, a);
        let (x, y) = if config.pixel_snap { (x.round(), y.round()) } else { (x, y) };
        let (x0, y0) = (clip.x.max(0.0).round() as i32, clip.y.max(0.0).round() as i32);
        let x1 = ((clip.x + clip.width).round() as i32).min(pm.width() as i32);
        let y1 = ((clip.y + clip.height).round() as i32).min(pm.height() as i32);
        let stride = pm.width() as usize;
        let data = pm.data_mut();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let physical = glyph.physical((x as f32, y as f32 + run.line_y), 1.0);
                let color = glyph.color_opt.unwrap_or(color);
                self.glyphs.with_pixels(&mut self.fonts, physical.cache_key, color, |gx, gy, c| {
                    let (px, py) = (physical.x + gx, physical.y + gy);
                    let a = c.a() as u32;
                    if a == 0 || px < x0 || px >= x1 || py < y0 || py >= y1 {
                        return;
                    }
                    let i = (py as usize * stride + px as usize) * 4;
                    for (d, s) in data[i..i + 4].iter_mut().zip([c.r(), c.g(), c.b(), 255]) {
                        *d = ((s as u32 * a + *d as u32 * (255 - a)) / 255) as u8;
                    }
                });
            }
        }
    }
}
