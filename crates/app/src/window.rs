//! The demo window: one borderless, always-on-top banner along the bottom of
//! the primary display, drawn on the CPU and presented with `softbuffer`, so
//! it runs without a GPU.
//!
//! Docking (reserving screen space) and multiple banners come later (#7, #8).

use std::error::Error;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use stocktape_core::SystemClock;
use stocktape_render::BannerStyle;
use stocktape_skin::SkinLibrary;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::ticker::Ticker;

/// Scroll speed of the demo, in logical pixels per second.
const SPEED: f64 = 120.0;
/// Frame interval when the monitor doesn't report its refresh rate.
const DEFAULT_FRAME: Duration = Duration::from_micros(16_667);

struct Banner {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    ticker: Ticker,
    frame_interval: Duration,
    next_frame: Instant,
}

/// Watches a skins folder and reports when anything in it changes.
struct SkinWatch {
    dir: PathBuf,
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
            events,
            _watcher: watcher,
        })
    }

    /// Whether any change arrived since the last call.
    fn changed(&self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.events.try_recv() {
            changed |= event.is_ok_and(|e| !e.kind.is_access());
        }
        changed
    }
}

struct App {
    clock: SystemClock,
    style: BannerStyle,
    skin: String,
    watch: Option<SkinWatch>,
    banner: Option<Banner>,
    error: Option<Box<dyn Error>>,
}

impl App {
    /// Re-reads the skins folder and restyles the banner. On error the current
    /// look stays and the problem is printed, so a half-typed edit never
    /// closes the app.
    fn reload_skin(&mut self) {
        let Some(watch) = &self.watch else { return };
        if !watch.changed() {
            return;
        }
        let mut library = SkinLibrary::builtin();
        let result = library
            .load_dir(&watch.dir)
            .map_err(|e| e.to_string())
            .and_then(|_| library.resolve(&self.skin).map_err(|e| e.to_string()));
        match result {
            Ok(skin) => {
                self.style = BannerStyle::from(&skin);
                eprintln!("reloaded skin `{}`", skin.id);
                if let Some(banner) = &mut self.banner {
                    banner.ticker.set_style(self.style.clone());
                    let width = banner.window.inner_size().width;
                    let height = banner.ticker.height_px();
                    let _ = banner
                        .window
                        .request_inner_size(PhysicalSize::new(width, height));
                    if let (Ok(pos), Some(monitor)) = (
                        banner.window.outer_position(),
                        banner.window.current_monitor(),
                    ) {
                        let bottom = monitor.position().y
                            + i32::try_from(monitor.size().height.saturating_sub(height))
                                .unwrap_or(0);
                        banner
                            .window
                            .set_outer_position(PhysicalPosition::new(pos.x, bottom));
                    }
                }
            }
            Err(e) => eprintln!("skin not reloaded: {e}"),
        }
    }

    fn create_banner(&self, event_loop: &ActiveEventLoop) -> Result<Banner, Box<dyn Error>> {
        let monitor = event_loop
            .primary_monitor()
            .or_else(|| event_loop.available_monitors().next())
            .ok_or("no display found")?;
        #[allow(clippy::cast_possible_truncation)]
        let scale = monitor.scale_factor() as f32;
        let ticker = Ticker::demo(seed(), SPEED, scale, self.style.clone(), &self.clock);
        let height = ticker.height_px();
        let size = monitor.size();
        let position = monitor.position();
        let attributes = Window::default_attributes()
            .with_title("stocktape")
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(size.width, height))
            .with_position(PhysicalPosition::new(
                position.x,
                position.y + i32::try_from(size.height.saturating_sub(height)).unwrap_or(0),
            ));
        let window = Rc::new(event_loop.create_window(attributes)?);
        let context = softbuffer::Context::new(window.clone())?;
        let surface = softbuffer::Surface::new(&context, window.clone())?;
        let frame_interval = monitor
            .refresh_rate_millihertz()
            .filter(|mhz| *mhz >= 10_000)
            .map_or(DEFAULT_FRAME, |mhz| {
                Duration::from_secs_f64(1000.0 / f64::from(mhz))
            });
        Ok(Banner {
            window,
            surface,
            ticker,
            frame_interval,
            next_frame: Instant::now(),
        })
    }

    fn draw(&mut self) -> Result<(), Box<dyn Error>> {
        let Some(banner) = self.banner.as_mut() else {
            return Ok(());
        };
        let size = banner.window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        banner.surface.resize(width, height)?;
        let frame = banner.ticker.frame(&self.clock, width.get());
        let mut buffer = banner.surface.buffer_mut()?;
        let shown = (frame.width() * frame.height().min(height.get())) as usize;
        let shown = shown.min(buffer.len());
        let (top, rest) = buffer.split_at_mut(shown);
        for (out, px) in top.iter_mut().zip(frame.data().as_chunks::<4>().0) {
            *out = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
        rest.fill(0);
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
        if self.banner.is_none() {
            match self.create_banner(event_loop) {
                Ok(banner) => self.banner = Some(banner),
                Err(e) => self.fail(event_loop, e),
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
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
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(banner) = self.banner.as_mut() {
                    #[allow(clippy::cast_possible_truncation)]
                    banner.ticker.set_scale(scale_factor as f32);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.reload_skin();
        // Redraw only when a frame is due; requesting a redraw on every wake-up
        // would spin the event loop at full CPU.
        if let Some(banner) = &mut self.banner {
            let now = Instant::now();
            if now >= banner.next_frame {
                banner.window.request_redraw();
                let next = banner.next_frame + banner.frame_interval;
                banner.next_frame = if next > now {
                    next
                } else {
                    now + banner.frame_interval
                };
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(banner.next_frame));
        }
    }
}

/// A seed for the demo feed that differs between runs.
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Opens the demo banner and runs until it is closed (Esc or the window
/// manager's close).
///
/// `skin` defaults to the library's default; with `skins_dir`, edits to the
/// files there restyle the banner live.
pub fn run(
    library: SkinLibrary,
    skin: Option<String>,
    skins_dir: Option<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let skin = skin.unwrap_or_else(|| SkinLibrary::DEFAULT.to_owned());
    // Fail before opening a window if the skin is broken.
    let style = BannerStyle::from(&library.resolve(&skin)?);
    let watch = skins_dir.map(SkinWatch::new).transpose()?;
    let event_loop = EventLoop::new()?;
    let mut app = App {
        clock: SystemClock::new(),
        style,
        skin,
        watch,
        banner: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}
