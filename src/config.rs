//! The crosshair's settings, and where they live on disk.
//!
//! The config file is the whole IPC mechanism: `foghud crosshair size 14` writes
//! the file, the running overlay notices the change and redraws. That keeps the
//! CLI and the overlay decoupled, and works identically on every platform.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Cross,
    TCross,
    Circle,
    Dot,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Cross, Style::TCross, Style::Circle, Style::Dot];

    pub fn as_str(self) -> &'static str {
        match self {
            Style::Cross => "cross",
            Style::TCross => "tcross",
            Style::Circle => "circle",
            Style::Dot => "dot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s.to_ascii_lowercase())
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&v| v == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Values each hotkey steps through, in order.
pub const SIZE_CYCLE: [f32; 8] = [6.0, 8.0, 10.0, 12.0, 14.0, 18.0, 22.0, 28.0];
pub const OPACITY_CYCLE: [f32; 5] = [1.0, 0.85, 0.7, 0.5, 0.3];
pub const COLOR_CYCLE: [&str; 6] = [
    "#00ff00", "#00e5ff", "#ff2b2b", "#ffd400", "#ff00d0", "#ffffff",
];

/// Next entry in a cycle, wrapping. Falls back to the first entry when the
/// current value isn't one of the presets (e.g. a hand-edited config).
pub fn next_f32(cycle: &[f32], current: f32) -> f32 {
    match cycle
        .iter()
        .position(|v| (v - current).abs() < f32::EPSILON)
    {
        Some(i) => cycle[(i + 1) % cycle.len()],
        None => cycle[0],
    }
}

pub fn next_str<'a>(cycle: &[&'a str], current: &str) -> &'a str {
    match cycle.iter().position(|v| v.eq_ignore_ascii_case(current)) {
        Some(i) => cycle[(i + 1) % cycle.len()],
        None => cycle[0],
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub enabled: bool,
    pub style: Style,
    pub color: String,
    pub outline_color: String,
    /// Arm length in pixels.
    pub size: f32,
    pub thickness: f32,
    /// Empty space between the centre and where the arms start.
    pub gap: f32,
    /// Centre dot radius. 0 disables it.
    pub dot: f32,
    /// Dark border width around every shape. 0 disables it.
    pub outline: f32,
    pub opacity: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    /// `all`, `primary`, or an output/display name such as `DP-3`.
    pub monitor: String,
    pub hotkeys: bool,

    /// Text of the hint panel to show over the crosshair. Set by the CLI.
    pub notice: String,
    /// Bumped every time a new hint should be shown. The overlay watches this
    /// rather than the text, so repeating the same hint still re-displays it.
    pub notice_id: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            style: Style::Cross,
            color: "#00ff00".into(),
            outline_color: "#000000".into(),
            size: 10.0,
            thickness: 2.0,
            gap: 4.0,
            dot: 0.0,
            outline: 1.0,
            opacity: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            monitor: "primary".into(),
            hotkeys: true,
            notice: String::new(),
            notice_id: 0,
        }
    }
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let dir = dirs::config_dir().context("no config directory for this user")?;
        Ok(dir.join("foghud").join("config.json"))
    }

    /// Never fails on a broken or missing file — a bad config shouldn't take the
    /// overlay down mid-match. Unreadable files fall back to defaults.
    pub fn load() -> Self {
        let Ok(path) = Self::path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("foghud: config is not valid JSON ({e}), using defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)? + "\n";
        // Written in place rather than renamed over: a rename swaps the inode and
        // breaks any file watch the running overlay holds on it.
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
    }
}

/// A colour as straight (non-premultiplied) RGBA.
pub type Rgba = [u8; 4];

const NAMED: &[(&str, Rgba)] = &[
    ("black", [0, 0, 0, 255]),
    ("white", [255, 255, 255, 255]),
    ("red", [255, 0, 0, 255]),
    ("green", [0, 255, 0, 255]),
    ("lime", [0, 255, 0, 255]),
    ("blue", [0, 0, 255, 255]),
    ("cyan", [0, 229, 255, 255]),
    ("magenta", [255, 0, 208, 255]),
    ("pink", [255, 105, 180, 255]),
    ("yellow", [255, 212, 0, 255]),
    ("orange", [255, 140, 0, 255]),
    ("purple", [160, 32, 240, 255]),
];

/// Accepts `#rgb`, `#rrggbb`, `#aarrggbb`, and the names above.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let b = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
                Some([d(0)?, d(1)?, d(2)?, 255])
            }
            6 => Some([b(0)?, b(2)?, b(4)?, 255]),
            // Qt/Hyprland convention: alpha leads.
            8 => Some([b(2)?, b(4)?, b(6)?, b(0)?]),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    NAMED.iter().find(|(n, _)| *n == lower).map(|(_, c)| *c)
}

pub fn is_valid_color(s: &str) -> bool {
    parse_color(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms_parse() {
        assert_eq!(parse_color("#ff0000"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("#f00"), Some([255, 0, 0, 255]));
        // Alpha leads in the 8-digit form.
        assert_eq!(parse_color("#80ff0000"), Some([255, 0, 0, 128]));
        assert_eq!(parse_color("cyan"), Some([0, 229, 255, 255]));
        assert_eq!(parse_color("#gg0000"), None);
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn styles_cycle_and_wrap() {
        assert_eq!(Style::Cross.next(), Style::TCross);
        assert_eq!(Style::Dot.next(), Style::Cross);
        assert_eq!(Style::parse("TCROSS"), Some(Style::TCross));
        assert_eq!(Style::parse("nope"), None);
    }

    #[test]
    fn cycles_wrap_and_recover() {
        assert_eq!(next_f32(&SIZE_CYCLE, 28.0), 6.0);
        assert_eq!(next_f32(&SIZE_CYCLE, 10.0), 12.0);
        // A hand-edited value that isn't a preset restarts the cycle.
        assert_eq!(next_f32(&SIZE_CYCLE, 13.5), 6.0);
        assert_eq!(next_str(&COLOR_CYCLE, "#ffffff"), "#00ff00");
    }

    #[test]
    fn partial_config_keeps_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"size": 20}"#).unwrap();
        assert_eq!(cfg.size, 20.0);
        assert_eq!(cfg.style, Style::Cross);
        assert!(cfg.enabled);
    }
}
