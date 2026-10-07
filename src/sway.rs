use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
};

use serde::Deserialize;

use crate::rect::Rect;

#[derive(Deserialize, Debug)]
pub struct Node {
    pub id: i64,
    pub name: Option<String>,
    pub layout: String,
    pub rect: Rect,
    pub window_rect: Rect,
    pub deco_rect: Rect,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub focus: Vec<i64>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub floating_nodes: Vec<Node>,
    pub fullscreen_mode: Option<u8>,
    pub pid: Option<i32>,
    pub app_id: Option<String>,
    pub window_properties: Option<WindowProperties>,
    pub foreign_toplevel_identifier: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct WindowProperties {
    pub class: Option<String>,
}

/// Connection to the sway IPC socket.
pub struct Ipc(UnixStream);

const RUN_COMMAND: u32 = 0;
const SUBSCRIBE: u32 = 2;
const GET_TREE: u32 = 4;

fn socket() -> io::Result<UnixStream> {
    let path = std::env::var_os("SWAYSOCK")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SWAYSOCK is not set"))?;
    UnixStream::connect(path)
}

fn request(stream: &mut UnixStream, kind: u32, payload: &[u8]) -> io::Result<()> {
    let mut message = b"i3-ipc".to_vec();
    message.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(payload);
    stream.write_all(&message)
}

impl Ipc {
    pub fn connect() -> io::Result<Self> {
        socket().map(Ipc)
    }

    /// Sends a request and returns the raw JSON reply.
    fn roundtrip(&mut self, kind: u32, payload: &[u8]) -> io::Result<Vec<u8>> {
        request(&mut self.0, kind, payload)?;
        let mut header = [0u8; 14];
        self.0.read_exact(&mut header)?;
        let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
        let mut reply = vec![0; len];
        self.0.read_exact(&mut reply)?;
        Ok(reply)
    }

    pub fn get_tree(&mut self) -> io::Result<Vec<u8>> {
        self.roundtrip(GET_TREE, b"")
    }

    /// Runs a sway command and returns the error message of each failed part.
    pub fn run_command(&mut self, command: &str) -> io::Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Outcome {
            success: bool,
            error: Option<String>,
        }
        let reply = self.roundtrip(RUN_COMMAND, command.as_bytes())?;
        let outcomes: Vec<Outcome> = serde_json::from_slice(&reply).map_err(io::Error::other)?;
        Ok(outcomes.into_iter().filter(|o| !o.success).map(|o| o.error.unwrap_or_default()).collect())
    }
}

/// Opens a non-blocking connection subscribed to `events`, a JSON array of sway event names.
pub fn subscribe(events: &str) -> io::Result<UnixStream> {
    let mut stream = socket()?;
    request(&mut stream, SUBSCRIBE, events.as_bytes())?;
    stream.set_nonblocking(true)?;
    Ok(stream)
}
