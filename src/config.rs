use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Deserializer};

use crate::color::Color;
use smithay_client_toolkit::seat::keyboard::{Keysym, Modifiers};
use xkbcommon::xkb;

#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Content {
    None,
    Visible,
    All,
}

/// Which windows are filled with the color of their app; the others are gray.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum AppColors {
    None,
    /// Only the apps listed in `window.colors.app_hues`.
    Listed,
    All,
}

/// Which windows show the icon of their app.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Icons {
    None,
    /// Only the apps not listed in `window.colors.app_hues`.
    Unlisted,
    All,
}

/// What windows and title bars show as their title.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Titles {
    None,
    /// The window title.
    Window,
    /// The app name (app_id, or X11 class), the name that `window.colors.app_hues` is keyed by.
    App,
}

/// A mode that a key cycles through its values in declaration order.
pub trait Cycle: Copy + PartialEq + 'static {
    const VALUES: &'static [Self];

    fn next(self) -> Self {
        let i = Self::VALUES.iter().position(|&v| v == self).unwrap();
        Self::VALUES[(i + 1) % Self::VALUES.len()]
    }
}

impl Cycle for Content {
    const VALUES: &'static [Self] = &[Self::None, Self::Visible, Self::All];
}

impl Cycle for AppColors {
    const VALUES: &'static [Self] = &[Self::None, Self::Listed, Self::All];
}

impl Cycle for Icons {
    const VALUES: &'static [Self] = &[Self::None, Self::Unlisted, Self::All];
}

impl Cycle for Titles {
    const VALUES: &'static [Self] = &[Self::None, Self::Window, Self::App];
}

/// A rect on the overview plane: position `x`, `y` and size `w`, `h`.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
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

const TEXT: Color = Color([0xdd, 0xdd, 0xdd, 0xff]);
const FILL: Color = Color([0x2a, 0x2a, 0x2a, 0xff]);

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

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceColors {
    /// The area of the workspace that windows can use.
    pub fill: Color,
    /// The area that the output reserves outside the workspace, such as a bar.
    pub bar: Color,
    /// Workspace labels.
    pub text: Color,
    pub border: WorkspaceBorderColors,
}

impl Default for WorkspaceColors {
    fn default() -> Self {
        WorkspaceColors {
            fill: FILL,
            bar: Color([0x3a, 0x3a, 0x3a, 0xff]),
            text: TEXT,
            border: WorkspaceBorderColors::default(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspaceConfig {
    /// Width of workspace borders, outside the workspace, in screen pixels.
    pub border: f64,
    /// Corner radius of the outer edge of workspace borders, in screen pixels. The workspace
    /// inside the border, to which its windows are clipped, has the radius less the border width.
    pub radius: f64,
    pub show_names: bool,
    /// Font family of workspace labels; the default is the system sans-serif font.
    pub font: Option<String>,
    /// Font size of workspace labels, in screen pixels.
    pub font_size: f64,
    pub colors: WorkspaceColors,
    /// The rect of each shown workspace on the overview plane, keyed by workspace name. The
    /// output of the workspace is mapped onto it, with separate horizontal and vertical scales.
    pub rect: BTreeMap<String, Placement>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        WorkspaceConfig {
            border: 3.0,
            radius: 6.0,
            show_names: true,
            font: None,
            font_size: 16.0,
            colors: WorkspaceColors::default(),
            rect: BTreeMap::new(),
        }
    }
}

/// Border colors of windows, also used to fill their title bars.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WindowBorderColors {
    /// OKLab lightness of the border in the normal state, which has the chroma and hue of the
    /// window fill.
    pub lightness: f64,
    /// OKLab lightness of the border of the focused window, with the chroma and hue of the window
    /// fill.
    pub lightness_focused: f64,
    pub urgent: Color,
    /// The border of title bars.
    pub title_bar: Color,
}

impl Default for WindowBorderColors {
    fn default() -> Self {
        WindowBorderColors { lightness: 0.4, lightness_focused: 0.7, urgent: URGENT, title_bar: FILL }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WindowColors {
    /// OKLab lightness of window fills: app colors, whose hue comes from a hash of the app name,
    /// and the gray of windows without app color.
    pub app_lightness: f64,
    /// OKLab chroma of app colors.
    pub app_chroma: f64,
    /// OKLCh hue in degrees of the color of each listed app, keyed by app_id (or X11 class),
    /// instead of the hue from the app name.
    pub app_hues: BTreeMap<String, f64>,
    /// Window titles.
    pub text: Color,
    pub border: WindowBorderColors,
}

impl Default for WindowColors {
    fn default() -> Self {
        WindowColors {
            app_lightness: 0.28,
            app_chroma: 0.04,
            app_hues: BTreeMap::new(),
            text: TEXT,
            border: WindowBorderColors::default(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct WindowConfig {
    pub content: Content,
    /// Width of window borders, inside the window, in screen pixels.
    pub border: f64,
    /// Corner radius of windows and title bars, at the outer edge of their border, in screen
    /// pixels. The inner edge of the border has the radius less the border width.
    pub radius: f64,
    /// Space between windows, and between windows and the edge of their workspace, in screen
    /// pixels. Each tile (a window with its title bar, or a stacked or tabbed container) is
    /// reduced by half of it on each side, by at most a quarter of its size, with its content
    /// scaled; each workspace is extended by half of it on each side.
    pub gap: f64,
    pub icons: Icons,
    pub titles: Titles,
    /// Which windows are filled with the color of their app; the others are gray of the same
    /// lightness.
    pub app_colors: AppColors,
    /// Font family of window titles; the default is the system sans-serif font.
    pub font: Option<String>,
    /// Font size of window titles, in screen pixels. Titles in title bars scale with the zoom
    /// instead, because title bars are part of the window geometry.
    pub font_size: f64,
    /// Size of app icons, in screen pixels.
    pub icon_size: f64,
    /// Space between the icon and the title of a window, in screen pixels.
    pub icon_title_gap: f64,
    /// Space between app icons and the inner edge of the window border, in screen pixels.
    pub icon_padding: f64,
    /// Space between window titles and the inner edge of the window border, in screen pixels.
    pub title_padding: f64,
    pub icon_theme: Option<String>,
    /// Wrap window titles that are wider than their window.
    pub wrap_titles: bool,
    /// Reduce the font size of window titles that do not fit their window, after wrapping.
    pub shrink_titles: bool,
    pub colors: WindowColors,
}

impl Default for WindowConfig {
    fn default() -> Self {
        WindowConfig {
            content: Content::All,
            border: 2.0,
            radius: 6.0,
            gap: 0.0,
            icons: Icons::All,
            titles: Titles::Window,
            app_colors: AppColors::All,
            font: None,
            font_size: 16.0,
            icon_size: 48.0,
            icon_title_gap: 6.0,
            icon_padding: 4.0,
            title_padding: 4.0,
            icon_theme: None,
            wrap_titles: true,
            shrink_titles: true,
            colors: WindowColors::default(),
        }
    }
}

/// Keyboard shortcuts.
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    /// Resets the view to the fit of the plane.
    pub reset: Key,
    /// Cycles `window.content` through none, visible and all.
    pub cycle_content: Key,
    /// Cycles `window.icons` through none, unlisted and all.
    pub cycle_icons: Key,
    /// Cycles `window.app_colors` through none, listed and all.
    pub cycle_colors: Key,
    /// Cycles `window.titles` through none, window and app.
    pub cycle_titles: Key,
    pub toggle_workspace_names: Key,
}

impl Default for Keys {
    fn default() -> Self {
        let key = |s| Key::parse(s).unwrap();
        Keys {
            reset: key("0"),
            cycle_content: key("n"),
            cycle_icons: key("i"),
            cycle_colors: key("c"),
            cycle_titles: key("t"),
            toggle_workspace_names: key("w"),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Interval of polling the sway tree, in milliseconds, for changes that sway reports with
    /// no IPC event, such as resizes and layout changes.
    pub poll_interval: u64,
    /// Space around the workspaces in plane units, measured like the space between two
    /// workspaces: with the margin equal to that space, the space between a workspace border and
    /// the window edge equals the space between two workspace borders.
    pub margin: f64,
    /// Round the edges of boxes and the positions of text and icons to whole pixels.
    pub pixel_snap: bool,
    pub background: Color,
    pub keys: Keys,
    pub workspace: WorkspaceConfig,
    pub window: WindowConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            poll_interval: 500,
            margin: 0.0,
            pixel_snap: false,
            background: Color([0x1e, 0x1e, 0x1e, 0xff]),
            keys: Keys::default(),
            workspace: WorkspaceConfig::default(),
            window: WindowConfig::default(),
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
    });
    base.join("sway-panorama/config.ron")
}

pub fn load(path: &std::path::Path) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}
