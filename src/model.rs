use std::collections::HashMap;

use crate::{
    config::{Config, Placement},
    sway::{Node, Rect},
};

pub struct Overview {
    pub name: String,
    /// The output area of the workspace, in world coordinates.
    pub rect: Rect,
    /// The part of `rect` that windows can use; the rest is reserved, for example by a bar.
    pub usable: Rect,
    /// False for a configured workspace that does not exist.
    pub exists: bool,
    pub visible: bool,
    pub focused: bool,
    pub urgent: bool,
    /// Title bars and windows in drawing order, in layers: the tiling layer first, then one
    /// layer for each floating container.
    pub layers: Vec<Vec<Element>>,
}

pub enum Element {
    Bar(Bar),
    Window(Window),
}

pub struct Bar {
    /// The sway id of the container the bar belongs to.
    pub id: i64,
    pub rect: Rect,
    pub title: String,
    /// The app of the window the bar belongs to; `None` for a container of several windows.
    pub app: Option<String>,
    pub focused: bool,
    pub urgent: bool,
}

pub struct Window {
    /// The sway id of the window's container.
    pub id: i64,
    /// The container rect, including borders.
    pub frame: Rect,
    /// The area of the window content.
    pub content: Rect,
    pub title: String,
    pub app: Option<String>,
    pub focused: bool,
    pub urgent: bool,
    /// True if sway draws a title bar for the window.
    pub bar: bool,
    pub identifier: Option<String>,
    /// True for the window of this program.
    pub own: bool,
}

/// Maps layout coordinates of one output to world coordinates of one workspace overview.
struct Mapping {
    origin: (f64, f64),
    place: Placement,
}

impl Mapping {
    fn map(&self, r: Rect) -> Rect {
        let s = self.place.scale;
        Rect {
            x: self.place.x + (r.x - self.origin.0) * s,
            y: self.place.y + (r.y - self.origin.1) * s,
            width: r.width * s,
            height: r.height * s,
        }
    }
}

/// Builds the overviews of the configured workspaces.
///
/// `sizes` keeps the last output size of each workspace, used for placeholders.
pub fn build(tree: &Node, config: &Config, sizes: &mut HashMap<String, (f64, f64)>) -> Vec<Overview> {
    let outputs: Vec<&Node> = tree.nodes.iter().filter(|o| o.name.as_deref() != Some("__i3")).collect();
    let default_size = outputs.first().map(|o| (o.rect.width, o.rect.height)).unwrap_or((1920.0, 1080.0));
    let mut found = HashMap::new();
    for output in &outputs {
        for ws in &output.nodes {
            if let Some(name) = &ws.name {
                found.insert(name.as_str(), (*output, ws));
            }
        }
    }

    config
        .workspace
        .iter()
        .map(|(name, &place)| match found.get(name.as_str()) {
            Some(&(output, ws)) => {
                sizes.insert(name.clone(), (output.rect.width, output.rect.height));
                let mapping = Mapping { origin: (output.rect.x, output.rect.y), place };
                let mut layers = vec![Vec::new()];
                match find_fullscreen(ws) {
                    Some(fs) => add(fs, (fs.rect.x, fs.rect.y), 1, true, &mapping, &mut layers[0]),
                    None => {
                        let origin = (ws.rect.x, ws.rect.y);
                        children(ws, origin, true, &mapping, &mut layers[0]);
                        for f in &ws.floating_nodes {
                            let mut layer = Vec::new();
                            add(f, origin, 1, true, &mapping, &mut layer);
                            layers.push(layer);
                        }
                    }
                }
                Overview {
                    name: name.clone(),
                    rect: mapping.map(output.rect),
                    usable: mapping.map(ws.rect),
                    exists: true,
                    visible: output.focus.first() == Some(&ws.id),
                    focused: contains_focus(ws),
                    urgent: ws.urgent,
                    layers,
                }
            }
            None => {
                let (w, h) = sizes.get(name).copied().unwrap_or(default_size);
                let rect = Rect { x: place.x, y: place.y, width: w * place.scale, height: h * place.scale };
                Overview {
                    name: name.clone(),
                    rect,
                    usable: rect,
                    exists: false,
                    visible: false,
                    focused: false,
                    urgent: false,
                    layers: vec![Vec::new()],
                }
            }
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
    for child in &node.nodes {
        let front = !(stacked || tabbed) || node.focus.first() == Some(&child.id);
        add(child, origin, count, shown && front, mapping, out);
    }
}

/// Adds the title bar, window and descendants of container `c`.
///
/// `parent_origin` is the box origin of the parent, to which sway's `deco_rect` is relative.
/// `count` is the number of title bars above the content of `c` (siblings in a stacked parent).
fn add(c: &Node, parent_origin: (f64, f64), count: usize, shown: bool, mapping: &Mapping, out: &mut Vec<Element>) {
    let d = c.deco_rect;
    let app = c.pid.and(c.app_id.clone().or_else(|| c.window_properties.as_ref()?.class.clone()));
    if d.height > 0.0 {
        out.push(Element::Bar(Bar {
            id: c.id,
            rect: mapping.map(Rect { x: parent_origin.0 + d.x, y: parent_origin.1 + d.y, ..d }),
            title: c.name.clone().unwrap_or_default(),
            app: app.clone(),
            focused: contains_focus(c),
            urgent: c.urgent,
        }));
    }
    if c.pid.is_some() {
        if shown {
            let w = c.window_rect;
            out.push(Element::Window(Window {
                id: c.id,
                frame: mapping.map(c.rect),
                content: mapping.map(Rect { x: c.rect.x + w.x, y: c.rect.y + w.y, ..w }),
                title: c.name.clone().unwrap_or_default(),
                app,
                focused: c.focused,
                urgent: c.urgent,
                bar: d.height > 0.0,
                identifier: c.foreign_toplevel_identifier.clone(),
                own: c.pid == Some(std::process::id() as i32),
            }));
        }
    } else {
        let origin = (c.rect.x, c.rect.y - d.height * count as f64);
        children(c, origin, shown, mapping, out);
    }
}
