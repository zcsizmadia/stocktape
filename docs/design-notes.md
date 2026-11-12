# stocktape: design notes

A broadcast-style scrolling stock quote bar for the desktop, in the spirit of
the business-TV tickers (two-tier crawl, green ▲ / red ▼), written in Rust.
MIT licensed. Cross-platform: Windows, Linux and macOS.

## Goals

- Smooth, TV-quality horizontal crawl of selected tickers.
- Configurable rules for *when* and *how* a ticker appears (pluggable strategies).
- Skins (colors, fonts, layout tiers, arrows, separators, logo slot).
- Pluggable market-data sources; users bring their own API key.

## Features

- **Rust throughout.** Modern, idiomatic Rust (stable toolchain, 2024 edition),
  a Cargo workspace with small, focused crates.
- **Multi-OS.** Windows, Linux (X11 and Wayland) and macOS as first-class
  targets, built and tested in CI on all three. Platform-specific code (docking,
  space reservation, monitor queries) sits behind a `Platform` trait.
- **Multi-monitor.**
  - Enumerate displays and react to hot-plug, resolution, DPI and arrangement
    changes at runtime.
  - Per-monitor DPI scaling and refresh rate; each banner animates in step with
    its own display's vsync.
  - Choose displays by stable identity (name, position, primary), not by index,
    so the configuration survives reboots and re-plugging.
- **Multi-banner.** Any number of independent banners, each with its own
  watchlist, strategy, skin, speed and feed:
  - Top and/or bottom edge of any display (e.g. indices on top, movers below).
  - Left/right edges with vertical scrolling or rotated text.
  - **Span across displays:** one continuous belt that flows from one monitor
    into the next, handling different DPI and refresh rates and gaps between
    screens.
  - **Perimeter ("around the display") mode:** a single belt that travels
    top → right → bottom → left around one display, or around the outer edge
    of the whole multi-monitor desktop.
  - Banners can be docked (reserve screen space) or overlay (click-through,
    always on top).
- **Customizable layouts (as far as possible).** Everything about a banner is
  configuration, not code:
  - Placement: any edge or edges, any display or displays, one belt path or
    several; edge order and direction (clockwise or counter-clockwise, left or
    right scroll); offsets, margins, length (full edge or a fraction), and
    alignment.
  - Geometry: height or thickness, number of tiers and each tier's height,
    pinned or flipper boxes (left, right or both), clock and status slots.
  - Behavior: speed, easing at segment joins, pause on hover, opacity,
    click-through, docked or overlay, auto-hide (e.g. outside market hours or
    when an app is fullscreen).
  - Text on side edges: rotated or stacked vertically.
  - Named layout presets (e.g. "two-tier bottom", "perimeter", "span all")
    that users can copy and change, and switch between at runtime or by
    schedule.
- **Skin styles, built for iteration.** Skins are a separate layer from layout
  so new looks can be added without touching code:
  - Each skin is a versioned, declarative file (TOML) with design tokens
    (colors, fonts, sizes, radii, spacing, shadows) plus per-element styles
    (symbol, price, change, separator, pinned box, tiers).
  - Skins can extend another skin and override only what changes, so
    variants are cheap.
  - Style families out of the box: Classic Dark (two-tier broadcast), Daylight
    (light, pill badges), Terminal (monospace phosphor) and Glass
    (translucent, sparklines). More can follow.
  - Optional elements per skin: sparklines, flags or exchange badges,
    logos (user-supplied), price-change flash, fade-in.
  - A user skins folder, hot reload, and a skin preview or gallery mode for
    designing new ones. Golden-image tests per skin catch regressions.
- **Multiple feeds.** Several `QuoteSource` providers can run at once (e.g.
  crypto from one provider, equities from another), with symbol mapping,
  failover, rate-limit handling and a stale-quote indicator. Mock and replay
  sources ship for demos and tests.
- **Global markets.** Foreign exchanges alongside US ones in the same belt,
  e.g. LSE, Euronext, XETRA, TSE (Tokyo), HKEX, TSX, ASX:
  - Symbols carry an exchange. The canonical form is `EXCHANGE:TICKER` with
    ISO 10383 MIC codes (e.g. `XLON:VOD`, `XTKS:7203`, `XETR:SAP`); a colon,
    not a dot suffix, because tickers like `BRK.B` contain dots. Feed adapters
    map to each provider's naming (`VOD.L`, `7203.T`, `SAP.DE`).
  - Each exchange has its own time zone, trading hours, holidays and
    open/closed status, which strategies can use (e.g. show Asian markets
    overnight in the US, Europe at the US open).
  - Prices are shown in local currency, with an option to convert to a chosen
    currency using FX rates from a feed; currency symbols and number formats
    follow the market.
  - Optional exchange or country badge per item, and grouping by region with
    header tiles ("EUROPE", "ASIA").
  - Different providers can cover different regions, routed by exchange.
- **Modern design.** Clean typography, smooth sub-pixel motion, subtle
  animations (price flash on change, fade-in of new items), light and dark
  skins, HiDPI-sharp text. Broadcast-inspired skins with neutral names.
- **Fully testable.** Every layer is testable without a real network, display or
  wall clock (see [Testing](#testing)).

## Landscape (why build it)

- Broadcast systems (Vizrt, Ross XPression, Chyron, Avid Maestro, Wasp3D) do
  this properly but cost five to six figures and need playout hardware.
- Cheaper broadcast-style tools: CasparCG (open source, HTML templates),
  Singular.live, NewBlue Captivate, H2R Graphics.
- Desktop and web: TradingView Ticker Tape widget, Rainmeter skins, macOS
  menu-bar apps (TickerBar, Market Ticker), signage apps (ScreenCloud, Yodeck,
  OptiSigns).
- Terminal tools: tickrs (Rust), ticker (Go), mop (Go).
- Gap: nothing affordable combines a broadcast-quality crawl, skins and
  rule-based ticker scheduling.

## Architecture

```
QuoteSource(s) ──► QuoteStore ──► Strategy/Scheduler ──► Belt planner ──► Renderer
 (tokio, WS/REST)   (latest+history)  (decides what/when)    (fills ahead of    (wgpu, constant
                                                              the viewport)      px/sec scroll)
```

- **Banners and layout:**
  - A `BannerLayout` maps each banner to one or more *edge segments*, each a
    (display, edge, rect) triple in virtual-desktop coordinates.
  - A spanning or perimeter belt is one logical path; each segment is a window
    showing its slice of that path. Position on the path is shared, so content
    passes seamlessly from one segment to the next.
  - All banners share one wgpu device and glyph cache; each segment has its own
    surface.
  - Layout is a pure function of (displays, config), so it can be unit tested
    against fake monitor sets.
- **Window:** winit, borderless, always on top. To dock to a screen edge and
  reserve the space:
  - Windows: AppBar (`SHAppBarMessage`).
  - Linux X11: `_NET_WM_STRUT`.
  - Linux Wayland: `wlr-layer-shell` (GNOME does not support it).
  - macOS: `NSWindow` level; no true space reservation.
- **Rendering:** wgpu plus glyphon or cosmic-text.
  - Render each segment to a cached texture.
  - Scroll by elapsed time, not frame count; vsync-locked.
  - Choose sub-pixel vs pixel-snapped movement deliberately (main source of jitter).
- **No web UI.** No HTML, CSS or webview anywhere; everything is native Rust.
  (Tauri with HTML/CSS skins was considered and rejected: heavier runtime,
  less control over frame timing.)
- **Runs without a GPU.** Rendering is CPU-first:
  - Each quote segment is drawn once into an RGBA image with `tiny-skia` +
    `cosmic-text` and the bundled fonts (never system fonts), so output is
    identical on every machine and checked against reference images.
  - Frames are composed on the CPU (a 4K × 56 px strip is under 1 MB) and
    presented with `softbuffer`. The demo uses about 0.3 % of one core.
  - A wgpu backend is an optional upgrade for vsync-locked presentation and
    effects; it falls back to software adapters (WARP, llvmpipe) and then to
    the CPU path.
- **Drawing the banner elements** (bars, tiers, pills, separators, gradients,
  shadows, arrows, sparklines, flash/fade animations): `tiny-skia` paths and
  fills into the segment images. Rotated side-edge text is a rotated image.
- **Fonts:** bundled Inter (tabular figures) and JetBrains Mono, both OFL.
- **Glass and overlays:** `window-vibrancy` for blur (Windows Acrylic/Mica,
  macOS vibrancy; Linux falls back to plain transparency); winit transparent
  windows and `set_cursor_hittest(false)` for click-through.
- **Settings UI: egui** (`eframe`). Pure Rust, lightweight, shares the wgpu
  and winit stack with the banners, restyled to match the mockups, and tested
  with `egui_kittest`. TOML with hot reload remains the source of truth; the
  settings window edits it. Alternatives considered: Slint (license needs
  review for MIT), iced (more code), wxDragon (needs a C++ dependency).
- **Skins:** declarative TOML with tokens, element styles and `extends`;
  a schema version so old skins keep loading. Hot reload with the `notify`
  crate. Layout (where and what shape) and skin (how it looks) are separate
  files, so any skin works on any layout.
- **Markets:** an `Exchange` registry (code, time zone, sessions, holidays,
  currency) used by the store, strategies and renderer; routes symbols to
  feeds by exchange.

## Strategies

`trait Strategy { fn plan(&mut self, store: &QuoteStore, now: Instant) -> Vec<Segment> }`,
composable:

- Round-robin through a watchlist, optionally grouped by sector with header segments.
- Movers-weighted: bigger % moves appear more often.
- Event-triggered: threshold cross, volume spike, 52-week high (with flash/highlight).
- Time-of-day schedules: pre-market futures, regular session, after-hours.
- Frequency caps: minimum gap, maximum repeats per loop.
- Pinned or flipper panel: fixed left box flipping between major indices while
  the main belt scrolls (the classic two-tier look).

## Market data

- `trait QuoteSource`; providers swappable.
- Candidates: Finnhub (WebSocket), Alpaca (WebSocket, IEX), Polygon.io,
  Twelve Data, Tiingo, Alpha Vantage. IEX Cloud shut down in 2024.
- Official APIs only. Yahoo Finance endpoints are unofficial and against its
  terms, and CNBC has no API: neither is supported (decided 2026-11-12).
  Publishers' headlines can come through their RSS feeds (#27).
- Massive (formerly Polygon.io, renamed late 2025) is the second provider;
  its free plan allows about 5 requests a minute, so it batches symbols.
- Real-time US exchange data carries display fees; 15-minute delayed data is
  cheaper or free. Users supply their own keys; the project never redistributes
  quote data.

## Testing

- **Pure core:** the quote store, strategies, belt planner and layout have no I/O
  and take time from an injected `Clock`, so tests are deterministic.
- **Fake time:** a `ManualClock` steps time exactly, so scroll positions, frequency
  caps and time-of-day schedules can be asserted to the pixel and millisecond.
- **Feeds:** a `MockSource` with scripted quotes and a `ReplaySource` that
  plays back recorded sessions; provider adapters are tested against recorded
  HTTP and WebSocket fixtures, never live APIs in CI.
- **Rendering:** headless wgpu renders to an offscreen texture and is compared
  with golden images (snapshot tests per skin), with a tolerance for GPU
  differences.
- **Layout:** table and property tests (`proptest`) over generated monitor
  arrangements (mixed DPI, negative coordinates, gaps, rotation).
- **Platform:** a `Platform` trait with a fake implementation; thin real
  backends are covered by smoke tests on each OS in CI.
- **CI:** GitHub Actions matrix (Windows, Ubuntu, macOS) running `cargo fmt`,
  `clippy -D warnings`, tests, and coverage.

## Naming and trademarks

- Name `stocktape` chosen 2026-11-12. Free on crates.io, PyPI and npm.
- GitHub: one dormant 0★ Swift repo `kapadakos/stocktape`.
- stocktape.com is registered; .dev / .app looked unregistered.
- No STOCKTAPE trademark found in USPTO mirror sites (direct USPTO search not
  yet done; do it at tmsearch.uspto.gov).
- Nearest similar product: iOS app "Stocks Tape: Market Browser" (Cinnamon Mobile).
- Fallback name with zero collisions: `quotetape`.
- Skins must not ship network logos or names (CNBC, Fox Business, Bloomberg).
  Describe them neutrally, e.g. "Classic Business Dark". Colors and general
  layout are fine.

## Next steps

1. Cargo workspace scaffold (core, sources, strategies, layout, renderer,
   platform, app) with the CI matrix and test harness from day one.
2. winit + wgpu scrolling bar with a mock quote source and `ManualClock` tests.
3. One or two strategies and a TOML skin; first golden-image tests.
4. Multi-banner and multi-monitor layout (top/bottom per display), then
   spanning and perimeter belts.
5. Docking per OS: Windows AppBar, X11 strut, Wayland layer-shell, macOS level.
6. Two real feed adapters plus failover, including at least one with
   non-US exchanges; exchange registry and currency handling.
7. Reserve the crate name on crates.io.
