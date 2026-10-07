use serde::Deserialize;

/// An axis-aligned rect, in any coordinates: sway layout, overview plane or screen.
#[derive(Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The rect with edges rounded to whole pixels.
    pub fn snap(&self) -> Rect {
        let (x, y) = (self.x.round(), self.y.round());
        Rect { x, y, width: (self.x + self.width).round() - x, height: (self.y + self.height).round() - y }
    }

    /// The rect shrunk by `d` on each side.
    pub fn inset(&self, d: f64) -> Rect {
        Rect { x: self.x + d, y: self.y + d, width: self.width - 2.0 * d, height: self.height - 2.0 * d }
    }

    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.width && p.1 >= self.y && p.1 < self.y + self.height
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Rect {
            x,
            y,
            width: (self.x + self.width).max(o.x + o.width) - x,
            height: (self.y + self.height).max(o.y + o.height) - y,
        }
    }

    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.width).min(o.x + o.width);
        let y1 = (self.y + self.height).min(o.y + o.height);
        (x1 > x0 && y1 > y0).then_some(Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 })
    }
}
