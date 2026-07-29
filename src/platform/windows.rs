//! Windows backend, via a layered window.
//!
//! Windows hands over everything Wayland refuses. `WS_EX_LAYERED` plus
//! `UpdateLayeredWindow` gives a window with real per-pixel alpha,
//! `WS_EX_TRANSPARENT` makes every click fall through to the game,
//! `WS_EX_TOPMOST` keeps it above, and `RegisterHotKey` claims F1-F4 globally.
//! It's the same mechanism Discord and OBS overlays use.

use anyhow::{Context, Result};
use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    EnumDisplayMonitors, GetDC, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey, VK_F1, VK_F2, VK_F3, VK_F4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
    PostQuitMessage, RegisterClassW, SW_SHOWNOACTIVATE, SetTimer, ShowWindow, TranslateMessage,
    ULW_ALPHA, UpdateLayeredWindow, WM_DESTROY, WM_HOTKEY, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::config::{self, Config};
use crate::render;

const HOTKEY_STYLE: usize = 1;
const HOTKEY_SIZE: usize = 2;
const HOTKEY_OPACITY: usize = 3;
const HOTKEY_COLOR: usize = 4;
const CONFIG_POLL_TIMER: usize = 100;

struct Monitor {
    name: String,
    rect: RECT,
}

/// Collected by `EnumDisplayMonitors` through the LPARAM below.
unsafe extern "system" fn collect_monitor(
    handle: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let list = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: size_of::<MONITORINFOEXW>() as u32,
            ..Default::default()
        },
        ..Default::default()
    };
    let ok = unsafe {
        GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool()
    };
    if ok {
        let name = String::from_utf16_lossy(&info.szDevice)
            .trim_end_matches('\0')
            .to_string();
        list.push(Monitor {
            name,
            rect: info.monitorInfo.rcMonitor,
        });
    }
    BOOL(1)
}

fn monitors() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut list as *mut Vec<Monitor> as isize),
        );
    }
    list
}

/// Monitors the crosshair should appear on, per the `monitor` setting.
fn selected(cfg: &Config) -> Vec<Monitor> {
    let all = monitors();
    match cfg.monitor.as_str() {
        "all" => all,
        "primary" => all.into_iter().take(1).collect(),
        name => {
            let want = name.to_ascii_lowercase();
            all.into_iter()
                .filter(|m| {
                    let lower = m.name.to_ascii_lowercase();
                    // Accept both the raw device name and a friendlier suffix,
                    // so `DISPLAY2` matches `\\.\DISPLAY2`.
                    lower == want || lower.ends_with(&want)
                })
                .collect()
        }
    }
}

/// Paints one window with the current crosshair.
fn present(hwnd: HWND, rect: RECT, cfg: &Config) -> Result<()> {
    let width = (rect.right - rect.left).max(1);
    let height = (rect.bottom - rect.top).max(1);

    let pixmap = render::draw(cfg, width as u32, height as u32);
    let bgra = render::to_bgra(&pixmap);

    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height means top-down, matching our buffer order.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .context("creating the overlay bitmap")?;
        let previous = SelectObject(mem_dc, bitmap.into());

        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len());
        }

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            // Our buffer is premultiplied, which is exactly what this asks for.
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let src = POINT { x: 0, y: 0 };
        let dst = POINT {
            x: rect.left,
            y: rect.top,
        };

        let result = UpdateLayeredWindow(
            hwnd,
            Some(screen_dc),
            Some(&dst),
            Some(&size),
            Some(mem_dc),
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(mem_dc, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        result.context("presenting the overlay")?;
    }
    Ok(())
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            let mut cfg = Config::load();
            match wparam.0 {
                HOTKEY_STYLE => cfg.style = cfg.style.next(),
                HOTKEY_SIZE => cfg.size = config::next_f32(&config::SIZE_CYCLE, cfg.size),
                HOTKEY_OPACITY => {
                    cfg.opacity = config::next_f32(&config::OPACITY_CYCLE, cfg.opacity)
                }
                HOTKEY_COLOR => {
                    cfg.color = config::next_str(&config::COLOR_CYCLE, &cfg.color).to_string()
                }
                _ => return LRESULT(0),
            }
            // Saving is enough: the poll timer notices and repaints, so a hotkey
            // press and a CLI change take exactly the same path.
            let _ = cfg.save();
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn register_hotkeys(hwnd: HWND, cfg: &Config) {
    if !cfg.hotkeys {
        return;
    }
    let none = HOT_KEY_MODIFIERS(0);
    unsafe {
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_STYLE as i32, none, VK_F1.0 as u32);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_SIZE as i32, none, VK_F2.0 as u32);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_OPACITY as i32, none, VK_F3.0 as u32);
        let _ = RegisterHotKey(Some(hwnd), HOTKEY_COLOR as i32, none, VK_F4.0 as u32);
    }
}

fn unregister_hotkeys(hwnd: HWND) {
    unsafe {
        for id in [HOTKEY_STYLE, HOTKEY_SIZE, HOTKEY_OPACITY, HOTKEY_COLOR] {
            let _ = UnregisterHotKey(Some(hwnd), id as i32);
        }
    }
}

fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(Config::path().ok()?)
        .ok()?
        .modified()
        .ok()
}

pub fn run() -> Result<()> {
    crate::daemon::write_pid()?;
    let result = run_inner();
    crate::daemon::clear_pid();
    result
}

fn run_inner() -> Result<()> {
    let mut cfg = Config::load();
    let class_name = w!("foghud_overlay");

    unsafe {
        let instance = GetModuleHandleW(None).context("getting the module handle")?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        // A zero return usually just means the class is already registered.
        RegisterClassW(&class);

        let targets = selected(&cfg);
        if targets.is_empty() {
            anyhow::bail!("no display matched monitor = '{}'", cfg.monitor);
        }

        let mut overlays: Vec<(HWND, RECT)> = Vec::new();
        for monitor in &targets {
            let r = monitor.rect;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                PCWSTR(class_name.as_ptr()),
                w!("foghud"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .context("creating the overlay window")?;

            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            present(hwnd, r, &cfg)?;
            overlays.push((hwnd, r));
        }

        // Hotkeys belong to one window; the first is as good as any.
        let owner = overlays[0].0;
        register_hotkeys(owner, &cfg);
        let _ = SetTimer(Some(owner), CONFIG_POLL_TIMER, 150, None);

        let mut mtime = config_mtime();
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.message == WM_TIMER && message.wParam.0 == CONFIG_POLL_TIMER {
                let now = config_mtime();
                if now != mtime {
                    mtime = now;
                    let previous_hotkeys = cfg.hotkeys;
                    cfg = Config::load();
                    if cfg.hotkeys != previous_hotkeys {
                        if cfg.hotkeys {
                            register_hotkeys(owner, &cfg);
                        } else {
                            unregister_hotkeys(owner);
                        }
                    }
                    for (hwnd, rect) in &overlays {
                        let _ = present(*hwnd, *rect, &cfg);
                    }
                }
                continue;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        unregister_hotkeys(owner);
        for (hwnd, _) in overlays {
            let _ = DestroyWindow(hwnd);
        }
    }
    Ok(())
}
