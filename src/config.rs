use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer};
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use xkbcommon::xkb;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    None,
    Visible,
    All,
}

#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Color(pub [u8; 4]);

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let hex = s.strip_prefix('#').unwrap_or(&s);
        let v = u32::from_str_radix(hex, 16).map_err(serde::de::Error::custom)?;
        match hex.len() {
            6 => Ok(Color([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])),
            8 => Ok(Color([(v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8])),
            _ => Err(serde::de::Error::custom("expected #rrggbb or #rrggbbaa")),
        }
    }
}

/// A key with modifiers, written as an xkb key name with optional modifier prefixes, such as
/// "Ctrl+0". Shift is required only if named, because it already selects the key name; the case
/// of characters is ignored, so that Caps Lock does not change the key.
#[derive(Clone, Copy, Debug)]
pub struct Key {
    keysym: Keysym,
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
}

impl Key {
    fn parse(s: &str) -> Result<Self, String> {
        let mut parts: Vec<&str> = s.split('+').collect();
        let name = parts.pop().unwrap_or_default();
        let mut keysym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        if keysym == Keysym::NoSymbol {
            keysym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
        }
        if keysym == Keysym::NoSymbol {
            return Err(format!("unknown key name `{name}`"));
        }
        let mut key = Key { keysym, ctrl: false, alt: false, shift: false, logo: false };
        for m in parts {
            match m.to_lowercase().as_str() {
                "ctrl" | "control" => key.ctrl = true,
                "alt" => key.alt = true,
                "shift" => key.shift = true,
                "super" | "logo" => key.logo = true,
                _ => return Err(format!("unknown modifier `{m}`")),
            }
        }
        Ok(key)
    }

    pub fn matches(&self, keysym: Keysym, m: &Modifiers) -> bool {
        let same = keysym == self.keysym
            || matches!((keysym.key_char(), self.keysym.key_char()), (Some(a), Some(b)) if a.to_lowercase().eq(b.to_lowercase()));
        same && m.ctrl == self.ctrl && m.alt == self.alt && m.logo == self.logo && (m.shift || !self.shift)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Key::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

const NORMAL: Color = Color([0x55, 0x55, 0x55, 0xff]);
const VISIBLE: Color = Color([0x99, 0x99, 0x99, 0xff]);
const FOCUSED: Color = Color([0x4c, 0x9a, 0xd9, 0xff]);
const URGENT: Color = Color([0xd0, 0x30, 0x30, 0xff]);

/// Border colors of workspaces; a workspace is visible when its output shows it.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceBorderColors {
    pub normal: Color,
    pub visible: Color,
    pub focused: Color,
    pub urgent: Color,
}

impl Default for WorkspaceBorderColors {
    fn default() -> Self {
        WorkspaceBorderColors { normal: NORMAL, visible: VISIBLE, focused: FOCUSED, urgent: URGENT }
    }
}

/// Border colors of windows, also used to fill their title bars.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WindowBorderColors {
    pub focused: Color,
    pub urgent: Color,
    /// OKLab lightness of the border in the normal state, which has the chroma and hue of the
    /// window fill.
    pub lightness: f64,
}

impl Default for WindowBorderColors {
    fn default() -> Self {
        WindowBorderColors { focused: FOCUSED, urgent: URGENT, lightness: 0.4 }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub background: Color,
    pub workspace: Color,
    /// The area that the output reserves outside the workspace, such as a bar.
    pub bar: Color,
    /// OKLab lightness of window fills: app colors, whose hue comes from a hash of the app name,
    /// and the gray of windows without app color.
    pub app_lightness: f64,
    /// OKLab chroma of app colors.
    pub app_chroma: f64,
    /// Degrees added to the hue of every app color.
    pub app_hue_offset: f64,
    pub text: Color,
    pub workspace_border: WorkspaceBorderColors,
    pub window_border: WindowBorderColors,
}

impl Default for Colors {
    fn default() -> Self {
        Colors {
            background: Color([0x1e, 0x1e, 0x1e, 0xff]),
            workspace: Color([0x2a, 0x2a, 0x2a, 0xff]),
            bar: Color([0x3a, 0x3a, 0x3a, 0xff]),
            app_lightness: 0.28,
            app_chroma: 0.04,
            app_hue_offset: 0.0,
            text: Color([0xdd, 0xdd, 0xdd, 0xff]),
            workspace_border: WorkspaceBorderColors::default(),
            window_border: WindowBorderColors::default(),
        }
    }
}

/// Keyboard shortcuts.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// Resets the view to the fit of the plane.
    pub reset: Key,
    pub toggle_icons: Key,
    /// Toggles `app_colors`.
    pub toggle_colors: Key,
    pub toggle_titles: Key,
    pub toggle_workspace_names: Key,
}

impl Default for Keys {
    fn default() -> Self {
        let key = |s| Key::parse(s).unwrap();
        Keys {
            reset: key("0"),
            toggle_icons: key("i"),
            toggle_colors: key("c"),
            toggle_titles: key("t"),
            toggle_workspace_names: key("w"),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub content: Content,
    /// Interval of polling the sway tree, in milliseconds, for changes that sway reports with
    /// no IPC event, such as resizes and layout changes.
    pub poll_interval: u64,
    /// Font family of labels and titles; the default is the system sans-serif font.
    pub font: Option<String>,
    /// Height of workspace labels and window titles, in screen pixels.
    pub label_size: f64,
    /// Size of app icons, in screen pixels.
    pub icon_size: f64,
    /// Space between the icon and the title of a window, in screen pixels.
    pub icon_title_gap: f64,
    /// Width of workspace borders, outside the workspace, in screen pixels.
    pub workspace_border: f64,
    /// Width of window borders, inside the window, in screen pixels.
    pub window_border: f64,
    /// Space around the workspaces in plane units, measured like the space between two
    /// workspaces: with the margin equal to that space, the space between a workspace border and
    /// the window edge equals the space between two workspace borders.
    pub margin: f64,
    /// Round the edges of boxes and the positions of text and icons to whole pixels.
    pub pixel_snap: bool,
    pub show_icons: bool,
    pub show_titles: bool,
    pub show_workspace_names: bool,
    /// Fill each window with a color derived from its app, else with gray of the same lightness.
    pub app_colors: bool,
    /// Wrap window titles that are wider than their window.
    pub wrap_titles: bool,
    /// Reduce the font size of window titles that do not fit their window, after wrapping.
    pub shrink_titles: bool,
    pub icon_theme: Option<String>,
    pub keys: Keys,
    pub colors: Colors,
    /// Placement of each shown workspace, keyed by workspace name.
    pub workspace: BTreeMap<String, Placement>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            content: Content::All,
            poll_interval: 500,
            font: None,
            label_size: 16.0,
            icon_size: 48.0,
            icon_title_gap: 6.0,
            workspace_border: 3.0,
            window_border: 2.0,
            margin: 0.0,
            pixel_snap: false,
            show_icons: true,
            show_titles: true,
            show_workspace_names: true,
            app_colors: true,
            wrap_titles: true,
            shrink_titles: true,
            icon_theme: None,
            keys: Keys::default(),
            colors: Colors::default(),
            workspace: BTreeMap::new(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
    });
    base.join("sway-panorama/config.toml")
}

pub fn load(path: &std::path::Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}
