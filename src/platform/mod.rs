//! Platform backends.
//!
//! Everything above this module is portable: the config, the CLI and the
//! rasteriser. A backend only has to do two things — put a click-through,
//! always-on-top surface on screen and present a BGRA buffer into it, and
//! register the hotkeys.
//!
//! Those two jobs are exactly where the platforms diverge hardest. Windows
//! hands them over directly (`WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST`
//! plus `RegisterHotKey`), while Wayland forbids both for ordinary clients and
//! routes them through `wlr-layer-shell` and the compositor's own keybinds.

use anyhow::Result;

#[cfg(target_os = "linux")]
mod wayland;

#[cfg(windows)]
mod windows;

/// Best-effort request that the control panel be floated at this size rather
/// than tiled into whatever slot the layout has spare.
///
/// A no-op anywhere it doesn't apply — Windows floats ordinary windows already.
pub fn float_control_panel(width: u32, height: u32) {
    #[cfg(target_os = "linux")]
    {
        wayland::float_window(width, height);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (width, height);
    }
}

/// Display names of the connected outputs, for the control panel's monitor
/// dropdown.
///
/// Empty when they can't be determined, which the panel treats as "offer only
/// `all` and `primary`" and leaves its free-text field to do the rest. Nothing
/// depends on this being complete or correct — it's a convenience, and the
/// authoritative matching happens in `Widget::wants_screen`.
pub fn monitor_names() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        wayland::monitor_names()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

/// Runs the overlay until it's asked to stop. Blocks.
pub fn run_overlay() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        wayland::run()
    }
    #[cfg(windows)]
    {
        windows::run()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        anyhow::bail!("no overlay backend for this platform yet")
    }
}
