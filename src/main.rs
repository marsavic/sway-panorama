mod app;
mod capture;
mod color;
mod config;
mod draw;
mod model;
mod place;
mod rect;
mod sway;
mod view;
mod wayland;

use std::{
    io::{ErrorKind, Read},
    mem::MaybeUninit,
    path::{Path, PathBuf},
    time::Duration,
};

use rustix::fs::inotify;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    reexports::{
        calloop::{
            EventLoop, Interest, Mode, PostAction,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::RegistryState,
    seat::{SeatState, keyboard::Modifiers},
    shell::{
        WaylandSurface,
        xdg::{XdgShell, window::WindowDecorations},
    },
    shm::Shm,
};
use wayland_client::{Connection, globals::registry_queue_init};

use app::{App, warn_no_capture};
use capture::Capture;
use config::Content;
use sway::Ipc;
use view::View;

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
    if capture.is_none() && config.window.content != Content::None {
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

/// Watches the directory of the config file, which also covers editors that replace the file.
fn watch_config(path: &Path) -> rustix::io::Result<std::os::fd::OwnedFd> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let fd = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
    inotify::add_watch(&fd, dir, inotify::WatchFlags::CLOSE_WRITE | inotify::WatchFlags::MOVED_TO)?;
    Ok(fd)
}
