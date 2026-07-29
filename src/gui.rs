//! The control panel.
//!
//! Deliberately not a special case: the panel is just another writer of the
//! config file, exactly like the CLI and the hotkeys. It holds no state the
//! overlay needs, so it can be opened and closed at any time, and the overlay
//! picks up its edits through the same mtime poll as everything else.
//!
//! Because the file is shared, the panel also *watches* it. Press F3 with the
//! panel open and the colour swatch follows — otherwise the two would disagree
//! and whichever wrote last would clobber the other.

use anyhow::Result;
use eframe::egui;
use std::time::{Duration, Instant, SystemTime};

use crate::config::{Anchor, Config, Crosshair, HOTKEYS, Kind, Style, Widget, parse_color, to_hex};
use crate::{daemon, platform, render};

/// How long to sit on edits before writing. A slider drag fires every frame and
/// the overlay only reads every 150ms, so batching costs nothing visible.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(80);
/// How often to check whether something else changed the file.
const RELOAD_POLL: Duration = Duration::from_millis(300);

/// Rasterised at this many pixels square, and shown 1:1 when there's room.
const PREVIEW_PX: u32 = 240;
const PREVIEW_W: f32 = 264.0;
const SIDEBAR_W: f32 = 200.0;

const WINDOW_W: u32 = 960;
const WINDOW_H: u32 = 660;

pub fn run() -> Result<()> {
    // Opening the panel implies wanting to see the overlay. Failing to start
    // isn't fatal — the panel is still where you'd go to work out why.
    if !daemon::is_running()
        && let Err(err) = crate::start()
    {
        eprintln!("foghud: {err:#}");
    }

    // The compositor is free to ignore this; `float_control_panel` is what
    // actually gets it honoured under a tiling layout.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_W as f32, WINDOW_H as f32])
            .with_min_inner_size([560.0, 400.0])
            .with_title("foghud"),
        ..Default::default()
    };
    platform::float_control_panel(WINDOW_W, WINDOW_H);

    eframe::run_native(
        "foghud",
        options,
        Box::new(|_cc| Ok(Box::new(ControlPanel::new()))),
    )
    .map_err(|e| anyhow::anyhow!("could not open the control panel: {e}"))
}

struct ControlPanel {
    cfg: Config,
    /// Index into `cfg.widgets`, clamped every frame in case the list shrank.
    selected: usize,
    /// Set by any edit; cleared once the debounce elapses and we write.
    dirty: bool,
    last_write: Instant,
    last_reload_check: Instant,
    file_mtime: Option<SystemTime>,
    monitors: Vec<String>,
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path().ok()?)
        .ok()?
        .modified()
        .ok()
}

impl ControlPanel {
    fn new() -> Self {
        Self {
            cfg: Config::load(),
            selected: 0,
            dirty: false,
            last_write: Instant::now(),
            last_reload_check: Instant::now(),
            file_mtime: config_mtime(),
            monitors: platform::monitor_names(),
        }
    }

    fn flush(&mut self) {
        if let Err(err) = self.cfg.save() {
            eprintln!("foghud: {err:#}");
        }
        self.dirty = false;
        self.last_write = Instant::now();
        self.file_mtime = config_mtime();
    }

    /// Writes pending edits, and adopts the file if something else changed it.
    ///
    /// Our own edits win while they're pending: a hotkey press mid-drag must not
    /// yank the slider out from under the cursor.
    fn sync(&mut self) {
        if self.dirty && self.last_write.elapsed() >= WRITE_DEBOUNCE {
            self.flush();
        }
        if self.dirty || self.last_reload_check.elapsed() < RELOAD_POLL {
            return;
        }
        self.last_reload_check = Instant::now();
        let mtime = config_mtime();
        if mtime != self.file_mtime {
            self.file_mtime = mtime;
            self.cfg = Config::load();
        }
    }
}

impl eframe::App for ControlPanel {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sync();
        self.selected = self.selected.min(self.cfg.widgets.len().saturating_sub(1));

        // Come back without waiting for input, so an external change to the file
        // shows up on its own.
        ui.ctx().request_repaint_after(RELOAD_POLL);

        // Panels must all be declared before the central area claims what's left.
        self.top_bar(ui);
        self.widget_list(ui);
        self.preview(ui);
        self.settings(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Never lose the last edit to the debounce window.
        if self.dirty {
            self.flush();
        }
    }
}

// ------------------------------------------------------------------ sections --

impl ControlPanel {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let running = daemon::is_running();
        let mut hotkeys = self.cfg.hotkeys;
        let mut changed = false;
        let mut action: Option<Action> = None;

        egui::Panel::top(egui::Id::new("foghud_top")).show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("foghud");
                ui.separator();

                let (dot, text) = if running {
                    (egui::Color32::from_rgb(80, 220, 120), "overlay running")
                } else {
                    (egui::Color32::from_rgb(230, 90, 90), "overlay stopped")
                };
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 5.0, dot);
                ui.label(text);

                ui.add_space(10.0);
                if running {
                    if ui.button("Stop").clicked() {
                        action = Some(Action::Stop);
                    }
                    if ui.button("Restart").clicked() {
                        action = Some(Action::Restart);
                    }
                } else if ui.button("Start").clicked() {
                    action = Some(Action::Start);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    changed = ui
                        .checkbox(&mut hotkeys, "F-key hotkeys")
                        .on_hover_text(
                            "While the overlay runs these are grabbed by the compositor, \
                             so they won't reach other apps.",
                        )
                        .changed();
                });
            });
            ui.add_space(6.0);
        });

        if changed {
            self.cfg.hotkeys = hotkeys;
            self.dirty = true;
        }
        match action {
            Some(Action::Start) => drop_err(crate::start()),
            Some(Action::Stop) => drop_err(daemon::stop().map(|_| ())),
            Some(Action::Restart) => {
                drop_err(daemon::stop().map(|_| ()));
                drop_err(crate::start());
            }
            None => {}
        }
    }

    fn widget_list(&mut self, ui: &mut egui::Ui) {
        let selected = self.selected;
        let mut new_selected = selected;
        let mut changed = false;
        let cfg = &mut self.cfg;

        egui::Panel::left(egui::Id::new("foghud_widgets"))
            .exact_size(SIDEBAR_W)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("WIDGETS").small().weak());
                ui.add_space(4.0);

                for (i, widget) in cfg.widgets.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut widget.enabled, "").changed();
                        let label = format!("{}  ({})", widget.id, widget.kind.as_str());
                        if ui.selectable_label(selected == i, label).clicked() {
                            new_selected = i;
                        }
                    });
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "A clock or a timer slots in here as a new widget kind — same \
                         placement controls, same file.",
                    )
                    .small()
                    .weak(),
                );

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(hotkey_legend()).small().weak());
                    ui.label(egui::RichText::new("HOTKEYS").small().weak());
                });
            });

        self.selected = new_selected;
        if changed {
            self.dirty = true;
        }
    }

    /// The live preview, as a fixed right-hand panel.
    ///
    /// Deliberately a panel rather than a floating window: the compositor decides
    /// this window's size — Hyprland tiles it, ignoring the requested geometry —
    /// so anything free-floating ends up on top of the controls at some sizes.
    fn preview(&mut self, ui: &mut egui::Ui) {
        let widget = self.cfg.widgets.get(self.selected).cloned();

        egui::Panel::right(egui::Id::new("foghud_preview"))
            .exact_size(PREVIEW_W)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("PREVIEW").small().weak());
                ui.add_space(6.0);

                let Some(widget) = widget else {
                    return;
                };
                let side = (ui.available_width().min(PREVIEW_PX as f32)).max(32.0);
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());

                // A mid-grey backdrop: a black outline on a light theme, or a dark
                // crosshair on a dark one, would otherwise be invisible.
                ui.painter()
                    .rect_filled(rect, 4.0, egui::Color32::from_gray(58));

                let image = preview_image(&widget);
                let texture =
                    ui.ctx()
                        .load_texture("foghud_preview", image, egui::TextureOptions::NEAREST);
                egui::Image::new(&texture).paint_at(ui, rect);

                ui.add_space(6.0);
                ui.label(egui::RichText::new("actual size, centred").small().weak());
            });
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        let monitors = self.monitors.clone();
        let selected = self.selected;
        let mut changed = false;
        let mut add_crosshair = false;
        let cfg = &mut self.cfg;

        egui::CentralPanel::default().show(ui, |ui| {
            if cfg.widgets.is_empty() {
                ui.add_space(20.0);
                ui.label("No widgets configured.");
                if ui.button("Add a crosshair").clicked() {
                    add_crosshair = true;
                }
                return;
            }

            if let Some(widget) = cfg.widgets.get_mut(selected) {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    changed |= placement_section(ui, widget, &monitors);
                    ui.add_space(16.0);
                    // An exhaustive match rather than `if let`: a new widget kind
                    // then fails to compile here instead of quietly rendering a
                    // settings pane with nothing kind-specific in it.
                    match &mut widget.kind {
                        Kind::Crosshair(c) => changed |= crosshair_section(ui, c),
                    }
                });
            }
        });

        if add_crosshair {
            self.cfg.ensure_crosshair();
            self.dirty = true;
        }
        if changed {
            self.dirty = true;
        }
    }
}

enum Action {
    Start,
    Stop,
    Restart,
}

/// The panel reports failures to the terminal it was launched from rather than
/// interrupting; none of them leave it in a state you can't retry from.
fn drop_err(result: Result<()>) {
    if let Err(err) = result {
        eprintln!("foghud: {err:#}");
    }
}

fn hotkey_legend() -> String {
    HOTKEYS
        .iter()
        .map(|(key, what)| format!("{key}   {what}\n"))
        .collect()
}

// ------------------------------------------------------------------ controls --

/// Position, monitor and opacity — the settings every widget kind has.
fn placement_section(ui: &mut egui::Ui, widget: &mut Widget, monitors: &[String]) -> bool {
    let mut changed = false;

    ui.label(egui::RichText::new("PLACEMENT").small().weak());
    ui.add_space(4.0);

    egui::Grid::new("placement")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Anchor");
            egui::ComboBox::from_id_salt("anchor")
                .selected_text(widget.anchor.as_str())
                .show_ui(ui, |ui| {
                    for a in Anchor::ALL {
                        changed |= ui
                            .selectable_value(&mut widget.anchor, a, a.as_str())
                            .changed();
                    }
                });
            ui.end_row();

            ui.label("Offset")
                .on_hover_text("Pixels away from the anchor point.");
            ui.horizontal(|ui| {
                ui.label("x");
                changed |= ui
                    .add(egui::DragValue::new(&mut widget.offset_x).speed(1.0))
                    .changed();
                ui.label("y");
                changed |= ui
                    .add(egui::DragValue::new(&mut widget.offset_y).speed(1.0))
                    .changed();
                if ui.button("Reset").clicked() {
                    widget.offset_x = 0.0;
                    widget.offset_y = 0.0;
                    changed = true;
                }
            });
            ui.end_row();

            ui.label("Monitor");
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("monitor")
                    .selected_text(widget.monitor.clone())
                    .show_ui(ui, |ui| {
                        for option in ["all", "primary"] {
                            changed |= ui
                                .selectable_value(&mut widget.monitor, option.to_string(), option)
                                .changed();
                        }
                        for name in monitors {
                            changed |= ui
                                .selectable_value(&mut widget.monitor, name.clone(), name)
                                .changed();
                        }
                    });
                // Still editable by hand: the dropdown can only offer monitors
                // that are plugged in right now.
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut widget.monitor)
                            .id_salt("monitor_text")
                            .desired_width(90.0)
                            .hint_text("DP-3"),
                    )
                    .changed();
            });
            ui.end_row();

            ui.label("Opacity");
            changed |= ui
                .add(
                    egui::Slider::new(&mut widget.opacity, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{}%", (v * 100.0).round()))
                        .custom_parser(|s| {
                            s.trim_end_matches('%')
                                .parse::<f64>()
                                .ok()
                                .map(|v| v / 100.0)
                        }),
                )
                .changed();
            ui.end_row();
        });

    changed
}

fn crosshair_section(ui: &mut egui::Ui, c: &mut Crosshair) -> bool {
    let mut changed = false;

    ui.label(egui::RichText::new("CROSSHAIR").small().weak());
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Type");
        for style in Style::ALL {
            changed |= ui
                .selectable_value(&mut c.style, style, style.as_str())
                .changed();
        }
    });
    ui.add_space(8.0);

    egui::Grid::new("crosshair")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Colour");
            changed |= color_row(ui, "color", &mut c.color);
            ui.end_row();

            ui.label("Outline colour");
            changed |= color_row(ui, "outline_color", &mut c.outline_color);
            ui.end_row();

            ui.label("Size");
            changed |= ui.add(egui::Slider::new(&mut c.size, 0.0..=40.0)).changed();
            ui.end_row();

            ui.label("Thickness");
            changed |= ui
                .add(egui::Slider::new(&mut c.thickness, 1.0..=12.0))
                .changed();
            ui.end_row();

            ui.label("Gap");
            changed |= ui.add(egui::Slider::new(&mut c.gap, 0.0..=30.0)).changed();
            ui.end_row();

            ui.label("Centre dot");
            changed |= ui
                .add(egui::Slider::new(&mut c.dot, 0.0..=12.0))
                .on_hover_text("0 for none. Always drawn for the 'dot' type.")
                .changed();
            ui.end_row();

            ui.label("Outline width");
            changed |= ui
                .add(egui::Slider::new(&mut c.outline, 0.0..=6.0))
                .changed();
            ui.end_row();
        });

    changed
}

/// A colour swatch and the hex field next to it, kept in step.
///
/// The text stays authoritative — the config stores a string, and names like
/// `cyan` are valid values a picker can't represent. An unparseable string is
/// left alone rather than corrected, so a half-typed `#00f` isn't destroyed
/// mid-keystroke.
fn color_row(ui: &mut egui::Ui, id: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if let Some(rgba) = parse_color(value) {
            let mut color = egui::Color32::from_rgb(rgba[0], rgba[1], rgba[2]);
            if ui.color_edit_button_srgba(&mut color).changed() {
                *value = to_hex([color.r(), color.g(), color.b(), 255]);
                changed = true;
            }
        } else {
            ui.colored_label(egui::Color32::from_rgb(230, 90, 90), "?")
                .on_hover_text("Not a colour foghud understands.");
        }
        changed |= ui
            .add(
                egui::TextEdit::singleline(value)
                    .id_salt(id)
                    .desired_width(110.0),
            )
            .changed();
    });
    changed
}

// ------------------------------------------------------------------ preview --

/// Rendered by the real rasteriser.
///
/// Going through `render::draw` rather than reimplementing the shapes in egui is
/// the whole point: what the panel shows is what the overlay will draw, including
/// pixel snapping and outline behaviour, because it *is* the same code.
fn preview_image(widget: &Widget) -> egui::ColorImage {
    // Centred and unconditionally visible: this previews appearance, not
    // placement, so the widget's own anchor and monitor are deliberately ignored.
    let shown = Widget {
        enabled: true,
        anchor: Anchor::Center,
        offset_x: 0.0,
        offset_y: 0.0,
        monitor: "all".into(),
        ..widget.clone()
    };

    let cfg = Config {
        widgets: vec![shown],
        ..Default::default()
    };
    let screen = render::Screen {
        name: "preview",
        index: 0,
        width: PREVIEW_PX,
        height: PREVIEW_PX,
    };
    let pixmap = render::draw(&cfg, &screen);

    // tiny-skia stores premultiplied pixels; egui wants straight alpha here.
    let mut rgba = Vec::with_capacity((PREVIEW_PX * PREVIEW_PX * 4) as usize);
    for pixel in pixmap.pixels() {
        let c = pixel.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    egui::ColorImage::from_rgba_unmultiplied([PREVIEW_PX as usize, PREVIEW_PX as usize], &rgba)
}
