# stocktape

A broadcast-style scrolling stock ticker for the desktop, written in Rust.

stocktape puts a smooth, TV-quality quote crawl on the edges of your screens:
two-tier bars with a pinned index box, green ▲ and red ▼ moves, and rules for
which quotes appear and when.

> **Status:** early development. Nothing to install yet. See
> [docs/design-notes.md](docs/design-notes.md) for the design.

## Planned features

- **Smooth crawl:** time-based, vsync-locked scrolling rendered with wgpu.
- **Multiple monitors and banners:** top and/or bottom of any display, side
  edges, one belt spanning several displays, or a belt running around the
  edge of a display or the whole desktop.
- **Customizable layouts:** placement, size, tiers, pinned boxes, speed,
  direction, docked or click-through overlay, presets.
- **Skins:** declarative TOML files with design tokens and inheritance, hot
  reloaded. Built-in styles: Classic Dark, Daylight, Terminal and Glass.
- **Strategies:** round-robin watchlists, movers weighting, event highlights,
  time-of-day schedules and frequency caps, composable.
- **Multiple feeds:** several market-data providers at once, with failover,
  including non-US exchanges with local trading hours and currencies.
- **Cross-platform:** Windows, Linux (X11 and Wayland) and macOS.
- **Native:** pure Rust, no web UI. The settings window uses egui.
- **Testable:** deterministic clock, mock and replay feeds, headless
  golden-image rendering tests.

## Building

Requires a stable Rust toolchain (see `rust-version` in `Cargo.toml`).

```sh
cargo build --workspace
cargo test --workspace          # or: cargo nextest run --workspace
cargo clippy --workspace --all-targets
cargo run -- --help
cargo run --release             # demo banner with sample (not real) prices; Esc quits
```

No GPU is required: banners are drawn and composed on the CPU.

The workspace is split into small crates under `crates/`: `core`, `sources`,
`strategies`, `layout`, `skin`, `render`, `platform`, `ui` and `app` (the
`stocktape` binary).

## Market data

stocktape does not ship or redistribute market data. You bring your own API
key for a supported provider. Check each provider's terms, especially for
real-time exchange data.

## Trademarks

stocktape is not affiliated with any broadcaster or exchange. Skins are
inspired by the general look of business TV tickers and do not use any
network's names or logos.

## License

[MIT](LICENSE)
