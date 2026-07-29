//! Wayland backend, via `wlr-layer-shell`.
//!
//! Wayland gives ordinary clients no way to sit above other windows or to grab
//! keys globally — both are refused by design. `wlr-layer-shell` is the
//! sanctioned escape hatch for the surface: a layer surface on the `Overlay`
//! layer draws above everything, including fullscreen windows. Click-through is
//! an empty input region.
//!
//! Hotkeys have no such escape hatch, so they go through the compositor. On
//! Hyprland that means `hyprctl`, which binds F1-F4 to `foghud crosshair cycle`
//! while the overlay is up and releases them when it exits.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use calloop::{
    EventLoop,
    timer::{TimeoutAction, Timer},
};
use calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_registry,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

use crate::config::Config;
use crate::render;

static TERMINATE: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    TERMINATE.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    // A plain flag flipped from the handler; the event loop notices it on its
    // next tick and unwinds normally, so hotkeys still get released.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
    }
}

// ------------------------------------------------------------------ hotkeys --

const HOTKEYS: [(&str, &str); 4] = [
    ("F1", "style"),
    ("F2", "size"),
    ("F3", "opacity"),
    ("F4", "color"),
];

fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

fn hyprctl(lua: &str) {
    let _ = std::process::Command::new("hyprctl")
        .arg("eval")
        .arg(lua)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

fn bind_hotkeys(cfg: &Config) {
    if !cfg.hotkeys || !on_hyprland() {
        return;
    }
    // Hyprland stacks duplicate binds rather than replacing them, and a doubled
    // bind would advance a cycle twice per press. Always clear first.
    unbind_hotkeys();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "foghud".into());
    let lua: String = HOTKEYS
        .iter()
        .map(|(key, what)| {
            format!(r#"hl.bind("{key}", hl.dsp.exec_cmd("{exe} crosshair cycle {what}")) "#)
        })
        .collect();
    hyprctl(&lua);
}

fn unbind_hotkeys() {
    if !on_hyprland() {
        return;
    }
    let lua: String = HOTKEYS
        .iter()
        .map(|(key, _)| format!(r#"hl.unbind("{key}") "#))
        .collect();
    hyprctl(&lua);
}

// -------------------------------------------------------------------- state --

struct Panel {
    layer: LayerSurface,
    output: wl_output::WlOutput,
    width: u32,
    height: u32,
    configured: bool,
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,

    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    panels: Vec<Panel>,
    exit: bool,
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path().ok()?)
        .ok()?
        .modified()
        .ok()
}

impl App {
    /// True when this output should carry a crosshair.
    fn wants_output(&self, output: &wl_output::WlOutput, index: usize) -> bool {
        match self.cfg.monitor.as_str() {
            "all" => true,
            "primary" => index == 0,
            name => self
                .output_state
                .info(output)
                .and_then(|i| i.name)
                .is_some_and(|n| n.eq_ignore_ascii_case(name)),
        }
    }

    fn rebuild_panels(&mut self, qh: &QueueHandle<Self>) {
        self.panels.clear();
        let outputs: Vec<_> = self.output_state.outputs().collect();
        for (i, output) in outputs.into_iter().enumerate() {
            if !self.wants_output(&output, i) {
                continue;
            }
            let surface = self.compositor.create_surface(qh);

            // An empty input region is what makes every click fall through to
            // the game underneath.
            if let Ok(region) = Region::new(&self.compositor) {
                surface.set_input_region(Some(region.wl_region()));
            }

            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Overlay,
                Some("foghud"),
                Some(&output),
            );
            layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
            // -1 keeps panels and bars from reserving space against us.
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();

            self.panels.push(Panel {
                layer,
                output,
                width: 0,
                height: 0,
                configured: false,
            });
        }
    }

    fn draw(&mut self, index: usize) {
        let Some(panel) = self.panels.get(index) else {
            return;
        };
        if !panel.configured || panel.width == 0 || panel.height == 0 {
            return;
        }
        let (w, h) = (panel.width, panel.height);
        let stride = w as i32 * 4;

        let pixmap = render::draw(&self.cfg, w, h);
        let bgra = render::to_bgra(&pixmap);

        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        canvas[..bgra.len()].copy_from_slice(&bgra);

        let panel = &self.panels[index];
        let surface = panel.layer.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        if buffer.attach_to(surface).is_ok() {
            surface.commit();
        }
    }

    fn draw_all(&mut self) {
        for i in 0..self.panels.len() {
            self.draw(i);
        }
    }

    /// Picks up config changes written by the CLI.
    fn poll_config(&mut self, qh: &QueueHandle<Self>) {
        let mtime = config_mtime();
        if mtime == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = mtime;

        let old_monitor = self.cfg.monitor.clone();
        let old_hotkeys = self.cfg.hotkeys;
        self.cfg = Config::load();

        if self.cfg.monitor != old_monitor {
            self.rebuild_panels(qh);
            return; // configure events will draw
        }
        if self.cfg.hotkeys != old_hotkeys {
            if self.cfg.hotkeys {
                bind_hotkeys(&self.cfg);
            } else {
                unbind_hotkeys();
            }
        }
        self.draw_all();
    }
}

// --------------------------------------------------------------------- run ---

pub fn run() -> Result<()> {
    crate::daemon::write_pid()?;
    install_signal_handlers();

    let result = run_inner();

    unbind_hotkeys();
    crate::daemon::clear_pid();
    result
}

fn run_inner() -> Result<()> {
    let conn = Connection::connect_to_env()
        .context("cannot reach a Wayland compositor (is WAYLAND_DISPLAY set?)")?;
    let (globals, event_queue) = registry_queue_init(&conn)?;
    let qh: QueueHandle<App> = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor unavailable")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor does not support wlr-layer-shell, which the overlay needs")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm unavailable")?;
    let pool = SlotPool::new(256 * 256 * 4, &shm).context("creating a buffer pool")?;

    let cfg = Config::load();
    bind_hotkeys(&cfg);

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        cfg,
        cfg_mtime: config_mtime(),
        panels: Vec::new(),
        exit: false,
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new()?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| anyhow::anyhow!("attaching the Wayland source: {e}"))?;

    // Outputs only exist after a roundtrip, so panels are built on the first tick.
    let qh_timer = qh.clone();
    let mut built = false;
    event_loop
        .handle()
        .insert_source(Timer::immediate(), move |_, _, app: &mut App| {
            if !built {
                app.rebuild_panels(&qh_timer);
                built = true;
            } else {
                app.poll_config(&qh_timer);
            }
            TimeoutAction::ToDuration(Duration::from_millis(150))
        })
        .map_err(|e| anyhow::anyhow!("adding the config poll timer: {e}"))?;

    while !app.exit {
        if TERMINATE.load(Ordering::SeqCst) {
            break;
        }
        event_loop.dispatch(Duration::from_millis(200), &mut app)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- handlers ---

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.rebuild_panels(qh);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, qh: &QueueHandle<Self>, o: wl_output::WlOutput) {
        self.panels.retain(|p| p.output != o);
        self.rebuild_panels(qh);
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.panels.retain(|p| &p.layer != layer);
        if self.panels.is_empty() {
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.panels.iter().position(|p| &p.layer == layer) else {
            return;
        };
        let (w, h) = configure.new_size;
        if w == 0 || h == 0 {
            return;
        }
        let panel = &mut self.panels[index];
        panel.width = w;
        panel.height = h;
        panel.configured = true;
        self.draw(index);
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

// SCTK 0.21 replaced the per-protocol delegate macros with one blanket impl
// that routes every object's events through its user data, so this pair is all
// the wiring the handlers above need.
delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);
