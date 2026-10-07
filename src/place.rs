//! Screen rects of workspaces and of their title bars and windows, with the window gap applied.

use crate::{config::Config, rect::Rect, view::View};

fn snapped(r: Rect, config: &Config) -> Rect {
    if config.pixel_snap { r.snap() } else { r }
}

/// The screen width outside the world rect of each workspace: half the window gap, by which the
/// workspace is extended, and the workspace border.
pub fn outside(config: &Config) -> f64 {
    config.window.gap / 2.0 + config.workspace.border
}

/// The height of the band above each workspace for its label, in screen pixels.
pub fn label_band(config: &Config) -> f64 {
    if config.workspace.show_names { 1.25 * config.workspace.font_size } else { 0.0 }
}

/// The corner radius of workspaces: `workspace.radius` applies to the outer edge of their border,
/// which lies outside the workspace.
pub fn workspace_radius(config: &Config) -> f64 {
    config.workspace.radius - config.workspace.border
}

/// The screen rect of world rect `r` of a workspace, extended by half the window gap on each side.
pub fn workspace_rect(r: Rect, view: &View, config: &Config) -> Rect {
    snapped(view.to_screen(r).inset(-config.window.gap / 2.0), config)
}

/// The screen rect of world rect `r` of an element in tile `tile`: the tile is reduced by half the
/// window gap on each side, by at most a quarter of its size, and `r` is scaled with it.
pub fn element_rect(r: Rect, tile: Rect, view: &View, config: &Config) -> Rect {
    let t = view.to_screen(tile);
    let g = (config.window.gap / 2.0).min(t.width / 4.0).min(t.height / 4.0);
    let s = t.inset(g);
    let (sx, sy) = (s.width / t.width, s.height / t.height);
    let r = view.to_screen(r);
    let r = Rect { x: s.x + (r.x - t.x) * sx, y: s.y + (r.y - t.y) * sy, width: r.width * sx, height: r.height * sy };
    snapped(r, config)
}

/// The screen area inside the border of the window with frame `frame` in tile `tile`.
pub fn window_area(frame: Rect, tile: Rect, view: &View, config: &Config) -> Rect {
    element_rect(frame, tile, view, config).inset(config.window.border)
}
