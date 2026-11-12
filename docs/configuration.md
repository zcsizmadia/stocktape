# Configuring banners

stocktape shows one or more **banners**. Each banner is one continuous belt of
quotes that can run along an edge of a display, span several displays, or go
all the way around. Every banner has its own skin, speed and strategy.

## The settings window

```sh
stocktape --settings
```

opens a window to edit everything on this page without writing TOML: banners
and their lines (skin, speed, what they show), where they go and on which
displays, feeds, the watchlist, and skins with a live preview.

- It edits the standard configuration file: `%APPDATA%stocktapestocktape.toml`
  on Windows, `~/.config/stocktape/stocktape.toml` on Linux,
  `~/Library/Application Support/stocktape/stocktape.toml` on macOS (or the
  file given with `--config`). A new file starts from `--layout` (default
  `bottom`).
- **Save** writes the file and keeps the previous one as `stocktape.toml.bak`.
  Comments in a hand-written file are not kept.
- **Running banners update by themselves** when the file changes, whether
  saved from the window or edited by hand. A file with a mistake is reported
  and the running banners stay as they were.
- API keys are never shown or written; each feed names the environment
  variable that holds its key, and shows whether it is set.

Without `--config` or `--layout`, `stocktape` uses the standard file if it
exists.

**Portable mode:** a `stocktape.toml` in the same folder as the `stocktape`
executable is used instead of the standard file, so a copy on a USB stick keeps
its settings with it.

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

## Real data and your watchlist

Without a `[[feed]]`, stocktape shows sample prices. To show real quotes,
add a feed and a watchlist:

```toml
[[feed]]
provider = "finnhub"             # https://finnhub.io; free accounts work
# api_key_env = "FINNHUB_API_KEY" (default): the key is read from this
# environment variable. You can also write api_key = "..." here, but keep
# the file private if you do.

refresh = 300                    # seconds between snapshots (min 10; default 300)
stream = true                    # live trades; false = snapshots only, every `refresh`

[feed.symbols]                   # optional: Finnhub names that differ
"BTC-USD" = "BINANCE:BTCUSDT"     # (crypto -USD pairs map like this already)

[[watchlist]]
title = "Indices"
symbols = ["SPY", "QQQ", "DIA"]

[[watchlist]]
title = "Tech"
symbols = ["AAPL", "MSFT", "NVDA"]
```

- Set the key once, e.g. on Windows `setx FINNHUB_API_KEY your-key` (new
  terminals pick it up), or `export FINNHUB_API_KEY=your-key` on Linux/macOS.
- **How often prices update:** with `stream = true` (default) every trade is
  pushed as it happens, and snapshots (which also refresh the previous close)
  come every `refresh` seconds. With `stream = false` prices only update from
  snapshots, every `refresh` seconds: calmer, and fewer moving numbers.
  Requests are always spaced about 1.1 s apart, so no setting exceeds the free
  tier's limit.
- Prices come from Finnhub snapshots (previous close, for the change) and its
  live trade stream. Free accounts allow 60 requests a minute and 50 streamed
  symbols, and some exchanges are delayed or paid; check Finnhub's terms.
- **Massive** (massive.com, formerly Polygon.io) is the second provider:

  ```toml
  [[feed]]
  provider = "massive"           # key from MASSIVE_API_KEY (or api_key_env)
  refresh = 60                   # default 60
  requests_per_minute = 5        # your plan's limit; free plan: 5
  ```

  It batches the whole watchlist into one snapshot request and paces every
  request to `requests_per_minute`. Snapshots need Stocks Starter or higher
  (15-minute delayed on Starter and Developer, real-time on Advanced). On the
  free Basic plan, stocktape notices that and shows end-of-day prices (the
  previous day's close), one symbol per request. US stocks only for now.
  A typical setup is Finnhub first (live) and Massive second as the backup.
- **Several feeds:** list more `[[feed]]` tables. The first is the primary;
  a later feed takes over a symbol only when the ones before it have been
  quiet for that symbol for 30 s, and hands it back when they recover.
- Quotes not refreshed for 15 minutes (for example while a feed is down) are
  drawn faded.
- Feed problems are printed once each (connected, disconnected, retrying,
  a rejected key). API keys never appear in messages.
- stocktape never stores or redistributes market data.

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
| `strategy` | how quotes are picked: `demo`, `watchlist`, `movers` | `demo` |
| `content` | `quotes`, `headlines`, `mixed` | `quotes` |
| `messages` | your own text messages, mixed in now and then | none |

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

### Display names, units and decimals (commodities, FX, rates)

`[labels]` changes how particular symbols are shown, which is handy for
commodities, currencies and rates whose provider symbols are cryptic:

```toml
[labels]
"OANDA:XAU_USD" = { name = "GOLD", unit = "/oz" }      # GOLD 3,412.20/oz
"OANDA:EUR_USD" = { name = "EUR/USD", decimals = 4 }   # EUR/USD 1.0832 +0.0032
"SPY" = "S&P 500 ETF"                                  # just a name
```

- `name` replaces the ticker, `unit` is appended to the price, and
  `decimals` (0 to 10) fixes the precision of the price and change.
- Which commodities and currencies you can get depends on your data
  provider and plan; use the provider's symbol (see `[feed.symbols]`).
- The settings window edits these under Watchlist → Display names.

### Headlines and messages

A line can show quotes, news headlines, or both, plus your own messages:

```toml
[[news]]                           # RSS or Atom feeds
url = "https://example.com/markets.rss"
name = "Markets"                   # optional label; the feed's title otherwise
refresh = 300                      # seconds between fetches (min 60)

[[banner]]
place = "bottom"

[[banner.line]]
content = "mixed"                  # quotes (default), headlines, or mixed
messages = ["Markets close at 4 PM"]

[[banner.line]]
skin = "daylight"
content = "headlines"
speed = 70
```

- `mixed` shows a headline after every few quotes; `headlines` shows only
  headlines, newest first, cycling through them; messages appear now and
  then on any line.
- Headlines show the source and the local time they were published, e.g.
  `MARKETS · 10:42  Stocks rise as inflation cools`. Headlines older than a
  day are dropped, and the same story from two feeds is shown once.
- Many publishers offer RSS feeds for personal reading; check each feed's
  terms. stocktape doesn't scrape web pages.

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
