//! The settings, and where they live on disk.
//!
//! The config file is the whole IPC mechanism: `foghud size 14` writes the file,
//! the running overlay notices the change and redraws. That keeps the CLI, the
//! GUI and the overlay decoupled, and works identically on every platform.
//!
//! Settings are a **list of widgets** rather than one flat crosshair. The
//! crosshair is simply the first widget kind; a clock or a timer is a new
//! [`Kind`] variant plus a draw arm in `render.rs`, not a reshuffle of
//! everything. Each widget carries its own position, monitor and opacity,
//! because "the clock in the corner of my second screen, dimmer than the
//! crosshair" is the normal case rather than an exception.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------- crosshair --

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

/// Everything specific to drawing a crosshair. Position, monitor and opacity
/// live on the [`Widget`] instead, since every widget kind needs those.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Crosshair {
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
}

impl Default for Crosshair {
    fn default() -> Self {
        Self {
            style: Style::Cross,
            color: "#00ff00".into(),
            outline_color: "#000000".into(),
            size: 10.0,
            thickness: 2.0,
            gap: 4.0,
            dot: 0.0,
            outline: 1.0,
        }
    }
}

// ------------------------------------------------------------------- anchors --

/// Where on the screen a widget's offset is measured from.
///
/// A crosshair wants the centre; a clock almost never does. Anchoring rather
/// than storing absolute coordinates means a widget stays where it was put when
/// the resolution changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Anchor {
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Anchor {
    pub const ALL: [Anchor; 9] = [
        Anchor::Center,
        Anchor::Top,
        Anchor::Bottom,
        Anchor::Left,
        Anchor::Right,
        Anchor::TopLeft,
        Anchor::TopRight,
        Anchor::BottomLeft,
        Anchor::BottomRight,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Anchor::Center => "center",
            Anchor::Top => "top",
            Anchor::Bottom => "bottom",
            Anchor::Left => "left",
            Anchor::Right => "right",
            Anchor::TopLeft => "topLeft",
            Anchor::TopRight => "topRight",
            Anchor::BottomLeft => "bottomLeft",
            Anchor::BottomRight => "bottomRight",
        }
    }

    /// Accepts the camelCase form and a few obvious spellings, so
    /// `foghud anchor top-left` and `top_left` both work.
    pub fn parse(s: &str) -> Option<Self> {
        let want: String = s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect();
        Self::ALL
            .into_iter()
            .find(|a| a.as_str().to_ascii_lowercase() == want)
    }

    /// The point on a `width` x `height` screen this anchor names.
    pub fn origin(self, width: f32, height: f32) -> (f32, f32) {
        let (cx, cy) = (width / 2.0, height / 2.0);
        match self {
            Anchor::Center => (cx, cy),
            Anchor::Top => (cx, 0.0),
            Anchor::Bottom => (cx, height),
            Anchor::Left => (0.0, cy),
            Anchor::Right => (width, cy),
            Anchor::TopLeft => (0.0, 0.0),
            Anchor::TopRight => (width, 0.0),
            Anchor::BottomLeft => (0.0, height),
            Anchor::BottomRight => (width, height),
        }
    }
}

// ------------------------------------------------------------------- widgets --

/// What a widget actually is.
///
/// Externally tagged (`"kind": { "crosshair": { ... } }`) rather than flattened:
/// `#[serde(flatten)]` combined with container-level `#[serde(default)]` is a
/// long-standing serde trap, and a config file that silently loses its defaults
/// is a worse outcome than one extra level of nesting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Crosshair(Crosshair),
}

impl Kind {
    /// Name used in the CLI, the GUI and hint text.
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Crosshair(_) => "crosshair",
        }
    }
}

impl Default for Kind {
    fn default() -> Self {
        Kind::Crosshair(Crosshair::default())
    }
}

/// One drawable thing on the overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Widget {
    /// Stable handle for the CLI and the GUI's selection. Unique per config.
    pub id: String,
    pub enabled: bool,
    /// `all`, `primary`, or an output/display name such as `DP-3`.
    pub monitor: String,
    pub anchor: Anchor,
    pub offset_x: f32,
    pub offset_y: f32,
    pub opacity: f32,
    pub kind: Kind,
}

impl Default for Widget {
    fn default() -> Self {
        Self {
            id: "crosshair".into(),
            enabled: true,
            monitor: "primary".into(),
            anchor: Anchor::Center,
            offset_x: 0.0,
            offset_y: 0.0,
            opacity: 1.0,
            kind: Kind::default(),
        }
    }
}

impl Widget {
    /// The crosshair settings, if this widget is one.
    pub fn crosshair(&self) -> Option<&Crosshair> {
        match &self.kind {
            Kind::Crosshair(c) => Some(c),
        }
    }

    pub fn crosshair_mut(&mut self) -> Option<&mut Crosshair> {
        match &mut self.kind {
            Kind::Crosshair(c) => Some(c),
        }
    }

    /// Whether this widget should be drawn on the given output.
    ///
    /// `index` is the output's position in the compositor's list; output 0 is
    /// treated as primary, matching how the backends enumerate.
    pub fn wants_screen(&self, name: &str, index: usize) -> bool {
        if !self.enabled {
            return false;
        }
        match self.monitor.as_str() {
            "all" => true,
            "primary" => index == 0,
            want => name.eq_ignore_ascii_case(want),
        }
    }
}

// -------------------------------------------------------------------- cycles --

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

pub fn percent(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

// ------------------------------------------------------------------- hotkeys --

/// The F-key mapping, in panel order.
///
/// Both backends bind from this table and the hint panel is generated from it,
/// so a key, the setting it steps and the label describing it cannot drift
/// apart. Editing this array is the only thing needed to remap a key.
pub const HOTKEYS: [(&str, &str); 4] = [
    ("F1", "style"),
    ("F2", "size"),
    ("F3", "color"),
    ("F4", "opacity"),
];

/// One setting as it appears in the hint panel: padded name, then value.
///
/// The single source of the wording, shared by `apply_cycle` and `full_hint` so
/// a value can't be described one way after a keypress and another in the
/// listing. `None` for a setting this widget doesn't have.
pub fn label(widget: &Widget, what: &str) -> Option<String> {
    let value = match what {
        "opacity" => percent(widget.opacity),
        "style" => widget.crosshair()?.style.as_str().to_string(),
        "size" => format!("{}", widget.crosshair()?.size as i32),
        "color" | "colour" => widget.crosshair()?.color.clone(),
        _ => return None,
    };
    let name = if what == "colour" { "color" } else { what };
    Some(format!("{name:<8} {value}"))
}

/// Steps one setting to its next preset and returns the hint text for it.
///
/// Both the CLI and the Windows hotkey handler go through here, so a hotkey
/// press behaves identically on either platform. Returns `None` for a setting
/// this widget doesn't have.
pub fn apply_cycle(widget: &mut Widget, what: &str) -> Option<String> {
    match what {
        "opacity" => widget.opacity = next_f32(&OPACITY_CYCLE, widget.opacity),
        "style" => {
            let c = widget.crosshair_mut()?;
            c.style = c.style.next();
        }
        "size" => {
            let c = widget.crosshair_mut()?;
            c.size = next_f32(&SIZE_CYCLE, c.size);
        }
        "color" | "colour" => {
            let c = widget.crosshair_mut()?;
            c.color = next_str(&COLOR_CYCLE, &c.color).to_string();
        }
        _ => return None,
    }
    label(widget, what)
}

/// The panel shown when the crosshair is switched on: every key and its value.
pub fn full_hint(widget: &Widget) -> String {
    let mut out = String::from("Crosshair on");
    for (key, what) in HOTKEYS {
        if let Some(line) = label(widget, what) {
            out.push_str(&format!("\n{key}  {line}"));
        }
    }
    out
}

// -------------------------------------------------------------------- config --

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub widgets: Vec<Widget>,
    pub hotkeys: bool,

    /// Text of the hint panel to show. Set by the CLI or the GUI.
    pub notice: String,
    /// Bumped every time a new hint should be shown. The overlay watches this
    /// rather than the text, so repeating the same hint still re-displays it.
    pub notice_id: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            widgets: vec![Widget::default()],
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
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::parse(&text).unwrap_or_else(|e| {
            eprintln!("foghud: config is not usable ({e}), using defaults");
            Self::default()
        })
    }

    /// Parses either the current widget-list format or the older flat crosshair
    /// one, migrating the latter. Split out from [`load`] so both paths are
    /// testable without touching the filesystem.
    pub fn parse(text: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(text).context("invalid JSON")?;
        // The presence of `widgets` is what distinguishes the formats. A v0 file
        // has crosshair fields at the top level and no widget list at all.
        if value.get("widgets").is_some() {
            serde_json::from_value(value).context("unexpected settings shape")
        } else {
            let legacy: LegacyConfig =
                serde_json::from_value(value).context("unexpected settings shape")?;
            Ok(legacy.migrate())
        }
    }

    /// The widget the top-level crosshair commands and the hotkeys act on.
    pub fn crosshair(&self) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.crosshair().is_some())
    }

    pub fn crosshair_mut(&mut self) -> Option<&mut Widget> {
        self.widgets.iter_mut().find(|w| w.crosshair().is_some())
    }

    /// Like [`crosshair_mut`], but creates the widget if the user deleted it, so
    /// `foghud size 14` can never fail with "no crosshair".
    pub fn ensure_crosshair(&mut self) -> &mut Widget {
        if self.crosshair().is_none() {
            self.widgets.push(Widget::default());
        }
        self.crosshair_mut().expect("just ensured")
    }

    /// Queues a hint panel. The bump is what tells the overlay this is a new
    /// hint rather than one it has already shown and expired.
    pub fn set_notice(&mut self, text: String) {
        self.notice = text;
        self.notice_id = self.notice_id.wrapping_add(1);
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

// ----------------------------------------------------------------- migration --

/// The pre-widget config: one crosshair, its settings at the top level.
///
/// Kept so upgrading doesn't silently reset a tuned crosshair to defaults. Can
/// be deleted once no v0 files are plausibly in the wild.
#[derive(Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LegacyConfig {
    enabled: bool,
    style: Style,
    color: String,
    outline_color: String,
    size: f32,
    thickness: f32,
    gap: f32,
    dot: f32,
    outline: f32,
    opacity: f32,
    offset_x: f32,
    offset_y: f32,
    monitor: String,
    hotkeys: bool,
    notice: String,
    notice_id: u64,
}

impl Default for LegacyConfig {
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

impl LegacyConfig {
    fn migrate(self) -> Config {
        Config {
            widgets: vec![Widget {
                id: "crosshair".into(),
                enabled: self.enabled,
                monitor: self.monitor,
                anchor: Anchor::Center,
                offset_x: self.offset_x,
                offset_y: self.offset_y,
                opacity: self.opacity,
                kind: Kind::Crosshair(Crosshair {
                    style: self.style,
                    color: self.color,
                    outline_color: self.outline_color,
                    size: self.size,
                    thickness: self.thickness,
                    gap: self.gap,
                    dot: self.dot,
                    outline: self.outline,
                }),
            }],
            hotkeys: self.hotkeys,
            notice: self.notice,
            // Carried over so the overlay doesn't mistake the migration for a
            // brand new hint and flash a panel on upgrade.
            notice_id: self.notice_id,
        }
    }
}

// -------------------------------------------------------------------- colour --

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

/// `#rrggbb` for a colour, used when the GUI writes a picker's value back.
pub fn to_hex(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crosshair_of(cfg: &Config) -> &Crosshair {
        cfg.crosshair().unwrap().crosshair().unwrap()
    }

    #[test]
    fn hex_forms_parse() {
        assert_eq!(parse_color("#ff0000"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("#f00"), Some([255, 0, 0, 255]));
        // Alpha leads in the 8-digit form.
        assert_eq!(parse_color("#80ff0000"), Some([255, 0, 0, 128]));
        assert_eq!(parse_color("cyan"), Some([0, 229, 255, 255]));
        assert_eq!(parse_color("#gg0000"), None);
        assert_eq!(parse_color("nonsense"), None);
        assert_eq!(to_hex([0, 229, 255, 255]), "#00e5ff");
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
    fn hint_panel_follows_the_hotkey_table() {
        let cfg = Config::default();
        let hint = full_hint(cfg.crosshair().unwrap());
        let lines: Vec<&str> = hint.lines().collect();
        assert_eq!(lines[0], "Crosshair on");
        assert_eq!(lines.len(), HOTKEYS.len() + 1);
        for (i, (key, what)) in HOTKEYS.iter().enumerate() {
            let line = lines[i + 1];
            assert!(line.starts_with(key), "line {i} should start with {key}");
            assert!(line.contains(what), "line {i} should name {what}");
        }
        // The requested order, spelled out.
        assert_eq!(lines[3], "F3  color    #00ff00");
        assert_eq!(lines[4], "F4  opacity  100%");
    }

    #[test]
    fn cycling_and_listing_describe_a_value_identically() {
        let mut widget = Widget::default();
        let after_press = apply_cycle(&mut widget, "size").unwrap();
        assert_eq!(after_press, label(&widget, "size").unwrap());
        assert!(full_hint(&widget).contains(&after_press));
    }

    #[test]
    fn opacity_cycles_on_the_widget_not_the_crosshair() {
        let mut widget = Widget::default();
        assert_eq!(widget.opacity, 1.0);
        apply_cycle(&mut widget, "opacity").unwrap();
        assert_eq!(widget.opacity, 0.85);
    }

    #[test]
    fn unknown_settings_are_rejected() {
        assert_eq!(label(&Widget::default(), "nonsense"), None);
        assert_eq!(apply_cycle(&mut Widget::default(), "nonsense"), None);
    }

    #[test]
    fn round_trips_through_json() {
        let mut cfg = Config::default();
        cfg.ensure_crosshair().anchor = Anchor::BottomRight;
        cfg.ensure_crosshair().offset_x = -40.0;
        cfg.ensure_crosshair().crosshair_mut().unwrap().size = 22.0;

        let text = serde_json::to_string_pretty(&cfg).unwrap();
        let back = Config::parse(&text).unwrap();
        assert_eq!(back.widgets.len(), 1);
        let w = back.crosshair().unwrap();
        assert_eq!(w.anchor, Anchor::BottomRight);
        assert_eq!(w.offset_x, -40.0);
        assert_eq!(crosshair_of(&back).size, 22.0);
    }

    #[test]
    fn partial_widget_keeps_defaults() {
        let cfg = Config::parse(r#"{"widgets":[{"id":"x"}]}"#).unwrap();
        let w = &cfg.widgets[0];
        assert_eq!(w.id, "x");
        // Everything unspecified falls back rather than zeroing out.
        assert_eq!(w.opacity, 1.0);
        assert_eq!(w.anchor, Anchor::Center);
        assert!(w.enabled);
        assert_eq!(crosshair_of(&cfg).style, Style::Cross);
        assert!(cfg.hotkeys);
    }

    /// The upgrade path that matters: a tuned v0 config must not reset.
    #[test]
    fn legacy_flat_config_migrates() {
        let v0 = r##"{
            "enabled": true,
            "style": "circle",
            "color": "#ff2b2b",
            "size": 22.0,
            "thickness": 3.0,
            "opacity": 0.5,
            "offsetX": 12.0,
            "offsetY": -8.0,
            "monitor": "DP-3",
            "hotkeys": false,
            "noticeId": 10
        }"##;
        let cfg = Config::parse(v0).unwrap();
        assert_eq!(cfg.widgets.len(), 1);
        let w = cfg.crosshair().unwrap();
        assert_eq!(w.monitor, "DP-3");
        assert_eq!(w.opacity, 0.5);
        assert_eq!(w.offset_x, 12.0);
        assert_eq!(w.offset_y, -8.0);
        // A v0 crosshair was always screen-centred.
        assert_eq!(w.anchor, Anchor::Center);
        assert_eq!(crosshair_of(&cfg).style, Style::Circle);
        assert_eq!(crosshair_of(&cfg).color, "#ff2b2b");
        assert_eq!(crosshair_of(&cfg).size, 22.0);
        assert!(!cfg.hotkeys);
        // Carried over, or the overlay flashes a stale hint on upgrade.
        assert_eq!(cfg.notice_id, 10);
    }

    #[test]
    fn legacy_partial_config_keeps_defaults() {
        let cfg = Config::parse(r#"{"size": 20}"#).unwrap();
        assert_eq!(crosshair_of(&cfg).size, 20.0);
        assert_eq!(crosshair_of(&cfg).style, Style::Cross);
        assert!(cfg.crosshair().unwrap().enabled);
    }

    #[test]
    fn broken_json_is_an_error_not_a_panic() {
        assert!(Config::parse("{not json").is_err());
    }

    #[test]
    fn anchors_resolve_to_screen_points() {
        assert_eq!(Anchor::Center.origin(1000.0, 600.0), (500.0, 300.0));
        assert_eq!(Anchor::TopLeft.origin(1000.0, 600.0), (0.0, 0.0));
        assert_eq!(Anchor::BottomRight.origin(1000.0, 600.0), (1000.0, 600.0));
        assert_eq!(Anchor::Top.origin(1000.0, 600.0), (500.0, 0.0));
        assert_eq!(Anchor::Right.origin(1000.0, 600.0), (1000.0, 300.0));
    }

    #[test]
    fn anchor_names_round_trip_and_forgive_spelling() {
        for a in Anchor::ALL {
            assert_eq!(Anchor::parse(a.as_str()), Some(a));
        }
        assert_eq!(Anchor::parse("top-left"), Some(Anchor::TopLeft));
        assert_eq!(Anchor::parse("bottom_right"), Some(Anchor::BottomRight));
        assert_eq!(Anchor::parse("CENTER"), Some(Anchor::Center));
        assert_eq!(Anchor::parse("nowhere"), None);
    }

    #[test]
    fn monitor_selection_covers_all_primary_and_named() {
        let mut w = Widget {
            monitor: "all".into(),
            ..Default::default()
        };
        assert!(w.wants_screen("DP-3", 0));
        assert!(w.wants_screen("DP-2", 1));

        w.monitor = "primary".into();
        assert!(w.wants_screen("DP-3", 0));
        assert!(!w.wants_screen("DP-2", 1));

        w.monitor = "dp-2".into();
        assert!(!w.wants_screen("DP-3", 0));
        assert!(w.wants_screen("DP-2", 1));

        // A disabled widget is on no screen at all.
        w.enabled = false;
        w.monitor = "all".into();
        assert!(!w.wants_screen("DP-3", 0));
    }

    #[test]
    fn a_deleted_crosshair_is_recreated() {
        let mut cfg = Config {
            widgets: Vec::new(),
            ..Default::default()
        };
        assert!(cfg.crosshair().is_none());
        cfg.ensure_crosshair().crosshair_mut().unwrap().size = 14.0;
        assert_eq!(crosshair_of(&cfg).size, 14.0);
    }
}
