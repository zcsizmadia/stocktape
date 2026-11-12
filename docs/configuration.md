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
| `span` | One belt along the bottom of every display, flowing from screen to screen |
| `around` | One belt around the edges of the primary display |
| `around-desktop` | One belt around the outer edge of all displays (side by side) |

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
| `displays` | `"primary"`, `"all"`, an index (`0`, `1`, ...), or `"name:<text>"` | `"primary"` |
| `skin` | a skin name (see `--list-skins`) | `classic-dark` |
| `speed` | logical pixels per second | `120` |
| `offset` | distance from the screen edge, to stack banners | `0` |
| `inset` | `[start, end]`: space left free at the ends of the edge | `[0, 0]` |
| `reverse` | flow the other way | `false` |
| `strategy` | `demo`, `watchlist`, `movers` | `demo` |

Unknown keys and bad values are errors that name the banner and key.

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
- Banners currently float on top of other windows. Reserving screen space so
  maximized windows stay clear of them comes with docking (#8).
- `--skin NAME` overrides every banner's skin; `--skins-dir DIR` loads your own
  skins and applies edits live (see [skins.md](skins.md)).
