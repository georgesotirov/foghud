mod config;
mod daemon;
mod platform;
mod render;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use config::{Config, Style};

#[derive(Parser)]
#[command(
    name = "foghud",
    version,
    about = "Overlay toolkit for Dead by Daylight",
    long_about = "Overlay toolkit for Dead by Daylight.\n\nRuns outside the game as an ordinary \
                  desktop overlay — it never reads, writes or injects into the game process."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Crosshair overlay
    Crosshair {
        #[command(subcommand)]
        action: Option<Crosshair>,
    },
    /// Inspect or reset the config file
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// Run the overlay in the foreground (used internally by `crosshair start`)
    #[command(hide = true)]
    Run,
}

#[derive(Subcommand)]
enum Crosshair {
    /// Start the overlay (the default when no action is given)
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
    /// Nudge away from the centre of the screen
    Offset {
        x: f32,
        y: f32,
    },
    /// Step a setting to its next preset (what the hotkeys call)
    Cycle {
        what: String,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the path of the config file
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
        Command::Run => platform::run_overlay(),
        Command::Config { action } => config_cmd(action),
        Command::Crosshair { action } => crosshair(action.unwrap_or(Crosshair::Start)),
    }
}

fn config_cmd(action: ConfigCmd) -> Result<()> {
    match action {
        ConfigCmd::Path => println!("{}", Config::path()?.display()),
        ConfigCmd::Show => println!("{}", serde_json::to_string_pretty(&Config::load())?),
        ConfigCmd::Reset => {
            Config::default().save()?;
            println!("settings reset to defaults");
        }
    }
    Ok(())
}

/// Load, mutate, save. Every setting change goes through here.
fn update(f: impl FnOnce(&mut Config) -> Result<()>) -> Result<()> {
    let mut cfg = Config::load();
    f(&mut cfg)?;
    cfg.save()
}

fn start() -> Result<()> {
    if daemon::is_running() {
        println!("crosshair is already running");
        return Ok(());
    }
    // Make sure a config exists before the overlay goes looking for one.
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

fn crosshair(action: Crosshair) -> Result<()> {
    match action {
        Crosshair::Start => start()?,

        Crosshair::Stop => {
            if daemon::stop()? {
                println!("crosshair stopped");
            } else {
                println!("crosshair is not running");
            }
        }

        Crosshair::Restart => {
            daemon::stop()?;
            start()?;
        }

        Crosshair::Toggle => {
            // The first press of a hotkey doubles as "launch it".
            if !daemon::is_running() {
                update(|c| {
                    c.enabled = true;
                    Ok(())
                })?;
                return start();
            }
            update(|c| {
                c.enabled = !c.enabled;
                Ok(())
            })?;
        }

        Crosshair::On => {
            update(|c| {
                c.enabled = true;
                Ok(())
            })?;
            if !daemon::is_running() {
                return start();
            }
        }

        Crosshair::Off => update(|c| {
            c.enabled = false;
            Ok(())
        })?,

        Crosshair::Status => {
            let cfg = Config::load();
            if daemon::is_running() {
                let vis = if cfg.enabled { "visible" } else { "hidden" };
                println!("running, crosshair is {vis}");
            } else {
                println!("not running");
            }
        }

        Crosshair::Color { value } => update(|c| {
            if !config::is_valid_color(&value) {
                bail!("'{value}' is not a colour — use #rrggbb, #aarrggbb, or a name like 'cyan'");
            }
            c.color = value;
            Ok(())
        })?,

        Crosshair::OutlineColor { value } => update(|c| {
            if !config::is_valid_color(&value) {
                bail!("'{value}' is not a colour — use #rrggbb, #aarrggbb, or a name like 'cyan'");
            }
            c.outline_color = value;
            Ok(())
        })?,

        Crosshair::Size { value } => update(|c| {
            c.size = value.max(0.0);
            Ok(())
        })?,
        Crosshair::Thickness { value } => update(|c| {
            c.thickness = value.max(1.0);
            Ok(())
        })?,
        Crosshair::Gap { value } => update(|c| {
            c.gap = value.max(0.0);
            Ok(())
        })?,
        Crosshair::Dot { value } => update(|c| {
            c.dot = value.max(0.0);
            Ok(())
        })?,
        Crosshair::Outline { value } => update(|c| {
            c.outline = value.max(0.0);
            Ok(())
        })?,
        Crosshair::Opacity { value } => update(|c| {
            c.opacity = value.clamp(0.0, 1.0);
            Ok(())
        })?,

        Crosshair::Style { value } => update(|c| {
            let Some(style) = Style::parse(&value) else {
                bail!("unknown style '{value}' — pick one of: cross, tcross, circle, dot");
            };
            c.style = style;
            Ok(())
        })?,

        Crosshair::Monitor { value } => update(|c| {
            c.monitor = value;
            Ok(())
        })?,

        Crosshair::Offset { x, y } => update(|c| {
            c.offset_x = x;
            c.offset_y = y;
            Ok(())
        })?,

        Crosshair::Cycle { what } => update(|c| {
            match what.as_str() {
                "style" => c.style = c.style.next(),
                "size" => c.size = config::next_f32(&config::SIZE_CYCLE, c.size),
                "opacity" => c.opacity = config::next_f32(&config::OPACITY_CYCLE, c.opacity),
                "color" | "colour" => {
                    c.color = config::next_str(&config::COLOR_CYCLE, &c.color).to_string()
                }
                other => bail!("cannot cycle '{other}' — try: style, size, opacity, color"),
            }
            Ok(())
        })?,
    }
    Ok(())
}
