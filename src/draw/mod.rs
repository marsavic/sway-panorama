//! Drawing of the workspace overviews with tiny-skia.

mod canvas;
mod icons;
mod shape;
mod text;

use cosmic_text::Buffer;
use tiny_skia::{FilterQuality, Paint, Pattern, PixmapMut, SpreadMode, Transform};

pub use canvas::Canvas;
use icons::IconCache;
use shape::{border, fill, mask, sk, sk_rect};
use text::Text;

use crate::{
    color::Color,
    config::{AppColors, Config, Icons, Titles},
    model::{Element, Kind, Overview, Window},
    place::{element_rect, label_band, workspace_radius, workspace_rect},
    rect::Rect,
    view::View,
};

/// Draws the workspace overviews: workspaces, title bars and windows, with text and icons.
pub struct Painter {
    text: Text,
    icons: IconCache,
}

/// The smallest font size that shrinking reduces a title to, in pixels.
const MIN_TITLE_SIZE: f32 = 5.0;

impl Painter {
    pub fn new(config: &Config) -> Self {
        Painter { text: Text::new(), icons: IconCache::new(config.window.icon_theme.clone()) }
    }

    pub fn set_icon_theme(&mut self, theme: Option<String>) {
        self.icons = IconCache::new(theme);
    }

    /// Paints the background, the workspace overviews and their tiling layers, and also the
    /// floating layers if `floating` is true.
    pub fn paint(&mut self, pm: &mut PixmapMut, overviews: &[Overview], view: &View, config: &Config, floating: bool) {
        let c = &config.workspace.colors;
        let screen = Rect { x: 0.0, y: 0.0, width: pm.width() as f64, height: pm.height() as f64 };
        pm.fill(sk(config.background));
        for ov in overviews {
            let r = workspace_rect(ov.rect, view, config);
            let outer = r.inset(-config.workspace.border);
            if config.workspace.show_names {
                let ws = &config.workspace;
                let (mut buffer, _, h) = self.text.layout(&ov.name, ws.font.as_deref(), ws.font_size as f32, None, false);
                let band = label_band(config);
                self.text.draw(pm, &mut buffer, outer.x, outer.y - band + (band - h) / 2.0, config, c.text, screen);
            }
            let radius = workspace_radius(config);
            fill(pm, r, radius, c.bar, None);
            fill(pm, workspace_rect(ov.usable, view, config), radius, c.fill, None);
            let b = &c.border;
            let state = if ov.urgent {
                b.urgent
            } else if ov.focused {
                b.focused
            } else if ov.visible {
                b.visible
            } else {
                b.normal
            };
            border(pm, outer, config.workspace.border, config.workspace.radius, state, None);
            let count = if floating { ov.layers.len() } else { 1 };
            for layer in &ov.layers[..count] {
                self.paint_layer(pm, layer, view, (0.0, 0.0), r, config);
            }
        }
    }

    /// Paints the elements of one layer, shifted by `-offset`, within the workspace with screen
    /// rect `workspace` and its rounded corners.
    pub fn paint_layer(
        &mut self,
        pm: &mut PixmapMut,
        layer: &[Element],
        view: &View,
        offset: (f64, f64),
        workspace: Rect,
        config: &Config,
    ) {
        let shift = |r: Rect| Rect { x: r.x - offset.0, y: r.y - offset.1, ..r };
        let workspace = shift(workspace);
        let bounds = Rect { x: 0.0, y: 0.0, width: pm.width() as f64, height: pm.height() as f64 };
        let Some(clip) = workspace.intersect(&bounds) else { return };
        let mask = mask(pm, workspace, workspace_radius(config));
        let (mask, radius) = (mask.as_ref(), config.window.radius);
        for e in layer {
            let r = shift(element_rect(e.rect, e.tile, view, config));
            if r.intersect(&clip).is_none() {
                continue;
            }
            // A title bar is filled with the border color of its window.
            let app = e.app.as_deref();
            let (color, edge) = match e.kind {
                Kind::Bar => (border_color(config, app, e.focused, e.urgent), config.window.colors.border.title_bar),
                Kind::Window(_) => (window_fill(config, app), border_color(config, app, e.focused, e.urgent)),
            };
            fill(pm, r, radius, color, mask);
            border(pm, r, config.window.border, radius, edge, mask);
            let inner = r.inset(config.window.border);
            let Some(inner_clip) = inner.intersect(&clip) else { continue };
            match &e.kind {
                Kind::Bar => {
                    let size = (inner.height * 0.7) as f32;
                    if config.window.titles != Titles::None && size >= MIN_TITLE_SIZE {
                        let text = title_text(config, &e.title, app);
                        let (mut buffer, _, h) = self.text.layout(text, config.window.font.as_deref(), size, None, false);
                        let x = inner.x + size as f64 * 0.3;
                        let y = inner.y + (inner.height - h) / 2.0;
                        self.text.draw(pm, &mut buffer, x, y, config, config.window.colors.text, inner_clip);
                    }
                }
                Kind::Window(w) => self.group(pm, e, w, inner, inner_clip, config),
            }
        }
    }

    /// Draws the icon and the title of window `w` of element `e` as one group centered in `area`,
    /// the area inside the window border, within `clip`. The icon keeps `icon_padding` and the
    /// title `title_padding` from the edges of `area`.
    fn group(&mut self, pm: &mut PixmapMut, e: &Element, w: &Window, area: Rect, clip: Rect, config: &Config) {
        let wc = &config.window;
        let (ip, tp) = (wc.icon_padding, wc.title_padding);
        let icon = wc.icon_size.min(area.width - 2.0 * ip).min(area.height - 2.0 * ip).min(256.0).floor();
        let shown = |app: &str| match wc.icons {
            Icons::All => true,
            Icons::Unlisted => !wc.colors.app_hues.contains_key(app),
            Icons::None => false,
        };
        let icon = match &e.app {
            Some(app) if shown(app) && icon >= 8.0 && self.icons.get(app, bucket(icon)).is_some() => icon,
            _ => 0.0,
        };
        let gap = if icon > 0.0 { wc.icon_title_gap } else { 0.0 };
        let top = if icon > 0.0 { ip } else { tp };
        let width = area.width - 2.0 * tp;
        let text = title_text(config, &e.title, e.app.as_deref());
        let title = if wc.titles != Titles::None && !w.bar && !text.is_empty() && width > 0.0 {
            self.fit_title(text, config, width, area.height - top - icon - gap - tp)
        } else {
            None
        };
        let bottom = if title.is_some() { tp } else { ip };
        let height = icon + title.as_ref().map_or(0.0, |(_, h)| gap + h);
        let y = area.y + top + ((area.height - top - bottom - height) / 2.0).max(0.0);
        if let Some(app) = &e.app
            && icon > 0.0
        {
            let (x, y) = (area.x + (area.width - icon) / 2.0, y);
            let (x, y) = if config.pixel_snap { (x.round(), y.round()) } else { (x, y) };
            self.icon(pm, app, icon, x, y, clip);
        }
        if let Some((mut buffer, _)) = title {
            self.text.draw(pm, &mut buffer, area.x + tp, y + icon + gap, config, wc.colors.text, clip);
        }
    }

    /// Lays out a window title in `width` and at most `height`, wrapping and shrinking it as
    /// configured. Returns `None` if shrinking cannot make it fit.
    fn fit_title(&mut self, s: &str, config: &Config, width: f64, height: f64) -> Option<(Buffer, f64)> {
        let (wrap, font) = (config.window.wrap_titles, config.window.font.as_deref());
        let fits = |w: f64, h: f64| h <= height && (wrap || w <= width);
        let size = config.window.font_size as f32;
        let (buffer, w, h) = self.text.layout(s, font, size, Some(width), wrap);
        if fits(w, h) || !config.window.shrink_titles {
            return Some((buffer, h));
        }
        // The largest size that fits, by bisection.
        let (mut lo, mut hi, mut best) = (MIN_TITLE_SIZE, size, None);
        for _ in 0..6 {
            let mid = (lo + hi) / 2.0;
            let (buffer, w, h) = self.text.layout(s, font, mid, Some(width), wrap);
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
}

/// The title shown for a window or title bar: the window title, or the app name if configured and
/// known.
fn title_text<'a>(config: &Config, title: &'a str, app: Option<&'a str>) -> &'a str {
    match app {
        Some(app) if config.window.titles == Titles::App => app,
        _ => title,
    }
}

/// The rasterization size of an icon drawn at `size` pixels.
fn bucket(size: f64) -> u32 {
    (size.ceil() as u32).next_power_of_two().clamp(16, 256)
}

/// The fill of a window of `app`: its app color if the app color mode includes the app, else gray
/// of the same lightness.
fn window_fill(config: &Config, app: Option<&str>) -> Color {
    let colors = &config.window.colors;
    match (config.window.app_colors, app) {
        (AppColors::All, Some(app)) => app_color(config, app),
        (AppColors::Listed, Some(app)) if colors.app_hues.contains_key(app) => app_color(config, app),
        _ => Color::from_oklab([colors.app_lightness, 0.0, 0.0], 255),
    }
}

/// The border color of a window of `app`, also the fill of its title bar. Unless the window is
/// urgent, it is the window fill with the configured OKLab lightness of the normal or focused
/// state and the same chroma and hue.
fn border_color(config: &Config, app: Option<&str>, focused: bool, urgent: bool) -> Color {
    let border = &config.window.colors.border;
    if urgent {
        return border.urgent;
    }
    let lightness = if focused { border.lightness_focused } else { border.lightness };
    let fill = window_fill(config, app);
    let [_, a, b] = fill.to_oklab();
    Color::from_oklab([lightness, a, b], fill.0[3])
}

/// The color of `app`, shared by its windows: an OKLCh color of the configured lightness and
/// chroma, whose hue is configured for the app, or else comes from a hash of the name.
fn app_color(config: &Config, app: &str) -> Color {
    let colors = &config.window.colors;
    let hue = colors.app_hues.get(app).copied().unwrap_or_else(|| {
        let hash = app.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193));
        hash as f64 / 2f64.powi(32) * 360.0
    });
    let hue = hue.to_radians();
    let (l, c) = (colors.app_lightness, colors.app_chroma);
    Color::from_oklab([l, c * hue.cos(), c * hue.sin()], 255)
}
