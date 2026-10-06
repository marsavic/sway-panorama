//! Live window content through ext-image-copy-capture-v1 with per-toplevel sources.
//!
//! Each captured window has a subsurface of the main surface. Captured shm buffers are attached
//! to it, and wp_viewport scales and crops them, so the client never touches pixels.
//!
//! A buffer is marked active exactly while the compositor holds it: during a capture into it,
//! and while it is attached to the subsurface until the compositor releases it.

use std::collections::HashMap;

use smithay_client_toolkit::{
    compositor::{CompositorState, FrameCallbackData, Region},
    dispatch2::Dispatch2,
    shell::WaylandSurface,
    foreign_toplevel_list::ForeignToplevelList,
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
    subcompositor::SubcompositorState,
};
use tiny_skia::PixmapMut;
use wayland_client::{
    Connection, Proxy, QueueHandle, WEnum,
    globals::GlobalList,
    protocol::{wl_shm, wl_subsurface::WlSubsurface, wl_surface::WlSurface},
};
use wayland_protocols::{
    ext::{
        image_capture_source::v1::client::{
            ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1 as SourceManager,
            ext_image_capture_source_v1::ExtImageCaptureSourceV1 as Source,
        },
        image_copy_capture::v1::client::{
            ext_image_copy_capture_frame_v1::{self as frame, ExtImageCopyCaptureFrameV1 as Frame},
            ext_image_copy_capture_manager_v1::{self as manager, ExtImageCopyCaptureManagerV1 as CopyManager},
            ext_image_copy_capture_session_v1::{self as session, ExtImageCopyCaptureSessionV1 as Session},
        },
    },
    wp::viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

use crate::{App, draw::Canvas, sway::Rect};

pub struct Capture {
    copy: CopyManager,
    sources: SourceManager,
    viewporter: WpViewporter,
    subcompositor: SubcompositorState,
    pub toplevels: ForeignToplevelList,
    windows: HashMap<String, Window>,
    decors: Vec<Decor>,
    /// An empty input region, so that pointer events over subsurfaces go to the main surface.
    no_input: Region,
}

/// A subsurface with the schematic drawing of one floating layer, below its captured windows.
struct Decor {
    surface: WlSurface,
    subsurface: WlSubsurface,
    canvas: Canvas,
}

/// One subsurface to place: a decoration at a position, or a captured window.
pub enum Item<'a> {
    Decor(usize, i32, i32),
    Window(&'a str, Placement),
}

/// Where a captured window is shown: the screen rect of the window content and its visible part.
#[derive(Clone, Copy)]
pub struct Placement {
    pub dest: Rect,
    pub clip: Option<Rect>,
}

/// A buffer and its size.
type Image = (Buffer, (u32, u32));

struct Window {
    id: String,
    session: Option<Session>,
    source: Source,
    surface: WlSurface,
    subsurface: WlSubsurface,
    viewport: WpViewport,
    pool: SlotPool,
    /// Constraints being received, applied on `done`.
    pending: (u32, u32, Option<wl_shm::Format>),
    constraints: Option<(u32, u32, wl_shm::Format)>,
    /// The latest complete image.
    shown: Option<Image>,
    /// True while `shown` is attached to the surface.
    attached: bool,
    /// The frame in progress and its buffer.
    frame: Option<(Frame, Image)>,
    spare: Vec<Image>,
    /// A frame callback of the subsurface is pending; the next capture waits for it.
    waiting: bool,
    placement: Option<Placement>,
}

/// User data of objects that have no events.
pub struct NoEvents;

impl<I: Proxy, D> Dispatch2<I, D> for NoEvents {
    fn event(&self, _: &mut D, _: &I, _: I::Event, _: &Connection, _: &QueueHandle<D>) {}
}

pub struct SessionData(String);
pub struct FrameData(String);

impl Capture {
    /// Returns `None` if the compositor lacks per-toplevel capture.
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<App>, compositor: &CompositorState) -> Option<Self> {
        Some(Capture {
            copy: globals.bind(qh, 1..=1, NoEvents).ok()?,
            sources: globals.bind(qh, 1..=1, NoEvents).ok()?,
            viewporter: globals.bind(qh, 1..=1, NoEvents).ok()?,
            subcompositor: SubcompositorState::bind(compositor.wl_compositor().clone(), globals, qh).ok()?,
            no_input: Region::new(compositor).ok()?,
            windows: HashMap::new(),
            decors: Vec::new(),
            // Last, so that its events arrive only if capture is bound.
            toplevels: ForeignToplevelList::new(globals, qh),
        })
    }

    /// Starts capturing the windows with identifiers in `wanted` and stops capturing the others.
    pub fn sync(&mut self, wanted: &[&str], parent: &WlSurface, shm: &Shm, qh: &QueueHandle<App>) {
        self.windows.retain(|id, _| wanted.contains(&id.as_str()));
        for &id in wanted {
            if self.windows.contains_key(id) {
                continue;
            }
            let Some(handle) = self
                .toplevels
                .toplevels()
                .iter()
                .find(|h| self.toplevels.info(h).is_some_and(|i| i.identifier == id))
            else {
                continue;
            };
            let source = self.sources.create_source(handle, qh, NoEvents);
            let session =
                self.copy.create_session(&source, manager::Options::empty(), qh, SessionData(id.to_string()));
            let (subsurface, surface) = self.subcompositor.create_subsurface(parent.clone(), qh);
            surface.set_input_region(Some(self.no_input.wl_region()));
            let viewport = self.viewporter.get_viewport(&surface, qh, NoEvents);
            self.windows.insert(
                id.to_string(),
                Window {
                    id: id.to_string(),
                    session: Some(session),
                    source,
                    surface,
                    subsurface,
                    viewport,
                    pool: SlotPool::new(4096, shm).expect("create pool"),
                    pending: (0, 0, None),
                    constraints: None,
                    shown: None,
                    attached: false,
                    frame: None,
                    spare: Vec::new(),
                    waiting: false,
                    placement: None,
                },
            );
        }
    }

    pub fn is_captured(&self, id: &str) -> bool {
        self.windows.contains_key(id)
    }

    /// Returns a pixmap of size `w`×`h` for drawing decoration `i`.
    pub fn decor(&mut self, i: usize, w: u32, h: u32, parent: &WlSurface, shm: &Shm, qh: &QueueHandle<App>) -> PixmapMut<'_> {
        while self.decors.len() <= i {
            let (subsurface, surface) = self.subcompositor.create_subsurface(parent.clone(), qh);
            surface.set_input_region(Some(self.no_input.wl_region()));
            self.decors.push(Decor { surface, subsurface, canvas: Canvas::new(shm) });
        }
        self.decors[i].canvas.pixmap(w, h)
    }

    /// Places the subsurfaces in the given order, bottom first, and removes unused decorations.
    ///
    /// The changes take effect on the next commit of the parent.
    pub fn place(&mut self, items: &[Item], parent: &WlSurface, qh: &QueueHandle<App>) {
        let mut below = parent.clone();
        let mut decors = 0;
        for item in items {
            match *item {
                Item::Decor(i, x, y) => {
                    let d = &self.decors[i];
                    d.subsurface.place_above(&below);
                    below = d.surface.clone();
                    d.subsurface.set_position(x, y);
                    d.canvas.attach(&d.surface);
                    d.surface.commit();
                    decors = decors.max(i + 1);
                }
                Item::Window(id, placement) => {
                    let Some(w) = self.windows.get_mut(id) else { continue };
                    w.subsurface.place_above(&below);
                    below = w.surface.clone();
                    w.placement = Some(placement);
                    w.present(qh);
                    w.capture(qh);
                }
            }
        }
        self.decors.truncate(decors);
    }

    /// Handles a frame callback of a subsurface.
    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<App>) {
        if let Some(w) = self.windows.values_mut().find(|w| &w.surface == surface) {
            w.waiting = false;
            w.capture(qh);
        }
    }
}

impl Window {
    /// The integer surface-local rect of the visible part.
    fn visible(&self) -> Option<(i32, i32, i32, i32)> {
        let clip = self.placement?.clip?;
        let (x0, y0) = (clip.x.round() as i32, clip.y.round() as i32);
        let (x1, y1) = ((clip.x + clip.width).round() as i32, (clip.y + clip.height).round() as i32);
        (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
    }

    /// Shows the latest image at the current placement and commits the subsurface.
    fn present(&mut self, qh: &QueueHandle<App>) {
        match (self.visible(), &self.shown, self.placement) {
            (Some((x, y, w, h)), Some((buffer, (bw, bh))), Some(Placement { dest, .. })) => {
                // The source is the part of the image under the whole pixels of the subsurface,
                // so the image keeps the real valued position and scale of `dest`.
                let (bw, bh) = (*bw as f64, *bh as f64);
                let sx = ((x as f64 - dest.x) / dest.width * bw).clamp(0.0, bw);
                let sy = ((y as f64 - dest.y) / dest.height * bh).clamp(0.0, bh);
                self.subsurface.set_position(x, y);
                self.viewport.set_source(
                    sx,
                    sy,
                    (w as f64 / dest.width * bw).min(bw - sx),
                    (h as f64 / dest.height * bh).min(bh - sy),
                );
                self.viewport.set_destination(w, h);
                if !self.attached {
                    let _ = buffer.activate();
                    self.surface.attach(Some(buffer.wl_buffer()), 0, 0);
                    self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                    self.surface.frame(qh, FrameCallbackData(self.surface.clone()));
                    self.attached = true;
                    self.waiting = true;
                }
            }
            _ if self.attached => {
                self.surface.attach(None, 0, 0);
                self.attached = false;
            }
            _ => {}
        }
        self.surface.commit();
    }

    /// Starts capturing the next frame, if nothing prevents it.
    fn capture(&mut self, qh: &QueueHandle<App>) {
        let (Some(session), Some((w, h, format))) = (&self.session, self.constraints) else { return };
        if self.frame.is_some() || self.waiting || self.visible().is_none() {
            return;
        }
        let pool = &mut self.pool;
        let image = match self.spare.iter().position(|(b, size)| *size == (w, h) && b.canvas(pool).is_some()) {
            Some(i) => self.spare.swap_remove(i),
            None => (pool.create_buffer(w as i32, h as i32, w as i32 * 4, format).expect("create buffer").0, (w, h)),
        };
        image.0.activate().expect("buffer is free");
        let frame = session.create_frame(qh, FrameData(self.id.clone()));
        frame.attach_buffer(image.0.wl_buffer());
        frame.damage_buffer(0, 0, w as i32, h as i32);
        frame.capture();
        self.frame = Some((frame, image));
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some((frame, _)) = self.frame.take() {
            frame.destroy();
        }
        if let Some(session) = self.session.take() {
            session.destroy();
        }
        self.source.destroy();
        self.viewport.destroy();
        self.subsurface.destroy();
        self.surface.destroy();
    }
}

impl Drop for Decor {
    fn drop(&mut self) {
        self.subsurface.destroy();
        self.surface.destroy();
    }
}

impl Dispatch2<Session, App> for SessionData {
    fn event(&self, app: &mut App, _: &Session, event: session::Event, _: &Connection, qh: &QueueHandle<App>) {
        let Some(w) = app.capture.as_mut().and_then(|c| c.windows.get_mut(&self.0)) else { return };
        match event {
            session::Event::BufferSize { width, height } => (w.pending.0, w.pending.1) = (width, height),
            // Buffers have 4 bytes per pixel.
            session::Event::ShmFormat { format: WEnum::Value(f @ (wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888)) } => {
                w.pending.2.get_or_insert(f);
            }
            session::Event::Done => {
                if let (width, height, Some(format)) = w.pending {
                    w.constraints = Some((width, height, format));
                    w.spare.retain(|(_, size)| *size == (width, height));
                }
                w.pending = (0, 0, None);
                w.capture(qh);
            }
            session::Event::Stopped => {
                if let Some(s) = w.session.take() {
                    s.destroy();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<Frame, App> for FrameData {
    fn event(&self, app: &mut App, _: &Frame, event: frame::Event, _: &Connection, qh: &QueueHandle<App>) {
        let parent = app.window.wl_surface();
        let Some(w) = app.capture.as_mut().and_then(|c| c.windows.get_mut(&self.0)) else { return };
        let (ready, retry) = match event {
            frame::Event::Ready => (true, true),
            frame::Event::Failed { reason } => {
                (false, reason == WEnum::Value(frame::FailureReason::BufferConstraints))
            }
            _ => return,
        };
        let Some((frame, image)) = w.frame.take() else { return };
        frame.destroy();
        let _ = image.0.deactivate();
        if ready {
            if let Some(old) = w.shown.replace(image) {
                w.spare.push(old);
            }
            w.attached = false;
            w.present(qh);
            parent.commit();
        } else {
            w.spare.push(image);
        }
        if retry {
            w.capture(qh);
        }
    }
}
