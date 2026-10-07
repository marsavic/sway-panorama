//! The state of the program and its reactions to sway, the config and input.

use std::path::PathBuf;

use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    output::OutputState,
    registry::RegistryState,
    seat::{
        SeatState,
        keyboard::{Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind},
    },
    shell::{WaylandSurface, xdg::window::Window},
    shm::Shm,
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_keyboard, wl_pointer},
};

use crate::{
    capture::{Capture, Item, Placement},
    config::{self, Config, Content, Cycle},
    draw,
    model::{self, Element, Kind, Overview},
    place,
    rect::Rect,
    sway::Ipc,
    view::View,
};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Pointer movement, in screen pixels, after which a press of the left button pans instead of
/// clicking.
const CLICK_DISTANCE: f64 = 4.0;

/// A press of the left button: where it started, where the pointer was last, and whether the
/// pointer has moved far enough to pan.
pub struct Drag {
    start: (f64, f64),
    last: (f64, f64),
    panning: bool,
}

/// The state of the program: its Wayland objects, the config, the sway tree and the view.
pub struct App {
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub shm: Shm,
    pub canvas: draw::Canvas,
    pub window: Window,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    pub modifiers: Modifiers,
    pub qh: QueueHandle<App>,
    pub capture: Option<Capture>,

    pub config: Config,
    pub config_path: PathBuf,
    pub ipc: Ipc,
    pub raw_tree: Vec<u8>,
    pub overviews: Vec<Overview>,
    pub view: View,
    pub painter: draw::Painter,

    pub size: (u32, u32),
    pub configured: bool,
    pub dirty: bool,
    pub frame_pending: bool,
    pub drag: Option<Drag>,
    pub exit: bool,
}

pub fn warn_no_capture() {
    eprintln!("sway-panorama: the compositor lacks per-window capture (sway 1.12); drawing schematically");
}

impl App {
    /// Handles the pointer events of one frame: zooming, panning and clicking.
    pub fn pointer(&mut self, events: &[PointerEvent]) {
        for event in events {
            let p = event.position;
            match event.kind {
                PointerEventKind::Axis { vertical, .. } if vertical.absolute != 0.0 => {
                    self.view.zoom_at(p, 1.2f64.powf(-vertical.absolute / 15.0));
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => {
                    self.drag = Some(Drag { start: p, last: p, panning: false });
                    continue;
                }
                PointerEventKind::Release { button: BTN_LEFT, .. } => {
                    if let Some(d) = self.drag.take()
                        && !d.panning
                    {
                        self.click(d.start);
                    }
                    continue;
                }
                PointerEventKind::Press { button: BTN_RIGHT, .. } => self.view.reset(),
                PointerEventKind::Motion { .. } => match &mut self.drag {
                    Some(d) if d.panning || (p.0 - d.start.0).hypot(p.1 - d.start.1) > CLICK_DISTANCE => {
                        d.panning = true;
                        self.view.pan((p.0 - d.last.0, p.1 - d.last.1));
                        d.last = p;
                    }
                    _ => continue,
                },
                _ => continue,
            }
            self.redraw();
        }
    }

    /// Handles a press of `key` with the bindings of `config.keys`.
    pub fn key(&mut self, key: Keysym) {
        let (keys, m) = (&self.config.keys, &self.modifiers);
        if keys.reset.matches(key, m) {
            self.view.reset();
        } else if keys.cycle_content.matches(key, m) {
            self.config.window.content = self.config.window.content.next();
            self.sync_captures();
        } else if keys.cycle_icons.matches(key, m) {
            self.config.window.icons = self.config.window.icons.next();
        } else if keys.cycle_colors.matches(key, m) {
            self.config.window.app_colors = self.config.window.app_colors.next();
        } else if keys.cycle_titles.matches(key, m) {
            self.config.window.titles = self.config.window.titles.next();
        } else if keys.toggle_workspace_names.matches(key, m) {
            self.config.workspace.show_names ^= true;
        } else {
            return;
        }
        self.redraw();
    }

    /// Reads the sway tree and rebuilds the overviews if it changed.
    pub fn poll(&mut self) {
        match self.ipc.get_tree() {
            Ok(raw) if raw == self.raw_tree => {}
            Ok(raw) => {
                self.raw_tree = raw;
                self.rebuild();
            }
            Err(e) => {
                eprintln!("sway-panorama: sway IPC: {e}");
                self.exit = true;
            }
        }
    }

    /// Loads the config file again; a file with errors is reported and the previous config stays.
    pub fn reload(&mut self) {
        let config = match config::load(&self.config_path) {
            Ok(config) => config,
            Err(e) => return eprintln!("sway-panorama: {e}"),
        };
        if config.window.icon_theme != self.config.window.icon_theme {
            self.painter.set_icon_theme(config.window.icon_theme.clone());
        }
        if self.capture.is_none() && config.window.content != Content::None && self.config.window.content == Content::None {
            warn_no_capture();
        }
        self.config = config;
        self.rebuild();
    }

    /// Focuses the window or title bar at screen point `p`, or else the workspace there.
    fn click(&mut self, p: (f64, f64)) {
        let command = self.overviews.iter().rev().find_map(|o| {
            let r = place::workspace_rect(o.rect, &self.view, &self.config);
            let item = o.layers.iter().rev().flat_map(|l| l.iter().rev()).find_map(|e| {
                let rect = place::element_rect(e.rect, e.tile, &self.view, &self.config);
                (r.contains(p) && rect.contains(p)).then(|| format!("[con_id={}] focus", e.id))
            });
            item.or_else(|| {
                let name = o.name.replace('\\', "\\\\").replace('"', "\\\"");
                r.inset(-self.config.workspace.border).contains(p)
                    .then(|| format!("workspace --no-auto-back-and-forth \"{name}\""))
            })
        });
        if let Some(command) = command {
            match self.ipc.run_command(&command) {
                Ok(errors) => errors.iter().for_each(|e| eprintln!("sway-panorama: {command}: {e}")),
                Err(e) => eprintln!("sway-panorama: sway IPC: {e}"),
            }
        }
    }

    /// Builds the overviews from the last sway tree and the config.
    fn rebuild(&mut self) {
        match serde_json::from_slice(&self.raw_tree) {
            Ok(tree) => {
                self.overviews = model::build(&tree, &self.config);
                self.sync_captures();
                self.redraw();
            }
            Err(e) => eprintln!("sway-panorama: cannot parse sway tree: {e}"),
        }
    }

    /// Captures the windows that content mode shows, except the window of this program.
    pub fn sync_captures(&mut self) {
        let Some(capture) = &mut self.capture else { return };
        let content = self.config.window.content;
        let wanted: Vec<&str> = self
            .overviews
            .iter()
            .filter(|o| content == Content::All || content == Content::Visible && o.visible)
            .flat_map(|o| o.layers.iter().flatten())
            .filter_map(|e| match &e.kind {
                Kind::Window(w) if !w.own => w.identifier.as_deref(),
                _ => None,
            })
            .collect();
        capture.sync(&wanted, self.window.wl_surface(), &self.shm, &self.qh);
    }

    pub fn redraw(&mut self) {
        self.dirty = true;
        if self.configured && !self.frame_pending {
            self.draw();
        }
    }

    pub fn draw(&mut self) {
        self.dirty = false;
        let (w, h) = self.size;
        self.view.size = (w as f64, h as f64);
        let bounds = self.overviews.iter().map(|o| o.rect).reduce(|a, b| a.union(&b)).unwrap_or_default();
        let (outside, label) = (place::outside(&self.config), place::label_band(&self.config));
        self.view.fit(bounds, self.config.margin, outside, label);

        // With captured windows, floating layers are drawn in subsurfaces above the captures below.
        let layered = self.capture.is_some() && self.config.window.content != Content::None;
        let mut pm = self.canvas.pixmap(w, h);
        self.painter.paint(&mut pm, &self.overviews, &self.view, &self.config, !layered);

        let surface = self.window.wl_surface();
        if let Some(capture) = &mut self.capture {
            let screen = Rect { x: 0.0, y: 0.0, width: w as f64, height: h as f64 };
            let mut items = Vec::new();
            let mut decors = 0;
            for o in self.overviews.iter().filter(|_| layered) {
                let rect = place::workspace_rect(o.rect, &self.view, &self.config);
                let area = rect.intersect(&screen);
                for (k, layer) in o.layers.iter().enumerate() {
                    if k > 0
                        && let Some(area) = area
                        && let Some(b) = layer_box(layer, &self.view, &self.config, area)
                    {
                        let mut pm = capture.decor(decors, b.width as u32, b.height as u32, surface, &self.shm, &self.qh);
                        pm.fill(tiny_skia::Color::TRANSPARENT);
                        self.painter.paint_layer(&mut pm, layer, &self.view, (b.x, b.y), rect, &self.config);
                        items.push(Item::Decor(decors, b.x as i32, b.y as i32));
                        decors += 1;
                    }
                    for e in layer {
                        if let Kind::Window(win) = &e.kind
                            && let Some(id) = win.identifier.as_deref()
                            && capture.is_captured(id)
                        {
                            let dest = place::element_rect(win.content, e.tile, &self.view, &self.config);
                            let inner = place::window_area(e.rect, e.tile, &self.view, &self.config);
                            let clip = area.and_then(|a| dest.intersect(&a)?.intersect(&inner));
                            items.push(Item::Window(id, Placement { dest, clip }));
                        }
                    }
                }
            }
            capture.place(&items, surface, &self.qh);
        }
        self.canvas.attach(surface);
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        self.window.commit();
        self.frame_pending = true;
    }
}

/// The screen rect covered by the elements of `layer` within `area`, with integer bounds.
fn layer_box(layer: &[Element], view: &View, config: &Config, area: Rect) -> Option<Rect> {
    let r = layer
        .iter()
        .map(|e| place::element_rect(e.rect, e.tile, view, config))
        .reduce(|a, b| a.union(&b))?
        .intersect(&area)?;
    let (x, y) = (r.x.floor(), r.y.floor());
    Some(Rect { x, y, width: (r.x + r.width).ceil() - x, height: (r.y + r.height).ceil() - y })
}
