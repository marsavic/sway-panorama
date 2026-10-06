use std::{collections::HashMap, path::PathBuf};

use freedesktop_desktop_entry::{DesktopEntry, find_app_by_id, unicase::Ascii};
use resvg::usvg;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

/// App icons, looked up through desktop entries and the icon theme.
pub struct Icons {
    theme: String,
    entries: Option<Vec<DesktopEntry>>,
    paths: HashMap<String, Option<PathBuf>>,
    pixmaps: HashMap<(String, u32), Option<Pixmap>>,
}

impl Icons {
    pub fn new(theme: Option<String>) -> Self {
        Icons {
            theme: theme.or_else(freedesktop_icons::default_theme_gtk).unwrap_or_else(|| "hicolor".into()),
            entries: None,
            paths: HashMap::new(),
            pixmaps: HashMap::new(),
        }
    }

    /// Returns the icon of `app` rasterized at `size` pixels.
    pub fn get(&mut self, app: &str, size: u32) -> Option<&Pixmap> {
        let key = (app.to_string(), size);
        if !self.pixmaps.contains_key(&key) {
            let pixmap = self.path(app).and_then(|p| rasterize(&p, size));
            self.pixmaps.insert(key.clone(), pixmap);
        }
        self.pixmaps[&key].as_ref()
    }

    fn path(&mut self, app: &str) -> Option<PathBuf> {
        if let Some(p) = self.paths.get(app) {
            return p.clone();
        }
        let entries = self.entries.get_or_insert_with(|| {
            freedesktop_desktop_entry::desktop_entries(&freedesktop_desktop_entry::get_languages_from_env())
        });
        let name = find_app_by_id(entries, Ascii::new(app)).and_then(|e| e.icon()).unwrap_or(app).to_string();
        let path = if name.starts_with('/') {
            Some(PathBuf::from(name))
        } else {
            freedesktop_icons::lookup(&name).with_theme(&self.theme).with_size(128).with_cache().find()
        };
        self.paths.insert(app.to_string(), path.clone());
        path
    }
}

fn rasterize(path: &std::path::Path, size: u32) -> Option<Pixmap> {
    let data = std::fs::read(path).ok()?;
    let mut out = Pixmap::new(size, size)?;
    if path.extension().is_some_and(|e| e == "svg") {
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
        let s = tree.size();
        let scale = size as f32 / s.width().max(s.height());
        let (dx, dy) = ((size as f32 - s.width() * scale) / 2.0, (size as f32 - s.height() * scale) / 2.0);
        resvg::render(&tree, Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), &mut out.as_mut());
    } else {
        let image = Pixmap::decode_png(&data).ok()?;
        let (w, h) = (image.width() as f32, image.height() as f32);
        let scale = size as f32 / w.max(h);
        let (dx, dy) = ((size as f32 - w * scale) / 2.0, (size as f32 - h * scale) / 2.0);
        let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
        out.draw_pixmap(0, 0, image.as_ref(), &paint, Transform::from_row(scale, 0.0, 0.0, scale, dx, dy), None);
    }
    Some(out)
}
