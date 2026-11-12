//! Per-OS integration for stocktape: reserving screen space for banners.
//!
//! A docked banner tells the desktop to keep that strip of the screen free,
//! so maximized windows and the taskbar stay clear of it:
//!
//! | Platform | How | Notes |
//! |---|---|---|
//! | Windows | AppBar (`SHAppBarMessage`) | The shell may move the strip, e.g. above the taskbar; [`dock`] returns where it ended up. |
//! | Linux, X11 | `_NET_WM_STRUT_PARTIAL` and the dock window type | Honoured by most window managers. |
//! | Linux, Wayland | not yet | Needs `wlr-layer-shell`; the banner stays an overlay. |
//! | macOS | not possible | macOS has no space reservation; the banner stays an overlay. |
//!
//! This is the only crate in the workspace that uses `unsafe`, for the
//! Windows API.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

use std::fmt;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use stocktape_layout::{Edge, Rect};

/// Why a banner couldn't be docked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockError {
    /// This platform or session can't reserve screen space.
    Unsupported(&'static str),
    /// The platform refused or failed.
    Failed(String),
}

impl fmt::Display for DockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "docking is not supported here: {why}"),
            Self::Failed(why) => write!(f, "docking failed: {why}"),
        }
    }
}

impl std::error::Error for DockError {}

/// A docked banner window. Dropping it gives the space back.
#[derive(Debug)]
pub struct Docked {
    /// Where the platform placed the strip, in physical pixels. The window
    /// should be moved here.
    pub rect: Rect,
    #[cfg(windows)]
    _appbar: windows::AppBar,
}

/// Reserves `rect` along `edge` for `window`.
///
/// # Errors
///
/// Returns [`DockError::Unsupported`] where space can't be reserved (macOS,
/// Wayland) and [`DockError::Failed`] when the platform call fails.
pub fn dock(window: &impl HasWindowHandle, edge: Edge, rect: Rect) -> Result<Docked, DockError> {
    let handle = window
        .window_handle()
        .map_err(|e| DockError::Failed(e.to_string()))?
        .as_raw();
    dock_raw(handle, edge, rect)
}

#[cfg(windows)]
fn dock_raw(handle: RawWindowHandle, edge: Edge, rect: Rect) -> Result<Docked, DockError> {
    let RawWindowHandle::Win32(h) = handle else {
        return Err(DockError::Unsupported("not a Win32 window"));
    };
    let (appbar, rect) = windows::AppBar::new(h.hwnd.get(), edge, rect)?;
    Ok(Docked {
        rect,
        _appbar: appbar,
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn dock_raw(handle: RawWindowHandle, edge: Edge, rect: Rect) -> Result<Docked, DockError> {
    let window = match handle {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window)
            .map_err(|_| DockError::Failed("X11 window id out of range".to_owned()))?,
        RawWindowHandle::Xcb(h) => h.window.get(),
        RawWindowHandle::Wayland(_) => {
            return Err(DockError::Unsupported(
                "Wayland needs wlr-layer-shell, which isn't implemented yet",
            ));
        }
        _ => return Err(DockError::Unsupported("unknown window system")),
    };
    x11::set_strut(window, edge, rect)?;
    Ok(Docked { rect })
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn dock_raw(_: RawWindowHandle, _: Edge, _: Rect) -> Result<Docked, DockError> {
    Err(DockError::Unsupported(
        "this platform has no way to reserve screen space",
    ))
}

/// The 12 values of `_NET_WM_STRUT_PARTIAL` for a strip `rect` along `edge`
/// of a root window `root_width × root_height` pixels.
///
/// Struts are measured from the edges of the whole virtual screen (the root
/// window), not of the monitor the banner is on.
#[must_use]
pub fn strut_partial(edge: Edge, rect: Rect, root_width: u32, root_height: u32) -> [u32; 12] {
    let clamp = |v: i64| u32::try_from(v.max(0)).unwrap_or(u32::MAX);
    let (x0, x1) = (clamp(i64::from(rect.x)), clamp(rect.right() - 1));
    let (y0, y1) = (clamp(i64::from(rect.y)), clamp(rect.bottom() - 1));
    let mut s = [0u32; 12];
    match edge {
        Edge::Left => {
            s[0] = clamp(rect.right());
            s[4] = y0;
            s[5] = y1;
        }
        Edge::Right => {
            s[1] = clamp(i64::from(root_width) - i64::from(rect.x));
            s[6] = y0;
            s[7] = y1;
        }
        Edge::Top => {
            s[2] = clamp(rect.bottom());
            s[8] = x0;
            s[9] = x1;
        }
        Edge::Bottom => {
            s[3] = clamp(i64::from(root_height) - i64::from(rect.y));
            s[10] = x0;
            s[11] = x1;
        }
    }
    s
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    //! Windows AppBars.

    use std::ffi::c_void;

    use stocktape_layout::{Edge, Rect};
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Shell::{
        ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS,
        APPBARDATA, SHAppBarMessage,
    };

    use crate::DockError;

    /// A registered AppBar; unregistered on drop.
    #[derive(Debug)]
    pub(crate) struct AppBar {
        hwnd: isize,
    }

    fn data(hwnd: isize, edge: u32, rc: RECT) -> APPBARDATA {
        APPBARDATA {
            cbSize: u32::try_from(std::mem::size_of::<APPBARDATA>()).unwrap_or(0),
            hWnd: hwnd as *mut c_void,
            uEdge: edge,
            rc,
            ..APPBARDATA::default()
        }
    }

    impl AppBar {
        pub(crate) fn new(hwnd: isize, edge: Edge, rect: Rect) -> Result<(Self, Rect), DockError> {
            let ab_edge = match edge {
                Edge::Top => ABE_TOP,
                Edge::Bottom => ABE_BOTTOM,
                Edge::Left => ABE_LEFT,
                Edge::Right => ABE_RIGHT,
            };
            let to_i32 = |v: i64| {
                i32::try_from(v).map_err(|_| DockError::Failed("rectangle out of range".to_owned()))
            };
            let rc = RECT {
                left: rect.x,
                top: rect.y,
                right: to_i32(rect.right())?,
                bottom: to_i32(rect.bottom())?,
            };
            let mut abd = data(hwnd, ab_edge, rc);
            // SAFETY: `abd` is a fully initialised APPBARDATA with its size set,
            // living for the duration of each call, and `hwnd` is a live window
            // owned by this process.
            if unsafe { SHAppBarMessage(ABM_NEW, &raw mut abd) } == 0 {
                return Err(DockError::Failed(
                    "the shell refused to register the AppBar".to_owned(),
                ));
            }
            let bar = Self { hwnd };
            // The shell moves the proposed strip clear of the taskbar and
            // other AppBars on that edge; keep our thickness on the far side.
            // SAFETY: as above.
            unsafe { SHAppBarMessage(ABM_QUERYPOS, &raw mut abd) };
            let thick = to_i32(match edge {
                Edge::Top | Edge::Bottom => i64::from(rect.height),
                Edge::Left | Edge::Right => i64::from(rect.width),
            })?;
            match edge {
                Edge::Top => abd.rc.bottom = abd.rc.top + thick,
                Edge::Bottom => abd.rc.top = abd.rc.bottom - thick,
                Edge::Left => abd.rc.right = abd.rc.left + thick,
                Edge::Right => abd.rc.left = abd.rc.right - thick,
            }
            // SAFETY: as above.
            unsafe { SHAppBarMessage(ABM_SETPOS, &raw mut abd) };
            let rc = abd.rc;
            let width = u32::try_from(rc.right - rc.left).unwrap_or(rect.width);
            let height = u32::try_from(rc.bottom - rc.top).unwrap_or(rect.height);
            Ok((bar, Rect::new(rc.left, rc.top, width, height)))
        }
    }

    /// See [`crate::release_own_console`].
    pub(crate) fn release_own_console() {
        use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
        let mut ids = [0u32; 4];
        // SAFETY: the buffer is valid for its full length, which is passed.
        let attached = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), 4) };
        // Only this process on the console: Windows created it for us.
        if attached == 1 {
            // SAFETY: detaching from our own console has no preconditions.
            unsafe { FreeConsole() };
        }
    }

    impl Drop for AppBar {
        fn drop(&mut self) {
            let mut abd = data(
                self.hwnd,
                0,
                RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
            );
            // SAFETY: `abd` is initialised with its size set; removing an AppBar
            // for a window that is already gone is harmless.
            unsafe { SHAppBarMessage(ABM_REMOVE, &raw mut abd) };
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod x11 {
    //! X11 struts.

    use stocktape_layout::{Edge, Rect};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    use crate::DockError;

    fn fail(e: impl std::fmt::Display) -> DockError {
        DockError::Failed(e.to_string())
    }

    pub(crate) fn set_strut(window: u32, edge: Edge, rect: Rect) -> Result<(), DockError> {
        let (conn, screen) = x11rb::connect(None).map_err(fail)?;
        let root = &conn.setup().roots[screen];
        let (w, h) = (
            u32::from(root.width_in_pixels),
            u32::from(root.height_in_pixels),
        );
        let strut = crate::strut_partial(edge, rect, w, h);
        let atom = |name: &[u8]| -> Result<u32, DockError> {
            Ok(conn
                .intern_atom(false, name)
                .map_err(fail)?
                .reply()
                .map_err(fail)?
                .atom)
        };
        let partial = atom(b"_NET_WM_STRUT_PARTIAL")?;
        let plain = atom(b"_NET_WM_STRUT")?;
        let window_type = atom(b"_NET_WM_WINDOW_TYPE")?;
        let dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            partial,
            AtomEnum::CARDINAL,
            &strut,
        )
        .map_err(fail)?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            plain,
            AtomEnum::CARDINAL,
            &strut[..4],
        )
        .map_err(fail)?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            window_type,
            AtomEnum::ATOM,
            &[dock],
        )
        .map_err(fail)?;
        conn.flush().map_err(fail)?;
        Ok(())
    }
}

/// On Windows, closes the console window that was opened just for
/// stocktape, i.e. when it was started by double-click, from the Start menu
/// or at login rather than from a terminal. Started from a terminal, the
/// terminal is shared and kept, so command-line output works as usual. Does
/// nothing on other platforms.
pub fn release_own_console() {
    #[cfg(windows)]
    windows::release_own_console();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struts_are_measured_from_the_root_edges() {
        // Root 3840×1080: two 1920×1080 monitors side by side.
        let bottom = strut_partial(Edge::Bottom, Rect::new(1920, 1024, 1920, 56), 3840, 1080);
        assert_eq!(bottom, [0, 0, 0, 56, 0, 0, 0, 0, 0, 0, 1920, 3839]);
        let top = strut_partial(Edge::Top, Rect::new(0, 0, 1920, 44), 3840, 1080);
        assert_eq!(top, [0, 0, 44, 0, 0, 0, 0, 0, 0, 1919, 0, 0]);
        let right = strut_partial(Edge::Right, Rect::new(3780, 0, 60, 1080), 3840, 1080);
        assert_eq!(right, [0, 60, 0, 0, 0, 0, 0, 1079, 0, 0, 0, 0]);
        let left = strut_partial(Edge::Left, Rect::new(0, 100, 50, 800), 3840, 1080);
        assert_eq!(left, [50, 0, 0, 0, 100, 899, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn a_shorter_monitor_reserves_from_the_root_bottom() {
        // Left monitor 1920×1080 next to a taller 1050×1680 one starting at
        // y = -251 would make a negative root origin; X11 roots start at 0,
        // so shift: the root is 2970×1680 and the 1080-high monitor ends at
        // 1080 + 251.
        let s = strut_partial(Edge::Bottom, Rect::new(0, 251 + 1024, 1920, 56), 2970, 1680);
        assert_eq!(s[3], 1680 - 1275);
    }
}
