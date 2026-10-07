# sway-panorama

A live overview of Sway workspaces in one Wayland window. Each workspace is drawn in a rect
on an overview plane given in the config file. Windows are drawn schematically (title, app
icon, color per app) or with their live content.

![Nine workspaces in a 3×3 layout, windows drawn schematically](docs/screenshot.png)

Pointer controls:

- left click on a window or title bar: focus it, switching to its workspace
- left click elsewhere on a workspace: switch to that workspace (Sway creates a missing
  workspace on its assigned output, else on the focused output)
- left button drag (beyond 4 pixels): pan
- scroll wheel: zoom around the pointer
- right button, or the `keys.reset` key (default `0`): fit the plane to the window again
  (the default view)

Keys also toggle the display of workspace names (`w`), and cycle the content mode (`n`), the
icon mode (`i`), the title mode (window titles, app names or none, `t`) and the app color mode
(`c`); see `keys` below. These states last until the config file changes.

## Requirements

- Sway. The tree is read through Sway IPC when Sway reports a window, workspace, output or
  key binding event, and also every `poll_interval` milliseconds, because Sway sends no event
  for some changes, such as resizes, layout changes, borders and gaps set by commands.
- Content mode needs per-window capture (`ext-foreign-toplevel-image-capture-source-v1`),
  which Sway has since 1.12. With an older Sway, the program prints a message and draws
  windows schematically. Windows on hidden workspaces are captured live as well.
- libxkbcommon, and to build, its development package (`libxkbcommon-devel` on Fedora,
  `libxkbcommon-dev` on Debian and Ubuntu).

## Install

```sh
cargo install sway-panorama
```

Or, from a clone of the repository: `cargo build --release`.

The app icon is `data/sway-panorama.svg` in the repository. Programs that look up icons by
app_id in the icon theme, such as bars, launchers and sway-panorama itself, find it once it is
installed there:

```sh
install -Dm644 data/sway-panorama.svg ~/.local/share/icons/hicolor/scalable/apps/sway-panorama.svg
```

## Config

`$XDG_CONFIG_HOME/sway-panorama/config.ron` (default `~/.config/sway-panorama/config.ron`),
or the path given as the first argument. Changes to the file apply when it is saved; a file
with errors is reported and the previous config stays in effect.

The file is in [RON](https://github.com/ron-rs/ron): structs in parentheses, maps in braces,
optional values as `Some(...)`, comments after `//`. Every setting can be left out, which
gives the default shown below; a workspace rect needs all of `x`, `y`, `w` and `h`.

```ron
(
    poll_interval: 500,    // milliseconds; 0 disables polling
    margin: 0,             // plane units around the workspaces, measured like the space between them
    pixel_snap: false,     // round box edges, text and icons to whole pixels
    background: "#1e1e1e", // all colors are "#rrggbb" or "#rrggbbaa"
    keys: (                // xkb key names with optional Ctrl+, Alt+, Shift+, Super+ prefixes
        reset: "0",
        cycle_content: "n",    // cycles window.content: none, visible, all
        cycle_icons: "i",      // cycles window.icons: none, unlisted, all
        cycle_colors: "c",     // cycles window.app_colors: none, listed, all
        cycle_titles: "t",     // cycles window.titles: none, window, app
        toggle_workspace_names: "w",
    ),
    workspace: (
        border: 3,             // screen pixels, outside the workspace
        radius: 6,             // corner radius of the outer edge of the border, in screen pixels
        show_names: true,
        font_size: 16,         // font size of workspace labels, in screen pixels
        // font: Some("Noto Sans"),    // default: the system sans-serif font
        colors: (
            fill: "#2a2a2a",   // the area that windows can use
            bar: "#3a3a3a",    // the area the output reserves outside the workspace, such as a bar
            text: "#dddddd",   // workspace labels
            border: (
                normal: "#555555",
                visible: "#999999",    // shown on its output
                focused: "#4c9ad9",
                urgent: "#d03030",
            ),
        ),
        // The rect of each shown workspace, keyed by workspace name: position x, y and size
        // w, h in plane coordinates. The output of the workspace is mapped onto the rect, with
        // separate horizontal and vertical scales.
        rect: {
            "1": (x: 0, y: 0, w: 3840, h: 2160),
            "2": (x: 4080, y: 0, w: 3840, h: 2160),
        },
    ),
    window: (
        content: all,          // none: schematic, visible: capture visible workspaces only, all
        border: 2,             // screen pixels, inside the window
        radius: 6,             // corner radius of windows and title bars, in screen pixels
        gap: 0,                // space between windows, and to the workspace edge, in screen pixels
        icons: all,            // none, unlisted (only apps not in colors.app_hues) or all
        titles: window,        // none, window (window titles) or app (app names, the keys of
                               // colors.app_hues)
        app_colors: all,       // windows filled with their app color: none, listed (only apps
                               // in colors.app_hues) or all; the others are gray
        font_size: 16,         // font size of window titles, in screen pixels (not in title bars)
        // font: Some("Noto Sans"),    // default: the system sans-serif font
        icon_size: 48,         // size of app icons, in screen pixels
        icon_title_gap: 6,     // space between the icon and the title of a window, in screen pixels
        icon_padding: 4,       // space between the icon and the inner edge of the window border
        title_padding: 4,      // space between the title and the inner edge of the window border
        wrap_titles: true,     // wrap titles wider than their window
        shrink_titles: true,   // reduce the font size of titles that do not fit, after wrapping
        // icon_theme: Some("Adwaita"),    // default: the GTK icon theme
        colors: (
            app_lightness: 0.28,   // OKLab lightness of window fills: app colors, whose hue
                                   // comes from the app name, and the gray of windows without
                                   // app color
            app_chroma: 0.04,      // OKLab chroma of app colors
            app_hues: {},          // OKLCh hue in degrees per app_id (or X11 class), used instead
                                   // of the hue from the name, e.g. {"foot": 250}
            text: "#dddddd",       // window titles
            border: (              // also the fill of title bars
                lightness: 0.4,    // OKLab lightness of the normal border, with the chroma and
                                   // hue of the fill
                lightness_focused: 0.7,    // the same for the border of the focused window
                urgent: "#d03030",
                title_bar: "#2a2a2a",      // the border of title bars
            ),
        ),
    ),
)
```

Only the workspaces listed in `workspace.rect` are shown. A listed workspace that does not exist
is drawn like an existing workspace without windows that its output does not show: its fill, its
border in the `normal` color and its label.

The view fits the plane to the window, so only relative positions and sizes matter. With
`margin` equal to the space between workspaces, the space between the outer workspace borders
and the window edge equals the space between two workspace borders, on the axis that limits
the zoom; the other axis gets the remaining space. A rect with the aspect ratio of its output
shows the workspace undistorted; a rect with another aspect ratio stretches the workspace
along one axis.

With `window.app_colors` set to `all`, each window is filled with the color of its app; with
`listed`, only the windows of apps listed in `window.colors.app_hues`; with `none`, no window.
The other windows are gray of the same lightness. Each app has a color of the OKLCh lightness
`window.colors.app_lightness` and chroma `window.colors.app_chroma`. Its hue is the one listed
for the app in `window.colors.app_hues`, else it comes from a hash of the app name. The app name
is the Wayland app_id, or the X11 class (`app_id` and `window_properties.class` in
`swaymsg -t get_tree`); `window.titles: app`, cycled to by `t`, shows it in place of each
window title. Window borders and title bars have the hue and
chroma of the window fill, with the lightness `window.colors.border.lightness`, or
`lightness_focused` for the focused window.

All hues of app colors and their borders lie inside sRGB if `window.colors.app_chroma` is at
most 0.17 times the lowest of `window.colors.app_lightness`,
`window.colors.border.lightness` and `window.colors.border.lightness_focused`, all at most 0.75
(above, the limit falls: about 0.10 at 0.8, 0.05 at 0.9). For example, lightness 0.28, 0.4 and
0.7 allow chroma up to 0.047. Colors outside sRGB are clipped.

Labels, icons, borders, corner radii and the window gap keep their size in screen pixels at
every zoom; an icon shrinks only when its window is smaller on screen. Workspace borders lie
outside the workspace, and window borders lie inside the window, so without a gap, windows reach
the edges of their workspace. Both radii apply to the outer edge of a border; its inner edge has
the radius less the border width, and square corners where the border is wider than the radius.
For a workspace, the inner edge of the border is the edge of the workspace, to which its windows
are clipped. Captured window content is rectangular, so with `window.radius` larger than
`window.border`, its corners cover part of the rounded inner edge of the border. The icon and
the title of a window are centered together in it, at least `window.icon_padding` and
`window.title_padding` from the inner edge of the border. Title bars and their text scale with
the zoom, because they are part of the window geometry.

With `window.gap` set, windows are that many screen pixels apart, and the same distance from the
edge of their workspace. Each tile, a window with its title bar or a stacked or tabbed container
with its title bars and the window it shows, is reduced by half the gap on each side, by at most
a quarter of its size, and its content is scaled with it; each workspace is extended by half the
gap on each side.

Without `pixel_snap`, boxes, borders, text and icons are drawn at their real valued positions,
with anti-aliased edges. Text is placed at quarter pixels horizontally and at whole pixels
vertically (cosmic-text hinting). Captured window content is always placed at whole pixels,
because subsurfaces have integer positions and sizes; the image itself keeps its real valued
position and scale, and only its clipped edge lies on a pixel boundary.

## Sway setup

The window has app_id `sway-panorama`.

As a persistent window, placed with rules, for example:

```
for_window [app_id="sway-panorama"] floating enable, sticky enable, resize set 800 450
exec sway-panorama
```

As an overview toggled by a key, through the scratchpad:

```
for_window [app_id="sway-panorama"] move scratchpad, resize set 90 ppt 90 ppt
exec sway-panorama
bindsym $mod+Tab [app_id="sway-panorama"] scratchpad show
```

While the window is hidden, Sway sends it no frame callbacks, so window capture stops.

## License

MIT or Apache-2.0, at your option.
