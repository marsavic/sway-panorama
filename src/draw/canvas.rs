use smithay_client_toolkit::shm::{
    Shm,
    slot::{Buffer, SlotPool},
};
use tiny_skia::PixmapMut;
use wayland_client::protocol::{wl_shm, wl_surface::WlSurface};

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
