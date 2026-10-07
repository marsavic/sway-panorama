//! Rects with rounded corners, filled, as borders, and as masks.

use tiny_skia::{FillRule, Mask, Paint, PathBuilder, PixmapMut, Transform};

use crate::{color::Color, rect::Rect};

pub fn sk(c: Color) -> tiny_skia::Color {
    let [r, g, b, a] = c.0;
    tiny_skia::Color::from_rgba8(r, g, b, a)
}

pub fn sk_rect(r: Rect) -> Option<tiny_skia::Rect> {
    tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.width as f32, r.height as f32)
}

/// Adds rect `r` with corners rounded to `radius`, at most half of its shorter side, to `pb`.
fn push_rounded(pb: &mut PathBuilder, r: Rect, radius: f64) {
    // A quarter circle as a cubic Bézier curve has its control points at this fraction of the
    // radius from the corner.
    const K: f32 = 1.0 - 0.552_284_7;
    let k = radius.min(r.width / 2.0).min(r.height / 2.0).max(0.0) as f32;
    let c = k * K;
    let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, (r.x + r.width) as f32, (r.y + r.height) as f32);
    pb.move_to(x0 + k, y0);
    pb.line_to(x1 - k, y0);
    pb.cubic_to(x1 - c, y0, x1, y0 + c, x1, y0 + k);
    pb.line_to(x1, y1 - k);
    pb.cubic_to(x1, y1 - c, x1 - c, y1, x1 - k, y1);
    pb.line_to(x0 + k, y1);
    pb.cubic_to(x0 + c, y1, x0, y1 - c, x0, y1 - k);
    pb.line_to(x0, y0 + k);
    pb.cubic_to(x0, y0 + c, x0 + c, y0, x0 + k, y0);
    pb.close();
}

fn fill_path(pm: &mut PixmapMut, pb: PathBuilder, rule: FillRule, c: Color, mask: Option<&Mask>) {
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(sk(c));
        pm.fill_path(&path, &paint, rule, Transform::identity(), mask);
    }
}

/// Fills rect `r` with corners rounded to `radius`, within `mask`.
pub fn fill(pm: &mut PixmapMut, r: Rect, radius: f64, c: Color, mask: Option<&Mask>) {
    if r.width <= 0.0 || r.height <= 0.0 {
        return;
    }
    let mut pb = PathBuilder::new();
    push_rounded(&mut pb, r, radius);
    fill_path(pm, pb, FillRule::Winding, c, mask);
}

/// Draws a border of `width` pixels inside the edges of rect `r`, whose corners are rounded to
/// `radius`, within `mask`. The inner edge has corners rounded to `radius - width`.
pub fn border(pm: &mut PixmapMut, r: Rect, width: f64, radius: f64, c: Color, mask: Option<&Mask>) {
    let w = width.min(r.width / 2.0).min(r.height / 2.0);
    if w <= 0.0 {
        return;
    }
    let mut pb = PathBuilder::new();
    push_rounded(&mut pb, r, radius);
    push_rounded(&mut pb, r.inset(w), radius - w);
    fill_path(pm, pb, FillRule::EvenOdd, c, mask);
}

/// A mask of the size of `pm` that covers rect `r` with corners rounded to `radius`.
pub fn mask(pm: &PixmapMut, r: Rect, radius: f64) -> Option<Mask> {
    let mut mask = Mask::new(pm.width(), pm.height())?;
    let mut pb = PathBuilder::new();
    push_rounded(&mut pb, r, radius);
    mask.fill_path(&pb.finish()?, FillRule::Winding, true, Transform::identity());
    Some(mask)
}
