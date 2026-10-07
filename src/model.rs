use std::collections::HashMap;

use crate::{config::Config, rect::Rect, sway::Node};

pub struct Overview {
    pub name: String,
    /// The output area of the workspace, in world coordinates.
    pub rect: Rect,
    /// The part of `rect` that windows can use; the rest is reserved, for example by a bar.
    pub usable: Rect,
    pub visible: bool,
    pub focused: bool,
    pub urgent: bool,
    /// Title bars and windows in drawing order, in layers: the tiling layer first, then one
    /// layer for each floating container.
    pub layers: Vec<Vec<Element>>,
}

/// A title bar or a window.
pub struct Element {
    /// The sway id of the container.
    pub id: i64,
    /// The title bar, or the window frame including its borders.
    pub rect: Rect,
    /// The tile the element belongs to: a window with its title bar, or a stacked or tabbed
    /// container with its title bars and the window it shows. The window gap reduces each tile.
    pub tile: Rect,
    pub title: String,
    /// The app of the window; `None` for the title bar of a container of several windows.
    pub app: Option<String>,
    /// True if the container has the focus or contains it.
    pub focused: bool,
    pub urgent: bool,
    pub kind: Kind,
}

pub enum Kind {
    Bar,
    Window(Window),
}

pub struct Window {
    /// The area of the window content.
    pub content: Rect,
    /// True if sway draws a title bar for the window.
    pub bar: bool,
    pub identifier: Option<String>,
    /// True for the window of this program.
    pub own: bool,
}

/// Maps layout coordinates of one output to world coordinates of one workspace overview.
struct Mapping {
    output: Rect,
    place: Rect,
}

impl Mapping {
    fn map(&self, r: Rect) -> Rect {
        let (sx, sy) = (self.place.width / self.output.width, self.place.height / self.output.height);
        Rect {
            x: self.place.x + (r.x - self.output.x) * sx,
            y: self.place.y + (r.y - self.output.y) * sy,
            width: r.width * sx,
            height: r.height * sy,
        }
    }
}

/// Builds the overviews of the configured workspaces.
pub fn build(tree: &Node, config: &Config) -> Vec<Overview> {
    let mut found = HashMap::new();
    for output in tree.nodes.iter().filter(|o| o.name.as_deref() != Some("__i3")) {
        for ws in &output.nodes {
            if let Some(name) = &ws.name {
                found.insert(name.as_str(), (output, ws));
            }
        }
    }

    config
        .workspace
        .rect
        .iter()
        .map(|(name, p)| (name, Rect { x: p.x, y: p.y, width: p.w, height: p.h }))
        .map(|(name, place)| match found.get(name.as_str()) {
            Some(&(output, ws)) => {
                let mapping = Mapping { output: output.rect, place };
                let mut layers = vec![Vec::new()];
                match find_fullscreen(ws) {
                    Some(fs) => add(fs, (fs.rect.x, fs.rect.y), 1, None, true, &mapping, &mut layers[0]),
                    None => {
                        let origin = (ws.rect.x, ws.rect.y);
                        children(ws, origin, true, &mapping, &mut layers[0]);
                        for f in &ws.floating_nodes {
                            let mut layer = Vec::new();
                            add(f, origin, 1, None, true, &mapping, &mut layer);
                            layers.push(layer);
                        }
                    }
                }
                Overview {
                    name: name.clone(),
                    rect: place,
                    usable: mapping.map(ws.rect),
                    visible: output.focus.first() == Some(&ws.id),
                    focused: contains_focus(ws),
                    urgent: ws.urgent,
                    layers,
                }
            }
            None => Overview {
                name: name.clone(),
                rect: place,
                usable: place,
                visible: false,
                focused: false,
                urgent: false,
                layers: vec![Vec::new()],
            },
        })
        .collect()
}

fn find_fullscreen(node: &Node) -> Option<&Node> {
    node.nodes.iter().chain(&node.floating_nodes).find_map(|c| {
        if c.fullscreen_mode.unwrap_or(0) != 0 { Some(c) } else { find_fullscreen(c) }
    })
}

fn contains_focus(node: &Node) -> bool {
    node.focused || node.nodes.iter().chain(&node.floating_nodes).any(contains_focus)
}

/// Adds the tiling children of `node`, whose box origin is `origin`.
fn children(node: &Node, origin: (f64, f64), shown: bool, mapping: &Mapping, out: &mut Vec<Element>) {
    let stacked = node.layout == "stacked";
    let tabbed = node.layout == "tabbed";
    let count = if stacked { node.nodes.len() } else { 1 };
    // The title bars of a stacked or tabbed container and the window it shows form one tile.
    let tile = (stacked || tabbed).then(|| node.nodes.iter().fold(node.rect, |r, c| r.union(&bar_rect(c, origin))));
    for child in &node.nodes {
        let front = !(stacked || tabbed) || node.focus.first() == Some(&child.id);
        add(child, origin, count, tile, shown && front, mapping, out);
    }
}

/// The layout rect of the title bar of container `c`, whose parent has box origin `origin`.
fn bar_rect(c: &Node, origin: (f64, f64)) -> Rect {
    let d = c.deco_rect;
    Rect { x: origin.0 + d.x, y: origin.1 + d.y, ..d }
}

/// Adds the title bar, window and descendants of container `c`.
///
/// `parent_origin` is the box origin of the parent, to which sway's `deco_rect` is relative.
/// `count` is the number of title bars above the content of `c` (siblings in a stacked parent).
/// `tile` is the tile of the title bar and window of `c` if the parent is stacked or tabbed.
fn add(
    c: &Node,
    parent_origin: (f64, f64),
    count: usize,
    tile: Option<Rect>,
    shown: bool,
    mapping: &Mapping,
    out: &mut Vec<Element>,
) {
    let d = c.deco_rect;
    let bar = bar_rect(c, parent_origin);
    let tile = mapping.map(tile.unwrap_or(if d.height > 0.0 { c.rect.union(&bar) } else { c.rect }));
    let element = |rect, kind| Element {
        id: c.id,
        rect: mapping.map(rect),
        tile,
        title: c.name.clone().unwrap_or_default(),
        app: c.pid.and(c.app_id.clone().or_else(|| c.window_properties.as_ref()?.class.clone())),
        focused: contains_focus(c),
        urgent: c.urgent,
        kind,
    };
    if d.height > 0.0 {
        out.push(element(bar, Kind::Bar));
    }
    if c.pid.is_some() {
        if shown {
            let w = c.window_rect;
            out.push(element(
                c.rect,
                Kind::Window(Window {
                    content: mapping.map(Rect { x: c.rect.x + w.x, y: c.rect.y + w.y, ..w }),
                    bar: d.height > 0.0,
                    identifier: c.foreign_toplevel_identifier.clone(),
                    own: c.pid == Some(std::process::id() as i32),
                }),
            ));
        }
    } else {
        let origin = (c.rect.x, c.rect.y - d.height * count as f64);
        children(c, origin, shown, mapping, out);
    }
}
