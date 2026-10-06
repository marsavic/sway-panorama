use cosmic_text::{Align, Attrs, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use smithay_client_toolkit::shm::{
    Shm,
    slot::{Buffer, SlotPool},
};
use tiny_skia::{FilterQuality, Paint, Pattern, PixmapMut, SpreadMode, Transform};
use wayland_client::protocol::{wl_shm, wl_surface::WlSurface};

use crate::{
    config::{Color, Config},
    icons::Icons,
    model::{Element, Overview, Window},
    oklab,
    sway::Rect,
    view::View,
};

/// An shm buffer to draw into, reused while the compositor does not hold it.
pub struct Canvas {
    pool: SlotPool,
    buffer: Option<Buffer>,
}

impl Canvas {
    pub fn new(shm: &Shm) -> Self {
        Canvas { pool: SlotPool::new(4096, shm).expect("create pool"), buffer: None }
    }

    /// Returns a pixmap of size `w`×`h` with undefined content.
    pub fn pixmap(&mut self, w: u32, h: u32) -> PixmapMut<'_> {
        let stride = w as i32 * 4;
        let reusable = self.buffer.as_ref().is_some_and(|b| {
            b.height() == h as i32 && b.stride() == stride && b.canvas(&mut self.pool).is_some()
        });
        if !reusable {
            let (buffer, _) =
                self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Abgr8888).expect("create buffer");
            self.buffer = Some(buffer);
        }
        let canvas = self.buffer.as_ref().unwrap().canvas(&mut self.pool).unwrap();
        PixmapMut::from_bytes(canvas, w, h).unwrap()
    }

    /// Attaches the buffer to `surface` and damages it whole.
    pub fn attach(&self, surface: &WlSurface) {
        if let Some(buffer) = &self.buffer {
            buffer.attach_to(surface).expect("buffer attach");
            surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        }
    }
}

pub struct Painter {
    fonts: FontSystem,
    glyphs: SwashCache,
    icons: Icons,
}

/// The smallest font size that shrinking reduces a title to, in pixels.
const MIN_TITLE_SIZE: f32 = 5.0;

/// The screen rect of world rect `r`, with edges at whole pixels if `pixel_snap` is set.
fn screen_rect(r: Rect, view: &View, config: &Config) -> Rect {
    let s = view.to_screen(r);
    if config.pixel_snap { s.snap() } else { s }
}

/// The screen area of workspace overview `ov`, where its windows are drawn.
pub fn workspace_area(ov: &Overview, view: &View, config: &Config, screen: Rect) -> Option<Rect> {
    screen_rect(ov.rect, view, config).intersect(&screen)
}

/// The screen area inside the border of the window with frame `frame`.
pub fn window_area(frame: Rect, view: &View, config: &Config) -> Rect {
    screen_rect(frame, view, config).inset(config.window_border)
}

impl Painter {
    pub fn new(config: &Config) -> Self {
        Painter { fonts: FontSystem::new(), glyphs: SwashCache::new(), icons: Icons::new(config.icon_theme.clone()) }
    }

    pub fn set_icon_theme(&mut self, theme: Option<String>) {
        self.icons = Icons::new(theme);
    }

    /// Paints the background, the workspace overviews and their tiling layers, and also the
    /// floating layers if `floating` is true.
    pub fn paint(&mut self, pm: &mut PixmapMut, overviews: &[Overview], view: &View, config: &Config, floating: bool) {
        let c = &config.colors;
        let screen = Rect { x: 0.0, y: 0.0, width: pm.width() as f64, height: pm.height() as f64 };
        pm.fill(sk(c.background, 1.0));
        for ov in overviews {
            let r = screen_rect(ov.rect, view, config);
            let alpha = if ov.exists { 1.0 } else { 0.4 };
            let outer = r.inset(-config.workspace_border);
            if config.show_workspace_names {
                let size = config.label_size as f32;
                let (mut buffer, _, h) = self.layout(&ov.name, config, size, None, false);
                let band = 1.25 * config.label_size;
                self.draw_text(pm, &mut buffer, outer.x, outer.y - band + (band - h) / 2.0, config, alpha, screen);
            }
            if ov.exists {
                fill(pm, r, c.bar, 1.0);
                fill(pm, screen_rect(ov.usable, view, config), c.workspace, 1.0);
            }
            let b = &c.workspace_border;
            let state = if ov.urgent {
                b.urgent
            } else if ov.focused {
                b.focused
            } else if ov.visible {
                b.visible
            } else {
                b.normal
            };
            border(pm, outer, config.workspace_border, state, alpha, screen);
            let Some(area) = workspace_area(ov, view, config, screen) else { continue };
            let count = if floating { ov.layers.len() } else { 1 };
            for layer in &ov.layers[..count] {
                self.paint_layer(pm, layer, view, (0.0, 0.0), area, config);
            }
        }
    }

    /// Paints the elements of one layer, shifted by `-offset`, within `clip`.
    pub fn paint_layer(
        &mut self,
        pm: &mut PixmapMut,
        layer: &[Element],
        view: &View,
        offset: (f64, f64),
        clip: Rect,
        config: &Config,
    ) {
        let c = &config.colors;
        let shift = |r: Rect| Rect { x: r.x - offset.0, y: r.y - offset.1, ..r };
        for el in layer {
            match el {
                Element::Bar(b) => {
                    let br = shift(screen_rect(b.rect, view, config));
                    let Some(visible) = br.intersect(&clip) else { continue };
                    fill(pm, visible, border_color(config, b.app.as_deref(), b.focused, b.urgent), 1.0);
                    border(pm, br, config.window_border, c.workspace, 1.0, clip);
                    let inner = br.inset(config.window_border);
                    let size = (inner.height * 0.7) as f32;
                    if config.show_titles
                        && size >= MIN_TITLE_SIZE
                        && let Some(text_clip) = inner.intersect(&clip)
                    {
                        let (mut buffer, _, h) = self.layout(&b.title, config, size, None, false);
                        let x = inner.x + size as f64 * 0.3;
                        self.draw_text(pm, &mut buffer, x, inner.y + (inner.height - h) / 2.0, config, 1.0, text_clip);
                    }
                }
                Element::Window(w) => {
                    let fr = shift(screen_rect(w.frame, view, config));
                    let Some(visible) = fr.intersect(&clip) else { continue };
                    fill(pm, visible, window_fill(config, w.app.as_deref()), 1.0);
                    let color = border_color(config, w.app.as_deref(), w.focused, w.urgent);
                    border(pm, fr, config.window_border, color, 1.0, clip);
                    let inner = shift(window_area(w.frame, view, config));
                    if let Some(inner_clip) = inner.intersect(&clip) {
                        self.group(pm, w, inner, inner_clip, config);
                    }
                }
            }
        }
    }

    /// Draws the icon and the title of window `w` as one group centered in `area`, within `clip`.
    fn group(&mut self, pm: &mut PixmapMut, w: &Window, area: Rect, clip: Rect, config: &Config) {
        let g = area.inset(2.0);
        if g.width <= 0.0 || g.height <= 0.0 {
            return;
        }
        let icon = config.icon_size.min(g.width).min(g.height).min(256.0).floor();
        let icon = match &w.app {
            Some(app) if config.show_icons && icon >= 8.0 && self.icons.get(app, bucket(icon)).is_some() => icon,
            _ => 0.0,
        };
        let gap = if icon > 0.0 { config.icon_title_gap } else { 0.0 };
        let title = if config.show_titles && !w.bar && !w.title.is_empty() {
            self.fit_title(&w.title, config, g.width, g.height - icon - gap)
        } else {
            None
        };
        let height = icon + title.as_ref().map_or(0.0, |(_, h)| gap + h);
        let y = g.y + ((g.height - height) / 2.0).max(0.0);
        if let Some(app) = &w.app
            && icon > 0.0
        {
            let (x, y) = (g.x + (g.width - icon) / 2.0, y);
            let (x, y) = if config.pixel_snap { (x.round(), y.round()) } else { (x, y) };
            self.icon(pm, app, icon, x, y, clip);
        }
        if let Some((mut buffer, _)) = title {
            self.draw_text(pm, &mut buffer, g.x, y + icon + gap, config, 1.0, clip);
        }
    }

    /// Lays out a window title in `width` and at most `height`, wrapping and shrinking it as
    /// configured. Returns `None` if shrinking cannot make it fit.
    fn fit_title(&mut self, s: &str, config: &Config, width: f64, height: f64) -> Option<(cosmic_text::Buffer, f64)> {
        let wrap = config.wrap_titles;
        let fits = |w: f64, h: f64| h <= height && (wrap || w <= width);
        let size = config.label_size as f32;
        let (buffer, w, h) = self.layout(s, config, size, Some(width), wrap);
        if fits(w, h) || !config.shrink_titles {
            return Some((buffer, h));
        }
        // The largest size that fits, by bisection.
        let (mut lo, mut hi, mut best) = (MIN_TITLE_SIZE, size, None);
        for _ in 0..6 {
            let mid = (lo + hi) / 2.0;
            let (buffer, w, h) = self.layout(s, config, mid, Some(width), wrap);
            if fits(w, h) {
                best = Some((buffer, h));
                lo = mid;
            } else {
                hi = mid;
            }
        }
        best
    }

    fn icon(&mut self, pm: &mut PixmapMut, app: &str, size: f64, x: f64, y: f64, clip: Rect) {
        let bucket = bucket(size);
        let (Some(icon), Some(visible)) = (self.icons.get(app, bucket), Rect { x, y, width: size, height: size }.intersect(&clip))
        else {
            return;
        };
        let s = (size / bucket as f64) as f32;
        let transform = Transform::from_row(s, 0.0, 0.0, s, x as f32, y as f32);
        let paint = Paint {
            shader: Pattern::new(icon.as_ref(), SpreadMode::Pad, FilterQuality::Bilinear, 1.0, transform),
            ..Default::default()
        };
        if let Some(rect) = sk_rect(visible) {
            pm.fill_rect(rect, &paint, Transform::identity(), None);
        }
    }

    /// Lays out `s` at font size `size`. With a width, lines are centered in it, and wrapped to
    /// it if `wrap`. Returns the buffer and the width and height of the text.
    fn layout(
        &mut self,
        s: &str,
        config: &Config,
        size: f32,
        width: Option<f64>,
        wrap: bool,
    ) -> (cosmic_text::Buffer, f64, f64) {
        let mut buffer = cosmic_text::Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.2));
        buffer.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
        buffer.set_size(width.map(|w| w as f32), None);
        let attrs = match config.font.as_deref() {
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
    fn draw_text(
        &mut self,
        pm: &mut PixmapMut,
        buffer: &mut cosmic_text::Buffer,
        x: f64,
        y: f64,
        config: &Config,
        alpha: f64,
        clip: Rect,
    ) {
        let [r, g, b, a] = config.colors.text.0;
        let color = cosmic_text::Color::rgba(r, g, b, (a as f64 * alpha) as u8);
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

/// The rasterization size of an icon drawn at `size` pixels.
fn bucket(size: f64) -> u32 {
    (size.ceil() as u32).next_power_of_two().clamp(16, 256)
}

fn sk(c: Color, alpha: f64) -> tiny_skia::Color {
    let [r, g, b, a] = c.0;
    tiny_skia::Color::from_rgba8(r, g, b, (a as f64 * alpha) as u8)
}

fn sk_rect(r: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.width as f32, r.height as f32)
}

fn fill(pm: &mut PixmapMut, r: Rect, c: Color, alpha: f64) {
    if let Some(rect) = sk_rect(r) {
        let mut paint = Paint::default();
        paint.set_color(sk(c, alpha));
        pm.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// Draws a border of `width` pixels inside the edges of `r`, clipped to `clip`.
fn border(pm: &mut PixmapMut, r: Rect, width: f64, c: Color, alpha: f64, clip: Rect) {
    let w = width.min(r.width / 2.0).min(r.height / 2.0);
    if w <= 0.0 {
        return;
    }
    let sides = [
        Rect { height: w, ..r },
        Rect { y: r.y + r.height - w, height: w, ..r },
        Rect { y: r.y + w, width: w, height: r.height - 2.0 * w, ..r },
        Rect { x: r.x + r.width - w, y: r.y + w, width: w, height: r.height - 2.0 * w },
    ];
    for side in sides {
        if let Some(s) = side.intersect(&clip) {
            fill(pm, s, c, alpha);
        }
    }
}

/// The fill of a window of `app`: its app color, or without app colors or app, gray of the same
/// lightness.
fn window_fill(config: &Config, app: Option<&str>) -> Color {
    match app {
        Some(app) if config.app_colors => app_color(config, app),
        _ => oklab::to_color([config.colors.app_lightness, 0.0, 0.0], 255),
    }
}

/// The border color of a window of `app`, also the fill of its title bar. In the normal state it
/// is the window fill with the configured OKLab lightness and the same chroma and hue.
fn border_color(config: &Config, app: Option<&str>, focused: bool, urgent: bool) -> Color {
    let b = &config.colors.window_border;
    if urgent {
        b.urgent
    } else if focused {
        b.focused
    } else {
        let fill = window_fill(config, app);
        let [_, a, b] = oklab::from_color(fill);
        oklab::to_color([config.colors.window_border.lightness, a, b], fill.0[3])
    }
}

/// A color derived from the app name, so that windows of one app share it: an OKLCh color of the
/// configured lightness and chroma, whose hue comes from a hash of the name plus the offset.
fn app_color(config: &Config, app: &str) -> Color {
    let hash = app.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
    let hue = (hash as f64 / 2f64.powi(32) * 360.0 + config.colors.app_hue_offset).to_radians();
    let (l, c) = (config.colors.app_lightness, config.colors.app_chroma);
    oklab::to_color([l, c * hue.cos(), c * hue.sin()], 255)
}
