//! Banner windows: one borderless, always-on-top window per banner segment,
//! drawn on the CPU and presented with `softbuffer`, so no GPU is needed.
//!
//! Displays are re-scanned every couple of seconds; when they change (a
//! monitor plugged in, unplugged, rearranged or rescaled) every banner is laid
//! out again. Docking (reserving screen space) comes later (#8).

use std::error::Error;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use stocktape_core::SystemClock;
use stocktape_layout::{BannerLayout, Display, Edge, Placement, Rect, Segment, plan};
use stocktape_render::BannerStyle;
use stocktape_render::frame::present_strip;
use stocktape_skin::SkinLibrary;
use tiny_skia::Pixmap;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::ticker::{BannerRuntime, LineSpec, Market, demo_groups};
use stocktape_config::{BannerConfig, Config};

/// Frame interval when no display reports its refresh rate.
const DEFAULT_FRAME: Duration = Duration::from_micros(16_667);
/// How often displays are re-scanned for changes.
const DISPLAY_SCAN: Duration = Duration::from_secs(2);

/// Watches a skins folder and reports when anything in it changes.
struct SkinWatch {
    dir: PathBuf,
    /// Only changes to this file count (for the configuration file).
    file: Option<std::ffi::OsString>,
    events: Receiver<notify::Result<notify::Event>>,
    // Dropping the watcher stops the notifications.
    _watcher: RecommendedWatcher,
}

impl SkinWatch {
    fn new(dir: PathBuf) -> Result<Self, Box<dyn Error>> {
        let (tx, events) = channel();
        let mut watcher = notify::recommended_watcher(tx)?;
        watcher.watch(&dir, RecursiveMode::NonRecursive)?;
        Ok(Self {
            dir,
            file: None,
            events,
            _watcher: watcher,
        })
    }

    /// Watches one file (through its folder, since editors often replace
    /// files rather than write them in place).
    fn file(path: &std::path::Path) -> Result<Self, Box<dyn Error>> {
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        // The folder may not exist before the first save; create it so it
        // can be watched.
        std::fs::create_dir_all(dir)?;
        let mut watch = Self::new(dir.to_path_buf())?;
        watch.file = path.file_name().map(std::ffi::OsStr::to_os_string);
        Ok(watch)
    }

    /// Whether any change arrived since the last call.
    fn changed(&self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            changed |= event.is_ok_and(|e| {
                !e.kind.is_access()
                    && self.file.as_ref().is_none_or(|name| {
                        e.paths
                            .iter()
                            .any(|p| p.file_name() == Some(name.as_os_str()))
                    })
            });
        }
        changed
    }
}

/// One window showing one segment of one banner.
struct SegmentWindow {
    // Declared first so the reserved space is released before the window
    // goes away.
    _docked: Option<stocktape_platform::Docked>,
    banner: usize,
    segment: Segment,
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    strip: Option<Pixmap>,
    /// How often this docked window was moved back into place.
    held: u32,
}

/// How often a docked window is moved back after the system moves it, before
/// giving up (so stocktape never fights the window manager in a loop).
const MAX_HOLDS: u32 = 8;

struct Banner {
    config: BannerConfig,
    runtime: BannerRuntime,
    layout: Option<BannerLayout>,
}

struct App {
    clock: SystemClock,
    market: Market,
    banners: Vec<Banner>,
    library: SkinLibrary,
    watch: Option<SkinWatch>,
    config_watch: Option<ConfigWatch>,
    seed: u64,
    windows: Vec<SegmentWindow>,
    displays: Vec<Display>,
    last_scan: Option<Instant>,
    frame_interval: Duration,
    next_frame: Instant,
    error: Option<Box<dyn Error>>,
}

/// The current displays as layout data.
fn scan(event_loop: &ActiveEventLoop) -> Vec<Display> {
    let primary = event_loop.primary_monitor();
    event_loop
        .available_monitors()
        .enumerate()
        .map(|(i, m)| {
            let (pos, size) = (m.position(), m.size());
            Display {
                name: m.name().unwrap_or_else(|| format!("display {i}")),
                rect: Rect::new(pos.x, pos.y, size.width, size.height),
                scale: m.scale_factor(),
                primary: primary.as_ref() == Some(&m),
            }
        })
        .collect()
}

fn refresh_interval(event_loop: &ActiveEventLoop) -> Duration {
    // Pace to the fastest display so no banner looks choppy.
    event_loop
        .available_monitors()
        .filter_map(|m| m.refresh_rate_millihertz())
        .filter(|mhz| *mhz >= 10_000)
        .max()
        .map_or(DEFAULT_FRAME, |mhz| {
            Duration::from_secs_f64(1000.0 / f64::from(mhz))
        })
}

fn create_window(
    event_loop: &ActiveEventLoop,
    banner: usize,
    name: &str,
    segment: &Segment,
    dock: Option<Edge>,
) -> Result<SegmentWindow, Box<dyn Error>> {
    let mut segment = segment.clone();
    let r = segment.rect;
    #[allow(unused_mut)]
    let mut attributes = Window::default_attributes()
        .with_title(format!("stocktape: {name}"))
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(WindowLevel::AlwaysOnTop)
        .with_inner_size(PhysicalSize::new(r.width, r.height))
        .with_position(PhysicalPosition::new(r.x, r.y));
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        // One taskbar button per segment would be noise.
        attributes = attributes.with_skip_taskbar(true);
    }
    let window = Rc::new(event_loop.create_window(attributes)?);
    let debug = std::env::var_os("STOCKTAPE_DEBUG").is_some();
    if debug {
        eprintln!(
            "[debug] `{name}` window requested {r:?}, created at {:?} size {:?}",
            window.outer_position().ok(),
            window.outer_size()
        );
    }
    let docked = dock.and_then(|edge| match stocktape_platform::dock(&*window, edge, r) {
        Ok(docked) => {
            if debug {
                eprintln!(
                    "[debug] `{name}` docked {edge:?}: granted {:?}",
                    docked.rect
                );
            }
            if docked.rect != r {
                // The desktop moved the strip, e.g. above the taskbar.
                let d = docked.rect;
                window.set_outer_position(PhysicalPosition::new(d.x, d.y));
                let _ = window.request_inner_size(PhysicalSize::new(d.width, d.height));
                segment.rect = d;
            }
            if debug {
                eprintln!(
                    "[debug] `{name}` after docking at {:?} size {:?}",
                    window.outer_position().ok(),
                    window.outer_size()
                );
            }
            Some(docked)
        }
        Err(e) => {
            eprintln!("banner `{name}` floats on top instead: {e}");
            None
        }
    });
    let context = softbuffer::Context::new(window.clone())?;
    let surface = softbuffer::Surface::new(&context, window.clone())?;
    Ok(SegmentWindow {
        _docked: docked,
        banner,
        segment,
        window,
        surface,
        strip: None,
        held: 0,
    })
}

impl App {
    /// Lays out every banner on the current displays and (re)creates windows.
    fn relayout(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        self.windows.clear();
        for (index, banner) in self.banners.iter_mut().enumerate() {
            let options = stocktape_layout::PlacementOptions {
                thickness: banner.runtime.thickness(),
                ..banner.config.options.clone()
            };
            match plan(&self.displays, &banner.config.placement, &options) {
                Ok(layout) => {
                    let scale = layout.segments.iter().map(|s| s.scale).fold(1.0, f64::max);
                    banner.runtime.set_measure_scale(scale);
                    let dock = match &banner.config.placement {
                        Placement::Edge { edge, .. } if banner.config.dock => Some(*edge),
                        _ => None,
                    };
                    for segment in &layout.segments {
                        self.windows.push(create_window(
                            event_loop,
                            index,
                            &banner.config.name,
                            segment,
                            dock,
                        )?);
                    }
                    banner.layout = Some(layout);
                }
                Err(e) => {
                    eprintln!("banner `{}` not shown: {e}", banner.config.name);
                    banner.layout = None;
                }
            }
        }
        if self.windows.is_empty() {
            return Err("no banner fits on the current displays".into());
        }
        self.frame_interval = refresh_interval(event_loop);
        Ok(())
    }

    /// Re-scans displays and lays out again if anything changed.
    fn check_displays(
        &mut self,
        event_loop: &ActiveEventLoop,
        force: bool,
    ) -> Result<(), Box<dyn Error>> {
        let now = Instant::now();
        if !force && self.last_scan.is_some_and(|t| now - t < DISPLAY_SCAN) {
            return Ok(());
        }
        self.last_scan = Some(now);
        let displays = scan(event_loop);
        if force || displays != self.displays {
            self.displays = displays;
            self.relayout(event_loop)?;
        }
        Ok(())
    }

    /// Re-reads the skins folder and restyles every banner. On error the
    /// current look stays and the problem is printed.
    fn reload_skins(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let Some(watch) = &self.watch else {
            return Ok(());
        };
        if !watch.changed() {
            return Ok(());
        }
        let mut library = SkinLibrary::builtin();
        if let Err(e) = library.load_dir(&watch.dir) {
            eprintln!("skins not reloaded: {e}");
            return Ok(());
        }
        let mut styles = Vec::new();
        for banner in &self.banners {
            match line_styles(&library, &banner.config) {
                Ok(s) => styles.push(s),
                Err(e) => {
                    eprintln!("skins not reloaded: {e}");
                    return Ok(());
                }
            }
        }
        for (banner, s) in self.banners.iter_mut().zip(styles) {
            banner.runtime.set_styles(s);
        }
        self.library = library;
        eprintln!("reloaded skins");
        // Thickness may have changed.
        self.check_displays(event_loop, true)
    }

    /// Re-reads the configuration file and rebuilds banners and feeds. On
    /// error the current setup stays and the problem is printed.
    fn reload_config(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let Some(cw) = &self.config_watch else {
            return Ok(());
        };
        if !cw.watch.changed() {
            return Ok(());
        }
        let loaded = std::fs::read_to_string(&cw.path)
            .map_err(|e| e.to_string())
            .and_then(|text| Config::parse(&text))
            .map(|mut config| {
                if let Some(skin) = &cw.skin_override {
                    for line in config.banners.iter_mut().flat_map(|b| &mut b.lines) {
                        line.skin.clone_from(skin);
                    }
                }
                config
            })
            .and_then(|config| config.check_keys().map(|()| config))
            .and_then(|config| {
                build_banners(&config, &self.library, self.seed, &self.clock)
                    .map(|banners| (config, banners))
                    .map_err(|e| e.to_string())
            });
        match loaded {
            Ok((config, banners)) => {
                self.market = Market::from_config(&config.feeds, &feed_groups(&config), self.seed)
                    .with_news(&config.news)
                    .with_labels(&config.labels);
                self.banners = banners;
                eprintln!("reloaded {}", cw.path.display());
                self.check_displays(event_loop, true)
            }
            Err(e) => {
                eprintln!("configuration not reloaded: {e}");
                Ok(())
            }
        }
    }

    fn draw(&mut self, id: WindowId) -> Result<(), Box<dyn Error>> {
        let Some(w) = self.windows.iter_mut().find(|w| w.window.id() == id) else {
            return Ok(());
        };
        let size = w.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        w.surface.resize(width, height)?;
        self.banners[w.banner]
            .runtime
            .render_segment(&w.segment, &mut w.strip);
        let mut buffer = w.surface.buffer_mut()?;
        if let Some(strip) = &w.strip {
            present_strip(
                strip,
                w.segment.flow,
                &mut buffer,
                width.get(),
                height.get(),
            );
        }
        buffer.present()?;
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn Error>) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.is_empty()
            && let Err(e) = self.check_displays(event_loop, true)
        {
            self.fail(event_loop, e);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested
            | WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => event_loop.exit(),
            WindowEvent::Moved(position) => {
                // Windows may move a docked banner out of the strip its own
                // AppBar reserved (seen with top edges right after start).
                // Put it back, a bounded number of times.
                if let Some(w) = self.windows.iter_mut().find(|w| w.window.id() == id) {
                    let r = w.segment.rect;
                    let moved = position.x != r.x || position.y != r.y;
                    if std::env::var_os("STOCKTAPE_DEBUG").is_some() {
                        eprintln!(
                            "[debug] window for banner {} moved to {position:?} (segment at {r:?})",
                            w.banner
                        );
                    }
                    if moved && w._docked.is_some() && w.held < MAX_HOLDS {
                        w.held += 1;
                        w.window.set_outer_position(PhysicalPosition::new(r.x, r.y));
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                // A display's scale changed: lay out again on the next scan.
                self.last_scan = None;
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw(id) {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(e) = self
            .reload_skins(event_loop)
            .and_then(|()| self.reload_config(event_loop))
            .and_then(|()| self.check_displays(event_loop, false))
        {
            self.fail(event_loop, e);
            return;
        }
        // Advance once per frame, then let every window draw its stretch.
        // Requesting redraws on every wake-up would spin the CPU.
        let now = Instant::now();
        if now >= self.next_frame {
            self.market.poll(&self.clock);
            for banner in &mut self.banners {
                if let Some(layout) = &banner.layout {
                    banner
                        .runtime
                        .update(&self.market, &self.clock, layout.belt_len());
                }
            }
            for w in &self.windows {
                w.window.request_redraw();
            }
            let next = self.next_frame + self.frame_interval;
            self.next_frame = if next > now {
                next
            } else {
                now + self.frame_interval
            };
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

/// A seed for the demo feed that differs between runs.
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Opens the configured banners and runs until closed (Esc on any banner, or
/// the window manager's close). With `skins_dir`, skin edits apply live.
pub fn run(
    config: Config,
    library: SkinLibrary,
    skins_dir: Option<PathBuf>,
    reload: Option<(PathBuf, Option<String>)>,
) -> Result<(), Box<dyn Error>> {
    let clock = SystemClock::new();
    let seed = seed();
    // Fail before opening any window if a skin is broken.
    let banners = build_banners(&config, &library, seed, &clock)?;
    let watch = skins_dir.map(SkinWatch::new).transpose()?;
    let config_watch = reload
        .map(|(path, skin_override)| {
            SkinWatch::file(&path).map(|watch| ConfigWatch {
                path,
                skin_override,
                watch,
            })
        })
        .transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        clock,
        market: Market::from_config(&config.feeds, &feed_groups(&config), seed)
            .with_news(&config.news)
            .with_labels(&config.labels),
        banners,
        library,
        watch,
        config_watch,
        seed,
        windows: Vec::new(),
        displays: Vec::new(),
        last_scan: None,
        frame_interval: DEFAULT_FRAME,
        next_frame: Instant::now(),
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

/// Watches the configuration file for live reload.
struct ConfigWatch {
    path: PathBuf,
    /// `--skin`, re-applied after each reload.
    skin_override: Option<String>,
    watch: SkinWatch,
}

/// Builds every configured banner; fails if a skin can't be resolved.
fn build_banners(
    config: &Config,
    library: &SkinLibrary,
    seed: u64,
    clock: &dyn stocktape_core::Clock,
) -> Result<Vec<Banner>, stocktape_skin::SkinError> {
    let groups = config.watchlist.clone().unwrap_or_else(demo_groups);
    config
        .banners
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, config)| {
            let specs: Vec<LineSpec> = config
                .lines
                .iter()
                .zip(line_styles(library, &config)?)
                .map(|(line, style)| LineSpec {
                    strategy: line.strategy,
                    content: line.content,
                    messages: line.messages.clone(),
                    groups: line.groups.clone(),
                    symbols: line.symbols.clone(),
                    speed: line.speed,
                    style,
                })
                .collect();
            let runtime = BannerRuntime::new(&specs, &groups, seed + i as u64, clock);
            Ok(Banner {
                config,
                runtime,
                layout: None,
            })
        })
        .collect()
}

/// Prints the displays as stocktape sees them, for choosing `displays` in the
/// configuration.
pub fn list_displays() -> Result<(), Box<dyn Error>> {
    struct Lister(Option<Vec<Display>>);
    impl ApplicationHandler for Lister {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.0 = Some(scan(event_loop));
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = EventLoop::new()?;
    let mut lister = Lister(None);
    event_loop.run_app(&mut lister)?;
    let displays = lister.0.unwrap_or_default();
    if displays.is_empty() {
        println!("no displays found");
    }
    println!(
        "{:<6} {:<24} {:>12} {:>11} {:>6}  orientation",
        "index", "name", "position", "size", "scale"
    );
    for (i, d) in displays.iter().enumerate() {
        println!(
            "{:<6} {:<24} {:>12} {:>11} {:>6}  {}{}",
            i,
            d.name,
            format!("{},{}", d.rect.x, d.rect.y),
            format!("{}x{}", d.rect.width, d.rect.height),
            format!("{:.2}", d.scale),
            if d.is_landscape() {
                "landscape"
            } else {
                "portrait"
            },
            if d.primary { ", primary" } else { "" },
        );
    }
    Ok(())
}

/// Resolves the skin of each of a banner's lines, top first.
fn line_styles(
    library: &SkinLibrary,
    config: &BannerConfig,
) -> Result<Vec<BannerStyle>, stocktape_skin::SkinError> {
    config
        .lines
        .iter()
        .map(|line| {
            library
                .resolve(&line.skin)
                .map(|skin| BannerStyle::from(&skin))
        })
        .collect()
}

/// Everything the feeds must follow: the watchlist plus any symbols lines
/// list on their own.
fn feed_groups(config: &Config) -> Vec<stocktape_strategies::Group> {
    let mut groups = config.watchlist.clone().unwrap_or_else(demo_groups);
    let extra: Vec<stocktape_core::Symbol> = config
        .banners
        .iter()
        .flat_map(|b| &b.lines)
        .flat_map(|l| l.symbols.iter().cloned())
        .collect();
    if !extra.is_empty() {
        groups.push(stocktape_strategies::Group {
            title: None,
            symbols: extra,
        });
    }
    groups
}
