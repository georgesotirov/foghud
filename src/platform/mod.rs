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
