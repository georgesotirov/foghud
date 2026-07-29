//! Starting, stopping and locating the background overlay process.
//!
//! The overlay runs detached so it survives the terminal that launched it, and
//! a pidfile in the cache directory is how later commands find it again.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub fn pid_path() -> Result<PathBuf> {
    let dir = dirs::cache_dir().context("no cache directory for this user")?;
    Ok(dir.join("foghud").join("overlay.pid"))
}

fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        // Confirm it's actually ours: a recycled pid belonging to some other
        // program must not be mistaken for a running overlay, or `stop` would
        // kill it.
        match std::fs::read_to_string(format!("/proc/{pid}/cmdline")) {
            Ok(cmdline) => cmdline
                .split('\0')
                .next()
                .is_some_and(|p| p.contains("foghud")),
            Err(_) => false,
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                Ok(h) => {
                    let _ = CloseHandle(h);
                    true
                }
                Err(_) => false,
            }
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = pid;
        false
    }
}

pub fn running_pid() -> Option<u32> {
    let path = pid_path().ok()?;
    let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
    if alive(pid) {
        Some(pid)
    } else {
        // Stale pidfile from a crash or a reboot.
        let _ = std::fs::remove_file(&path);
        None
    }
}

pub fn is_running() -> bool {
    running_pid().is_some()
}

pub fn write_pid() -> Result<()> {
    let path = pid_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, std::process::id().to_string())?;
    Ok(())
}

pub fn clear_pid() {
    if let Ok(path) = pid_path() {
        let _ = std::fs::remove_file(path);
    }
}

/// Re-executes ourselves as `foghud run`, detached from this terminal.
pub fn spawn_detached() -> Result<()> {
    let exe = std::env::current_exe().context("locating the foghud executable")?;
    let mut cmd = Command::new(exe);
    cmd.arg("run")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Own process group, so closing the terminal doesn't take it down.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    cmd.spawn().context("starting the overlay process")?;
    Ok(())
}

/// Asks the overlay to exit. Returns false if it wasn't running.
pub fn stop() -> Result<bool> {
    let Some(pid) = running_pid() else {
        return Ok(false);
    };

    #[cfg(unix)]
    unsafe {
        // SIGTERM so the backend can unbind its hotkeys before exiting.
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
        if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(h, 0);
            let _ = CloseHandle(h);
        }
    }

    // Give it a moment to go away so `stop` can report honestly.
    for _ in 0..40 {
        if !is_running() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    clear_pid();
    Ok(true)
}
