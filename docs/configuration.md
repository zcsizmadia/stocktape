# Configuring banners

stocktape shows one or more **banners**. Each banner is one continuous belt of
quotes that can run along an edge of a display, span several displays, or go
all the way around. Every banner has its own skin, speed and strategy.

## Quick start: presets

```sh
stocktape --list-layouts
stocktape --layout top-bottom
stocktape --layout around --skin daylight
```

| Preset | What it does |
|---|---|
| `bottom` (default) | One banner along the bottom of the primary display |
| `top-bottom` | Movers on top, the watchlist along the bottom |
| `two-tier` | Two lines along the bottom: fast movers above the slower watchlist |
| `span` | One belt along the bottom of every landscape display, flowing from screen to screen |
| `around` | One belt around the edges of the primary display |
| `around-desktop` | One belt around the outer edge of all landscape displays (side by side) |

## Choosing monitors

`stocktape --list-displays` shows each display with its index, name,
position, size, scale and orientation:

```text
index  name                         position        size  scale  orientation
0      \\.\DISPLAY1                      0,0   1920x1080   1.00  landscape, primary
1      \\.\DISPLAY2                  -1920,0   1920x1080   1.00  landscape
2      \\.\DISPLAY3                1920,-251   1050x1680   1.00  portrait
```

Then pick displays in the banner:

```toml
displays = "landscape"                  # every landscape display
displays = ["DISPLAY2", "DISPLAY1"]     # exactly these (order on screen decides the flow)
displays = "all"
exclude = "portrait"                    # everything except portrait displays
```

Names are matched case-insensitively by substring and survive re-plugging and
reboots better than indexes. A listed display that is missing is skipped; the
banner is laid out on the ones that are there.

## Configuration file

For anything else, write a `stocktape.toml` and run `stocktape --config stocktape.toml`.
Each `[[banner]]` table adds a banner:

```toml
[[banner]]
name = "Indices"
place = "top"
skin = "terminal"
strategy = "movers"
speed = 90

[[banner]]
name = "Watchlist"
place = "bottom"
displays = "all"        # span every display
inset = [0, 320]        # keep the right end clear (e.g. for the clock)

[[banner]]
name = "Side"
place = "right"
displays = "name:DELL"  # by name, so it survives re-plugging
reverse = true          # flow downward instead of upward
```

| Key | Values | Default |
|---|---|---|
| `name` | any text, used in messages | `"Banner"` |
| `place` | `top`, `bottom`, `left`, `right`, `around` | `bottom` |
| `displays` | `"primary"`, `"all"`, `"landscape"`, `"portrait"`, an index, a display name (or part of one), or a list of these | `"primary"` |
| `exclude` | same forms as `displays`; displays to leave out | none |
| `skin` | a skin name (see `--list-skins`) | `classic-dark` |
| `speed` | logical pixels per second | `120` |
| `offset` | distance from the screen edge, to stack banners | `0` |
| `inset` | `[start, end]`: space left free at the ends of the edge | `[0, 0]` |
| `reverse` | flow the other way | `false` |
| `dock` | reserve screen space so maximized windows and the taskbar stay clear (edge banners) | `true` |
| `strategy` | `demo`, `watchlist`, `movers` | `demo` |

Unknown keys and bad values are errors that name the banner and key.

### Lines: several belts in one banner

A banner can stack several lines, each scrolling at its own speed with its own
skin and strategy. Add `[[banner.line]]` tables after the banner; a line
inherits the banner's `skin`, `speed` and `strategy` unless it sets its own:

```toml
[[banner]]
name = "Two-tier"
place = "bottom"

[[banner.line]]             # top line
strategy = "movers"
speed = 180

[[banner.line]]             # bottom line
skin = "daylight"
strategy = "watchlist"
speed = 90
```

Each line is as tall as its skin, and the banner is as tall as its lines
together (docking reserves the full height). All lines share the banner's
placement, so they cross monitors and go around corners together.

### How banners behave

- **Spanning:** when `displays` picks several displays for an edge, one belt
  runs across them in screen order. Each display keeps its own scaling, so
  text has the same physical size everywhere.
- **Around:** the belt runs counter-clockwise (leftward along the top, down the
  left side, rightward along the bottom, up the right side); `reverse = true`
  makes it clockwise. Side edges show rotated text. Around several displays
  needs them side by side in one row.
- **Thickness** comes from the skin.
- **Display changes** (plugging, unplugging, rearranging, rescaling) are
  picked up within a couple of seconds and the banners are laid out again.
- **Docking:** edge banners reserve their strip of the screen, like a taskbar.
  Maximized windows stop at the banner, and on Windows the banner sits above
  the taskbar. Space is given back when stocktape exits.
  - Windows: AppBar. Linux X11: `_NET_WM_STRUT_PARTIAL` (most window managers
    honour it).
  - Wayland and macOS: not yet supported (macOS has no way to reserve space);
    banners float on top instead, with a message saying so.
  - `around` banners always float on top. Set `dock = false` to make any
    banner float.
- `--skin NAME` overrides every banner's skin; `--skins-dir DIR` loads your own
  skins and applies edits live (see [skins.md](skins.md)).
