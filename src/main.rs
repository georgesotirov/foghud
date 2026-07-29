mod config;
mod daemon;
mod gui;
mod platform;
mod render;
mod text;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use config::{Anchor, Config, Style, Widget, full_hint};

#[derive(Parser)]
#[command(
    name = "foghud",
    version,
    about = "Overlay toolkit for Dead by Daylight",
    long_about = "Overlay toolkit for Dead by Daylight.\n\nRun with no arguments to open the \
                  control panel. Runs outside the game as an ordinary desktop overlay — it never \
                  reads, writes or injects into the game process."
)]
struct Cli {
    /// With no subcommand, `foghud` opens the control panel.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Crosshair commands sit at the top level rather than under a `crosshair`
/// noun: it's the thing you reach for constantly, and `foghud size 14` beats
/// `foghud crosshair size 14` several times a match. Later widget kinds get
/// their own noun (`foghud clock ...`).
#[derive(Subcommand)]
enum Command {
    /// Open the control panel
    Gui,
    /// Start the overlay
    Start,
    /// Stop the overlay process
    Stop,
    Restart,
    /// Show or hide without stopping the process
    Toggle,
    On,
    Off,
    Status,

    /// #rrggbb, #aarrggbb, or a name such as `cyan`
    Color {
        value: String,
    },
    OutlineColor {
        value: String,
    },
    /// Arm length in pixels
    Size {
        value: f32,
    },
    Thickness {
        value: f32,
    },
    /// Empty space between the centre and the arms
    Gap {
        value: f32,
    },
    /// Centre dot radius, 0 for none
    Dot {
        value: f32,
    },
    /// Border width, 0 for none
    Outline {
        value: f32,
    },
    /// 0.0 to 1.0
    Opacity {
        value: f32,
    },
    /// cross, tcross, circle, or dot
    Style {
        value: String,
    },
    /// all, primary, or a display name such as DP-3
    Monitor {
        value: String,
    },
    /// Where the offset is measured from: center, topLeft, bottomRight, ...
    Anchor {
        value: String,
    },
    /// Nudge away from the anchor
    Offset {
        x: f32,
        y: f32,
    },
    /// Turn the F1-F4 hotkeys on or off
    Hotkeys {
        value: bool,
    },

    /// Step a setting to its next preset (what F1-F4 call)
    Cycle {
        what: String,
    },

    /// Inspect or reset the settings file
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },

    /// Run the overlay in the foreground (used internally by `start`)
    #[command(hide = true)]
    Run,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the path of the settings file
    Path,
    /// Print the current settings
    Show,
    /// Restore every setting to its default
    Reset,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("foghud: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        // Bare `foghud` opens the panel: the common case is wanting to look at
        // the thing, not to remember a verb.
        None | Some(Command::Gui) => gui::run(),
        Some(Command::Run) => platform::run_overlay(),
        Some(Command::Config { action }) => config_cmd(action),
        Some(other) => crosshair(other),
    }
}

fn config_cmd(action: ConfigCmd) -> Result<()> {
    match action {
        ConfigCmd::Path => println!("{}", Config::path()?.display()),
        ConfigCmd::Show => println!("{}", serde_json::to_string_pretty(&Config::load())?),
        ConfigCmd::Reset => {
            // Keep the hint counter monotonic, or the overlay would mistake a
            // reset for a hint it has already shown.
            let cfg = Config {
                notice_id: Config::load().notice_id.wrapping_add(1),
                ..Default::default()
            };
            cfg.save()?;
            println!("settings reset to defaults");
        }
    }
    Ok(())
}

/// Load, mutate the crosshair widget, save. Every setting change goes through
/// here or [`update_hinted`].
fn update(f: impl FnOnce(&mut Widget) -> Result<()>) -> Result<()> {
    let mut cfg = Config::load();
    f(cfg.ensure_crosshair())?;
    cfg.save()
}

/// Like [`update`], but also raises a hint panel naming the setting that
/// changed. The label comes from `config::label`, so a CLI change and a hotkey
/// press describe themselves with the same words.
fn update_hinted(what: &str, f: impl FnOnce(&mut Widget) -> Result<()>) -> Result<()> {
    let mut cfg = Config::load();
    f(cfg.ensure_crosshair())?;
    if let Some(hint) = config::label(cfg.ensure_crosshair(), what) {
        cfg.set_notice(hint);
    }
    cfg.save()
}

/// Turns the crosshair on and shows the full key listing.
fn switch_on() -> Result<()> {
    let mut cfg = Config::load();
    let widget = cfg.ensure_crosshair();
    widget.enabled = true;
    let hint = full_hint(cfg.ensure_crosshair());
    cfg.set_notice(hint);
    cfg.save()
}

pub fn start() -> Result<()> {
    if daemon::is_running() {
        println!("crosshair is already running");
        return Ok(());
    }
    if Config::path().is_ok_and(|p| !p.exists()) {
        Config::load().save()?;
    }
    daemon::spawn_detached()?;
    for _ in 0..60 {
        if daemon::is_running() {
            println!("crosshair started");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!("the overlay did not come up — run `foghud run` to see why")
}

fn require_color(value: &str) -> Result<()> {
    if config::is_valid_color(value) {
        Ok(())
    } else {
        bail!("'{value}' is not a colour — use #rrggbb, #aarrggbb, or a name like 'cyan'")
    }
}

fn crosshair(action: Command) -> Result<()> {
    match action {
        Command::Start => start()?,

        Command::Stop => {
            if daemon::stop()? {
                println!("crosshair stopped");
            } else {
                println!("crosshair is not running");
            }
        }

        Command::Restart => {
            daemon::stop()?;
            start()?;
        }

        Command::Toggle => {
            // The first press of the hotkey doubles as "launch it".
            if !daemon::is_running() {
                switch_on()?;
                return start();
            }
            if Config::load().crosshair().is_some_and(|w| w.enabled) {
                update(|w| {
                    w.enabled = false;
                    Ok(())
                })?;
            } else {
                switch_on()?;
            }
        }

        Command::On => {
            switch_on()?;
            if !daemon::is_running() {
                return start();
            }
        }

        Command::Off => update(|w| {
            w.enabled = false;
            Ok(())
        })?,

        Command::Status => {
            let cfg = Config::load();
            if daemon::is_running() {
                let visible = cfg.crosshair().is_some_and(|w| w.enabled);
                let vis = if visible { "visible" } else { "hidden" };
                println!("running, crosshair is {vis}");
            } else {
                println!("not running");
            }
        }

        Command::Color { value } => {
            require_color(&value)?;
            update_hinted("color", |w| {
                w.crosshair_mut().expect("crosshair widget").color = value;
                Ok(())
            })?
        }

        Command::OutlineColor { value } => {
            require_color(&value)?;
            update(|w| {
                w.crosshair_mut().expect("crosshair widget").outline_color = value;
                Ok(())
            })?
        }

        Command::Size { value } => update_hinted("size", |w| {
            w.crosshair_mut().expect("crosshair widget").size = value.max(0.0);
            Ok(())
        })?,

        Command::Opacity { value } => update_hinted("opacity", |w| {
            w.opacity = value.clamp(0.0, 1.0);
            Ok(())
        })?,

        Command::Style { value } => {
            let Some(style) = Style::parse(&value) else {
                bail!("unknown style '{value}' — pick one of: cross, tcross, circle, dot");
            };
            update_hinted("style", |w| {
                w.crosshair_mut().expect("crosshair widget").style = style;
                Ok(())
            })?
        }

        Command::Anchor { value } => {
            let Some(anchor) = Anchor::parse(&value) else {
                let names: Vec<&str> = Anchor::ALL.iter().map(|a| a.as_str()).collect();
                bail!(
                    "unknown anchor '{value}' — pick one of: {}",
                    names.join(", ")
                );
            };
            update(|w| {
                w.anchor = anchor;
                Ok(())
            })?
        }

        Command::Thickness { value } => update(|w| {
            w.crosshair_mut().expect("crosshair widget").thickness = value.max(1.0);
            Ok(())
        })?,
        Command::Gap { value } => update(|w| {
            w.crosshair_mut().expect("crosshair widget").gap = value.max(0.0);
            Ok(())
        })?,
        Command::Dot { value } => update(|w| {
            w.crosshair_mut().expect("crosshair widget").dot = value.max(0.0);
            Ok(())
        })?,
        Command::Outline { value } => update(|w| {
            w.crosshair_mut().expect("crosshair widget").outline = value.max(0.0);
            Ok(())
        })?,
        Command::Monitor { value } => update(|w| {
            w.monitor = value;
            Ok(())
        })?,
        Command::Offset { x, y } => update(|w| {
            w.offset_x = x;
            w.offset_y = y;
            Ok(())
        })?,

        // Hotkeys are global rather than per-widget, so this one bypasses the
        // crosshair helpers.
        Command::Hotkeys { value } => {
            let mut cfg = Config::load();
            cfg.hotkeys = value;
            cfg.save()?;
        }

        Command::Cycle { what } => {
            let mut cfg = Config::load();
            let Some(hint) = config::apply_cycle(cfg.ensure_crosshair(), &what) else {
                bail!("cannot cycle '{what}' — try: style, size, color, opacity");
            };
            cfg.set_notice(hint);
            cfg.save()?;
        }

        Command::Gui | Command::Config { .. } | Command::Run => unreachable!("handled by run()"),
    }
    Ok(())
}
