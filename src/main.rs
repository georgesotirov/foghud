mod config;
mod daemon;
mod platform;
mod render;
mod text;

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

/// Crosshair commands sit at the top level rather than under a `crosshair`
/// noun: it's the thing you reach for constantly, and `foghud size 14` beats
/// `foghud crosshair size 14` several times a match. Later features get their
/// own noun (`foghud stats`).
#[derive(Subcommand)]
enum Command {
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
    /// Nudge away from the centre of the screen
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
        Command::Run => platform::run_overlay(),
        Command::Config { action } => config_cmd(action),
        other => crosshair(other),
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

/// Load, mutate, save. Every setting change goes through here.
fn update(f: impl FnOnce(&mut Config) -> Result<()>) -> Result<()> {
    let mut cfg = Config::load();
    f(&mut cfg)?;
    cfg.save()
}

/// Like [`update`], but also raises a hint panel describing what changed. The
/// hint is built after the mutation so it reports the new value.
fn update_with_hint(
    hint: impl FnOnce(&Config) -> String,
    f: impl FnOnce(&mut Config) -> Result<()>,
) -> Result<()> {
    let mut cfg = Config::load();
    f(&mut cfg)?;
    cfg.notice = hint(&cfg);
    cfg.notice_id = cfg.notice_id.wrapping_add(1);
    cfg.save()
}

fn percent(v: f32) -> String {
    format!("{}%", (v * 100.0).round() as i32)
}

/// The panel shown when the crosshair is switched on: every key and its value.
fn full_hint(cfg: &Config) -> String {
    format!(
        "Crosshair on\nF1  style    {}\nF2  size     {}\nF3  opacity  {}\nF4  color    {}",
        cfg.style.as_str(),
        cfg.size as i32,
        percent(cfg.opacity),
        cfg.color,
    )
}

fn start() -> Result<()> {
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
                update_with_hint(full_hint, |c| {
                    c.enabled = true;
                    Ok(())
                })?;
                return start();
            }
            if Config::load().enabled {
                update(|c| {
                    c.enabled = false;
                    Ok(())
                })?;
            } else {
                update_with_hint(full_hint, |c| {
                    c.enabled = true;
                    Ok(())
                })?;
            }
        }

        Command::On => {
            update_with_hint(full_hint, |c| {
                c.enabled = true;
                Ok(())
            })?;
            if !daemon::is_running() {
                return start();
            }
        }

        Command::Off => update(|c| {
            c.enabled = false;
            Ok(())
        })?,

        Command::Status => {
            let cfg = Config::load();
            if daemon::is_running() {
                let vis = if cfg.enabled { "visible" } else { "hidden" };
                println!("running, crosshair is {vis}");
            } else {
                println!("not running");
            }
        }

        Command::Color { value } => {
            require_color(&value)?;
            update_with_hint(
                |c| format!("color    {}", c.color),
                |c| {
                    c.color = value;
                    Ok(())
                },
            )?
        }

        Command::OutlineColor { value } => {
            require_color(&value)?;
            update(|c| {
                c.outline_color = value;
                Ok(())
            })?
        }

        Command::Size { value } => update_with_hint(
            |c| format!("size     {}", c.size as i32),
            |c| {
                c.size = value.max(0.0);
                Ok(())
            },
        )?,

        Command::Opacity { value } => update_with_hint(
            |c| format!("opacity  {}", percent(c.opacity)),
            |c| {
                c.opacity = value.clamp(0.0, 1.0);
                Ok(())
            },
        )?,

        Command::Style { value } => {
            let Some(style) = Style::parse(&value) else {
                bail!("unknown style '{value}' — pick one of: cross, tcross, circle, dot");
            };
            update_with_hint(
                |c| format!("style    {}", c.style.as_str()),
                |c| {
                    c.style = style;
                    Ok(())
                },
            )?
        }

        Command::Thickness { value } => update(|c| {
            c.thickness = value.max(1.0);
            Ok(())
        })?,
        Command::Gap { value } => update(|c| {
            c.gap = value.max(0.0);
            Ok(())
        })?,
        Command::Dot { value } => update(|c| {
            c.dot = value.max(0.0);
            Ok(())
        })?,
        Command::Outline { value } => update(|c| {
            c.outline = value.max(0.0);
            Ok(())
        })?,
        Command::Monitor { value } => update(|c| {
            c.monitor = value;
            Ok(())
        })?,
        Command::Offset { x, y } => update(|c| {
            c.offset_x = x;
            c.offset_y = y;
            Ok(())
        })?,
        Command::Hotkeys { value } => update(|c| {
            c.hotkeys = value;
            Ok(())
        })?,

        Command::Cycle { what } => match what.as_str() {
            "style" => update_with_hint(
                |c| format!("style    {}", c.style.as_str()),
                |c| {
                    c.style = c.style.next();
                    Ok(())
                },
            )?,
            "size" => update_with_hint(
                |c| format!("size     {}", c.size as i32),
                |c| {
                    c.size = config::next_f32(&config::SIZE_CYCLE, c.size);
                    Ok(())
                },
            )?,
            "opacity" => update_with_hint(
                |c| format!("opacity  {}", percent(c.opacity)),
                |c| {
                    c.opacity = config::next_f32(&config::OPACITY_CYCLE, c.opacity);
                    Ok(())
                },
            )?,
            "color" | "colour" => update_with_hint(
                |c| format!("color    {}", c.color),
                |c| {
                    c.color = config::next_str(&config::COLOR_CYCLE, &c.color).to_string();
                    Ok(())
                },
            )?,
            other => bail!("cannot cycle '{other}' — try: style, size, opacity, color"),
        },

        Command::Config { .. } | Command::Run => unreachable!("handled by run()"),
    }
    Ok(())
}
