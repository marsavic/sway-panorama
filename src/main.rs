mod capture;
mod config;
mod draw;
mod icons;
mod model;
mod oklab;
mod sway;
mod view;

use std::{
    collections::HashMap,
    io::{ErrorKind, Read},
    mem::MaybeUninit,
    path::{Path, PathBuf},
    time::Duration,
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    delegate_registry,
    foreign_toplevel_list::{ForeignToplevelList, ForeignToplevelListHandler},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            EventLoop, Interest, Mode, PostAction,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;

use capture::{Capture, Item, Placement};
use config::{Config, Content};
use model::Overview;
use rustix::fs::inotify;
use sway::{Ipc, Rect};
use view::View;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Pointer movement, in screen pixels, after which a press of the left button pans instead of
/// clicking.
const CLICK_DISTANCE: f64 = 4.0;

/// A press of the left button: where it started, where the pointer was last, and whether the
/// pointer has moved far enough to pan.
struct Drag {
    start: (f64, f64),
    last: (f64, f64),
    panning: bool,
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    canvas: draw::Canvas,
    window: Window,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    modifiers: Modifiers,
    qh: QueueHandle<App>,
    capture: Option<Capture>,

    config: Config,
    config_path: PathBuf,
    ipc: Ipc,
    raw_tree: Vec<u8>,
    sizes: HashMap<String, (f64, f64)>,
    overviews: Vec<Overview>,
    view: View,
    painter: draw::Painter,

    size: (u32, u32),
    configured: bool,
    dirty: bool,
    frame_pending: bool,
    drag: Option<Drag>,
    exit: bool,
}

fn main() {
    let path = std::env::args_os().nth(1).map(PathBuf::from).unwrap_or_else(config::path);
    let config = config::load(&path).unwrap_or_else(|e| {
        eprintln!("sway-panorama: {e}");
        std::process::exit(1);
    });
    let (ipc, events) = Ipc::connect()
        .and_then(|ipc| Ok((ipc, sway::subscribe(r#"["window","workspace","output","binding"]"#)?)))
        .unwrap_or_else(|e| {
            eprintln!("sway-panorama: cannot connect to sway: {e}");
            std::process::exit(1);
        });

    let conn = Connection::connect_to_env().expect("cannot connect to the Wayland display");
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<App> = EventLoop::try_new().unwrap();
    WaylandSource::new(conn.clone(), event_queue).insert(event_loop.handle()).unwrap();

    let compositor = CompositorState::bind(&globals, &qh).expect("wl_compositor is not available");
    let xdg_shell = XdgShell::bind(&globals, &qh).expect("xdg_shell is not available");
    let shm = Shm::bind(&globals, &qh).expect("wl_shm is not available");

    let window = xdg_shell.create_window(compositor.create_surface(&qh), WindowDecorations::RequestServer, &qh);
    window.set_title("sway-panorama");
    window.set_app_id("sway-panorama");
    window.commit();

    let capture = Capture::bind(&globals, &qh, &compositor);
    if capture.is_none() && config.content != Content::None {
        warn_no_capture();
    }

    // Sway events signal that the tree may have changed; their content is not needed.
    let handle = event_loop.handle();
    handle
        .insert_source(Generic::new(events, Interest::READ, Mode::Level), |_, stream, app| {
            let mut buf = [0u8; 65536];
            loop {
                match (&**stream).read(&mut buf) {
                    Ok(0) => app.exit = true,
                    Ok(_) => continue,
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => app.exit = true,
                }
                break;
            }
            app.poll();
            Ok(PostAction::Continue)
        })
        .unwrap();

    // Polling covers the changes that sway reports with no event, such as resizes.
    handle
        .insert_source(Timer::from_duration(Duration::from_secs(1)), |_, _, app| {
            if app.config.poll_interval > 0 {
                app.poll();
            }
            TimeoutAction::ToDuration(Duration::from_millis(match app.config.poll_interval {
                0 => 1000,
                ms => ms,
            }))
        })
        .unwrap();

    let watch = watch_config(&path).unwrap_or_else(|e| {
        eprintln!("sway-panorama: cannot watch {}: {e}", path.display());
        std::process::exit(1);
    });
    let name = path.file_name().unwrap_or_default().to_owned();
    handle
        .insert_source(Generic::new(watch, Interest::READ, Mode::Level), move |_, fd, app| {
            let mut buf = [MaybeUninit::uninit(); 4096];
            let mut reader = inotify::Reader::new(&**fd, &mut buf);
            let mut changed = false;
            while let Ok(event) = reader.next() {
                changed |= event.file_name().is_some_and(|n| n.to_bytes() == name.as_encoded_bytes());
            }
            if changed {
                app.reload();
            }
            Ok(PostAction::Continue)
        })
        .unwrap();

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        canvas: draw::Canvas::new(&shm),
        shm,
        window,
        pointer: None,
        keyboard: None,
        modifiers: Modifiers::default(),
        qh,
        capture,
        painter: draw::Painter::new(&config),
        config,
        config_path: path,
        ipc,
        raw_tree: Vec::new(),
        sizes: HashMap::new(),
        overviews: Vec::new(),
        view: View::new(),
        size: (960, 600),
        configured: false,
        dirty: false,
        frame_pending: false,
        drag: None,
        exit: false,
    };

    app.poll();
    while !app.exit {
        if let Err(e) = event_loop.dispatch(None, &mut app) {
            eprintln!("sway-panorama: {e}");
            break;
        }
    }
}

fn warn_no_capture() {
    eprintln!("sway-panorama: the compositor lacks per-window capture (sway 1.12); drawing schematically");
}

/// Watches the directory of the config file, which also covers editors that replace the file.
fn watch_config(path: &Path) -> rustix::io::Result<std::os::fd::OwnedFd> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
    inotify::add_watch(&fd, dir, inotify::WatchFlags::CLOSE_WRITE | inotify::WatchFlags::MOVED_TO)?;
    Ok(fd)
}

impl App {
    fn poll(&mut self) {
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

    fn reload(&mut self) {
        let config = match config::load(&self.config_path) {
            Ok(config) => config,
            Err(e) => return eprintln!("sway-panorama: {e}"),
        };
        if config.icon_theme != self.config.icon_theme {
            self.painter.set_icon_theme(config.icon_theme.clone());
        }
        if self.capture.is_none() && config.content != Content::None && self.config.content == Content::None {
            warn_no_capture();
        }
        self.config = config;
        self.rebuild();
    }

    /// Focuses the window or title bar at screen point `p`, or else the workspace there.
    fn click(&mut self, p: (f64, f64)) {
        let inside = |r: Rect| p.0 >= r.x && p.0 < r.x + r.width && p.1 >= r.y && p.1 < r.y + r.height;
        let command = self.overviews.iter().rev().find_map(|o| {
            let r = self.view.to_screen(o.rect);
            let item = o.layers.iter().rev().flat_map(|l| l.iter().rev()).find_map(|e| {
                let (id, rect) = match e {
                    model::Element::Bar(b) => (b.id, b.rect),
                    model::Element::Window(w) => (w.id, w.frame),
                };
                (inside(r) && inside(self.view.to_screen(rect))).then(|| format!("[con_id={id}] focus"))
            });
            item.or_else(|| {
                let name = o.name.replace('\\', "\\\\").replace('"', "\\\"");
                inside(r.inset(-self.config.workspace_border))
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
                self.overviews = model::build(&tree, &self.config, &mut self.sizes);
                self.sync_captures();
                self.redraw();
            }
            Err(e) => eprintln!("sway-panorama: cannot parse sway tree: {e}"),
        }
    }

    /// Captures the windows that content mode shows, except the window of this program.
    fn sync_captures(&mut self) {
        let Some(capture) = &mut self.capture else { return };
        let content = self.config.content;
        let wanted: Vec<&str> = self
            .overviews
            .iter()
            .filter(|o| content == Content::All || content == Content::Visible && o.visible)
            .flat_map(|o| o.layers.iter().flatten())
            .filter_map(|e| match e {
                model::Element::Window(w) if !w.own => w.identifier.as_deref(),
                _ => None,
            })
            .collect();
        capture.sync(&wanted, self.window.wl_surface(), &self.shm, &self.qh);
    }

    fn redraw(&mut self) {
        self.dirty = true;
        if self.configured && !self.frame_pending {
            self.draw();
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let (w, h) = self.size;
        self.view.size = (w as f64, h as f64);
        let bounds = self.overviews.iter().map(|o| o.rect).reduce(|a, b| a.union(&b)).unwrap_or_default();
        let label = if self.config.show_workspace_names { 1.25 * self.config.label_size } else { 0.0 };
        self.view.fit(bounds, self.config.margin, self.config.workspace_border, label);

        // With captured windows, floating layers are drawn in subsurfaces above the captures below.
        let layered = self.capture.is_some() && self.config.content != Content::None;
        let mut pm = self.canvas.pixmap(w, h);
        self.painter.paint(&mut pm, &self.overviews, &self.view, &self.config, !layered);

        let surface = self.window.wl_surface();
        if let Some(capture) = &mut self.capture {
            let screen = Rect { x: 0.0, y: 0.0, width: w as f64, height: h as f64 };
            let mut items = Vec::new();
            let mut decors = 0;
            for o in self.overviews.iter().filter(|_| layered) {
                let area = draw::workspace_area(o, &self.view, &self.config, screen);
                for (k, layer) in o.layers.iter().enumerate() {
                    if k > 0
                        && let Some(area) = area
                        && let Some(b) = layer_box(layer, &self.view, area)
                    {
                        let mut pm = capture.decor(decors, b.width as u32, b.height as u32, surface, &self.shm, &self.qh);
                        pm.fill(tiny_skia::Color::TRANSPARENT);
                        let clip = Rect { x: area.x - b.x, y: area.y - b.y, ..area };
                        self.painter.paint_layer(&mut pm, layer, &self.view, (b.x, b.y), clip, &self.config);
                        items.push(Item::Decor(decors, b.x as i32, b.y as i32));
                        decors += 1;
                    }
                    for e in layer {
                        if let model::Element::Window(win) = e
                            && let Some(id) = win.identifier.as_deref()
                            && capture.is_captured(id)
                        {
                            let dest = self.view.to_screen(win.content);
                            let inner = draw::window_area(win.frame, &self.view, &self.config);
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
fn layer_box(layer: &[model::Element], view: &View, area: Rect) -> Option<Rect> {
    let r = layer
        .iter()
        .map(|e| match e {
            model::Element::Bar(b) => view.to_screen(b.rect),
            model::Element::Window(w) => view.to_screen(w.frame),
        })
        .reduce(|a, b| a.union(&b))?
        .intersect(&area)?;
    let (x, y) = (r.x.floor(), r.y.floor());
    Some(Rect { x, y, width: (r.x + r.width).ceil() - x, height: (r.y + r.height).ceil() - y })
}

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        if surface != self.window.wl_surface() {
            if let Some(capture) = &mut self.capture {
                capture.frame_done(surface, qh);
            }
            return;
        }
        self.frame_pending = false;
        if self.dirty {
            self.draw();
        }
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window, configure: WindowConfigure, _: u32) {
        if let (Some(w), Some(h)) = configure.new_size {
            self.size = (w.get(), h.get());
        }
        self.configured = true;
        self.redraw();
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer
            && let Some(p) = self.pointer.take()
        {
            p.release();
        }
        if capability == Capability::Keyboard
            && let Some(k) = self.keyboard.take()
        {
            k.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
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
}

impl ForeignToplevelListHandler for App {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelList {
        &mut self.capture.as_mut().expect("bound with capture").toplevels
    }

    fn new_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: ExtForeignToplevelHandleV1) {
        self.sync_captures();
        self.redraw();
    }

    fn update_toplevel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: ExtForeignToplevelHandleV1) {}

    fn toplevel_closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: ExtForeignToplevelHandleV1) {}
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: &wl_surface::WlSurface, _: u32) {}

    fn press_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, event: KeyEvent) {
        let (keys, key, m) = (&self.config.keys, event.keysym, &self.modifiers);
        if keys.reset.matches(key, m) {
            self.view.reset();
        } else if keys.toggle_icons.matches(key, m) {
            self.config.show_icons ^= true;
        } else if keys.toggle_colors.matches(key, m) {
            self.config.app_colors ^= true;
        } else if keys.toggle_titles.matches(key, m) {
            self.config.show_titles ^= true;
        } else if keys.toggle_workspace_names.matches(key, m) {
            self.config.show_workspace_names ^= true;
        } else {
            return;
        }
        self.redraw();
    }

    fn repeat_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn release_key(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_keyboard::WlKeyboard, _: u32, _: KeyEvent) {}

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
