use crate::sway::Rect;

/// Camera over the overview plane.
pub struct View {
    pub size: (f64, f64),
    zoom: f64,
    center: (f64, f64),
    /// While true, the camera follows the fit of the content.
    fit: bool,
}

impl View {
    pub fn new() -> Self {
        View { size: (1.0, 1.0), zoom: 1.0, center: (0.0, 0.0), fit: true }
    }

    pub fn to_screen(&self, r: Rect) -> Rect {
        Rect {
            x: (r.x - self.center.0) * self.zoom + self.size.0 / 2.0,
            y: (r.y - self.center.1) * self.zoom + self.size.1 / 2.0,
            width: r.width * self.zoom,
            height: r.height * self.zoom,
        }
    }

    /// Fits `bounds` into the view if the camera follows the fit.
    ///
    /// On each side it leaves `margin` plane units less `border` screen pixels, so that with
    /// `margin` equal to the space between two workspaces, the space outside the outer workspace
    /// borders equals the space between two workspace borders. It leaves at least `border` screen
    /// pixels, and above at least `border + label` for the labels.
    pub fn fit(&mut self, bounds: Rect, margin: f64, border: f64, label: f64) {
        if !self.fit || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return;
        }
        let (w, h) = self.size;
        let (bw, bh, m, b) = (bounds.width, bounds.height, margin, border);
        let t = b + label;
        // Each side takes max(m·zoom − b, minimum) pixels; every combination of the two terms
        // bounds the zoom.
        let zoom = [
            (w + 2.0 * b) / (bw + 2.0 * m),
            (w - 2.0 * b) / bw,
            (h + 2.0 * b) / (bh + 2.0 * m),
            h / (bh + m),
            (h - t + b) / (bh + m),
            (h - t - b) / bh,
        ]
        .into_iter()
        .fold(f64::INFINITY, f64::min)
        .max(1e-6);
        let side = |min: f64| (m * zoom - b).max(min);
        self.zoom = zoom;
        self.center = (bounds.x + bw / 2.0, bounds.y + bh / 2.0 + (side(b) - side(t)) / 2.0 / zoom);
    }

    pub fn reset(&mut self) {
        self.fit = true;
    }

    /// Zooms by `factor`, keeping the world point under screen point `p` fixed.
    pub fn zoom_at(&mut self, p: (f64, f64), factor: f64) {
        let w = (
            self.center.0 + (p.0 - self.size.0 / 2.0) / self.zoom,
            self.center.1 + (p.1 - self.size.1 / 2.0) / self.zoom,
        );
        self.zoom *= factor;
        self.center = (
            w.0 - (p.0 - self.size.0 / 2.0) / self.zoom,
            w.1 - (p.1 - self.size.1 / 2.0) / self.zoom,
        );
        self.fit = false;
    }

    /// Moves the content by `d` screen pixels.
    pub fn pan(&mut self, d: (f64, f64)) {
        self.center = (self.center.0 - d.0 / self.zoom, self.center.1 - d.1 / self.zoom);
        self.fit = false;
    }
}
