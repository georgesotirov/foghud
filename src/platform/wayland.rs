//! Wayland backend, via `wlr-layer-shell`.
//!
//! Wayland gives ordinary clients no way to sit above other windows or to grab
//! keys globally — both are refused by design. `wlr-layer-shell` is the
//! sanctioned escape hatch for the surface: a layer surface on the `Overlay`
//! layer draws above everything, including fullscreen windows. Click-through is
//! an empty input region.
//!
//! Hotkeys have no such escape hatch, so they go through the compositor. On
//! Hyprland that means `hyprctl`, which binds F1-F4 to `foghud cycle ...` while
//! the overlay is up and releases them when it exits.

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

use crate::config::{Config, HOTKEYS};
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

fn on_hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

fn hyprctl(lua: &str) {
    let output = std::process::Command::new("hyprctl")
        .arg("eval")
        .arg(lua)
        .output();
    // `eval` reports only whether the Lua parsed — a bind whose *command* is
    // broken still answers "ok", because that command doesn't run until the key
    // is pressed, in a child process nothing here can see. So this catches very
    // little; the tests below are the real guard. Surfaced anyway because
    // `foghud run` in a terminal is how the hotkeys get debugged.
    if let Ok(out) = output
        && !out.status.success()
    {
        eprintln!(
            "foghud: hyprctl eval failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
}

/// Wraps a string in single quotes for the shell, escaping embedded quotes.
///
/// This exists because the overlay's own path goes into the bind command, and
/// that path is entirely outside our control — it's wherever the user cloned the
/// repo. An unquoted path containing a space silently broke every hotkey: the
/// shell split it, `exec_cmd` got a nonexistent program, and the failure went to
/// a child process whose stderr nobody reads.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escapes a string for use inside a Lua double-quoted literal.
fn lua_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

/// The Lua handed to `hyprctl eval` to claim every hotkey.
///
/// Split out from [`bind_hotkeys`] purely so it can be tested: this is a string
/// crossing a process boundary into an interpreter, so the compiler checks
/// nothing about it and a typo here is invisible until a key is pressed.
fn bind_script(exe: &str) -> String {
    HOTKEYS
        .iter()
        .map(|(key, what)| {
            let cmd = lua_escape(&format!("{} cycle {what}", shell_quote(exe)));
            format!(r#"hl.bind("{key}", hl.dsp.exec_cmd("{cmd}")) "#)
        })
        .collect()
}

fn unbind_script() -> String {
    HOTKEYS
        .iter()
        .map(|(key, _)| format!(r#"hl.unbind("{key}") "#))
        .collect()
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
    hyprctl(&bind_script(&exe));
}

fn unbind_hotkeys() {
    if !on_hyprland() {
        return;
    }
    hyprctl(&unbind_script());
}

/// Runs one Lua dispatcher through `hyprctl eval`.
///
/// Note this is *not* `hyprctl dispatch <name> <args>`. On a Lua config that
/// shorthand expands to `hl.dispatch(<raw text>)` and fails to parse — the same
/// non-legacy-parser trap that rules out `hyprctl keyword`. Everything has to be
/// written as real Lua.
fn hypr_dispatch(lua: &str) {
    hyprctl(&format!("hl.dispatch({lua})"));
}

/// Whether the window belonging to `pid` is floating, or `None` if Hyprland
/// doesn't know about such a window yet.
fn is_floating(pid: u32) -> Option<bool> {
    let out = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    let clients: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    clients
        .as_array()?
        .iter()
        .find(|c| c.get("pid").and_then(serde_json::Value::as_u64) == Some(pid as u64))
        .and_then(|c| c.get("floating")?.as_bool())
}

/// Asks Hyprland to float, size and centre the control panel.
///
/// A tiling layout hands a settings window whatever slot happens to be spare —
/// in testing that was a 1082x201 strip, which is unusable. Wayland gives a
/// client no way to request floating, and `inner_size` is only a hint the
/// compositor may ignore, so this asks the compositor directly, exactly as the
/// hotkeys do.
///
/// Best effort and silent: a tiled panel is a nuisance, not a failure, and anyone
/// who wants it tiled can say so with their own window rule.
pub fn float_window(width: u32, height: u32) {
    if !on_hyprland() {
        return;
    }
    let pid = std::process::id();
    // On its own thread: the window doesn't exist for Hyprland to match until
    // it's mapped, and the first frame must not block on `hyprctl`.
    std::thread::spawn(move || {
        // Wait for the window to appear rather than guessing a delay.
        let mut floating = None;
        for _ in 0..20 {
            floating = is_floating(pid);
            if floating.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let Some(floating) = floating else {
            return;
        };

        let target = format!("window = \"pid:{pid}\"");
        // `float` is a *toggle*, so calling it on an already-floating window
        // would tile the thing we're trying to rescue.
        if !floating {
            hypr_dispatch(&format!("hl.dsp.window.float({{ {target} }})"));
        }
        hypr_dispatch(&format!(
            "hl.dsp.window.resize({{ {target}, x = {width}, y = {height}, exact = true }})"
        ));
        hypr_dispatch(&format!("hl.dsp.window.center({{ {target} }})"));
    });
}

/// Output names as the compositor reports them.
///
/// Asks `hyprctl` rather than opening a Wayland connection: the panel needs this
/// while the overlay may already hold the outputs, and shelling out avoids a
/// second client for a list of strings. Any failure is just an empty list.
pub fn monitor_names() -> Vec<String> {
    if !on_hyprland() {
        return Vec::new();
    }
    let Ok(out) = std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
    else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| m.get("name")?.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Set when Hyprland reports that it reloaded its config.
static CONFIG_RELOADED: AtomicBool = AtomicBool::new(false);

/// Watches Hyprland's event socket for config reloads.
///
/// Reloading rebuilds the bind table from the config file, which silently drops
/// anything added at runtime through `hyprctl eval` — including ours. Without
/// this, editing any part of the Hyprland config while the overlay is up leaves
/// F1-F4 dead until the overlay is restarted.
fn watch_for_reloads() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    let (Some(signature), Some(runtime)) = (
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
        std::env::var_os("XDG_RUNTIME_DIR"),
    ) else {
        return;
    };
    let socket = std::path::Path::new(&runtime)
        .join("hypr")
        .join(signature)
        .join(".socket2.sock");

    std::thread::spawn(move || {
        let Ok(stream) = UnixStream::connect(&socket) else {
            return;
        };
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if line.starts_with("configreloaded") {
                CONFIG_RELOADED.store(true, Ordering::SeqCst);
            }
        }
    });
}

// -------------------------------------------------------------------- state --

struct Panel {
    layer: LayerSurface,
    output: wl_output::WlOutput,
    /// Display name and position in the output list, passed to the renderer so
    /// it can apply each widget's own `monitor` setting.
    name: String,
    index: usize,
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
    /// Hint currently on screen, and when it should disappear.
    notice: String,
    notice_until: Option<std::time::Instant>,
    /// Whether the last frame we painted included the hint. Comparing against
    /// this is what makes an expiring hint trigger exactly one clearing redraw.
    notice_drawn: bool,
    last_notice_id: u64,
    needs_rebuild: bool,
    exit: bool,
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path().ok()?)
        .ok()?
        .modified()
        .ok()
}

impl App {
    /// Puts a surface on **every** output, unconditionally.
    ///
    /// Which widgets actually appear on which screen is the renderer's job now
    /// that each widget carries its own `monitor` setting — a surface that ends
    /// up with nothing on it just presents a transparent buffer. That costs one
    /// idle buffer per unused monitor and removes a whole class of bug: panels no
    /// longer need rebuilding when a monitor setting changes, so there's no
    /// window during which the overlay has torn down its surfaces.
    fn rebuild_panels(&mut self, qh: &QueueHandle<Self>) {
        self.needs_rebuild = false;
        self.panels.clear();
        let outputs: Vec<_> = self.output_state.outputs().collect();
        for (i, output) in outputs.into_iter().enumerate() {
            let name = self
                .output_state
                .info(&output)
                .and_then(|info| info.name)
                .unwrap_or_default();
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
                name,
                index: i,
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
        // Copied out so the pool can be borrowed mutably further down.
        let name = panel.name.clone();
        let output_index = panel.index;
        let stride = w as i32 * 4;

        let show_notice = self.notice_visible();
        self.notice_drawn = show_notice;
        let screen = render::Screen {
            name: &name,
            index: output_index,
            width: w,
            height: h,
        };
        let mut pixmap = render::draw(&self.cfg, &screen);
        if show_notice {
            render::draw_hint(&mut pixmap, &self.cfg, &screen, &self.notice);
        }
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

    fn notice_visible(&self) -> bool {
        self.notice_until
            .is_some_and(|until| std::time::Instant::now() < until)
    }

    /// A full key listing stays up long enough to read; a single changed value
    /// is a glance, so it goes away quickly.
    fn show_notice(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let secs = if text.contains('\n') { 4 } else { 2 };
        self.notice_until = Some(std::time::Instant::now() + Duration::from_secs(secs));
        self.notice = text;
    }

    /// Picks up config changes written by the CLI, and re-asserts the hotkeys if
    /// a Hyprland config reload dropped them.
    fn poll_config(&mut self, qh: &QueueHandle<Self>) {
        if CONFIG_RELOADED.swap(false, Ordering::SeqCst) {
            bind_hotkeys(&self.cfg);
        }

        if self.needs_rebuild {
            self.rebuild_panels(qh);
        }

        // A hint that has timed out needs one more frame to wipe it away.
        if self.notice_visible() != self.notice_drawn {
            self.draw_all();
        }

        let mtime = config_mtime();
        if mtime == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = mtime;

        let old_hotkeys = self.cfg.hotkeys;
        self.cfg = Config::load();

        if self.cfg.notice_id != self.last_notice_id {
            self.last_notice_id = self.cfg.notice_id;
            let text = self.cfg.notice.clone();
            self.show_notice(text);
        }

        // Note there is no rebuild on a monitor change any more: surfaces exist
        // on every output regardless, so moving a widget between monitors is
        // just the next redraw.
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

    let cfg_at_start = Config::load();
    let cfg = cfg_at_start.clone();
    bind_hotkeys(&cfg);
    watch_for_reloads();

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
        notice: String::new(),
        notice_until: None,
        notice_drawn: false,
        last_notice_id: cfg_at_start.notice_id,
        needs_rebuild: false,
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
        // A closed surface is not a reason to quit. Compositors close layer
        // surfaces for all sorts of transient reasons — a monitor going away, a
        // config reload — and a background overlay that exits on the first one
        // is one unplugged cable away from silently disappearing. Drop the panel
        // and let the next tick rebuild from whatever outputs exist now.
        self.panels.retain(|p| &p.layer != layer);
        self.needs_rebuild = true;
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

// ------------------------------------------------------------------- tests ---

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this whole quoting layer exists for. The overlay lives at
    /// whatever path the user cloned it to; ours contains a space, and the
    /// unquoted form made `sh` try to run `/mnt/storage/Coding`.
    #[test]
    fn quoted_path_survives_the_shell() {
        let path = "/mnt/storage/Coding Projects/foghud/target/release/foghud";
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(path)))
            .output()
            .expect("sh should be available");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            path,
            "the shell must hand the path back in one piece"
        );
    }

    #[test]
    fn shell_quote_handles_an_embedded_quote() {
        let path = "/home/it's odd/foghud";
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", shell_quote(path)))
            .output()
            .expect("sh should be available");
        assert_eq!(String::from_utf8_lossy(&out.stdout), path);
    }

    #[test]
    fn bind_script_quotes_the_executable() {
        let lua = bind_script("/a b/foghud");
        assert!(
            lua.contains(r#"hl.bind("F1", hl.dsp.exec_cmd("'/a b/foghud' cycle style"))"#),
            "unexpected Lua: {lua}"
        );
        // The exact shape that was broken: path sitting bare against its args.
        assert!(
            !lua.contains("(\"/a b/foghud cycle"),
            "the executable path must never be passed unquoted: {lua}"
        );
    }

    #[test]
    fn bind_script_covers_every_hotkey() {
        let lua = bind_script("/opt/foghud");
        for (key, what) in HOTKEYS {
            assert!(
                lua.contains(&format!(r#"hl.bind("{key}""#)),
                "{key} missing"
            );
            assert!(lua.contains(&format!("cycle {what}")), "{what} missing");
        }
        assert_eq!(unbind_script().matches("hl.unbind").count(), HOTKEYS.len());
    }

    /// Guards the mapping itself, which is the part a user notices.
    #[test]
    fn hotkeys_are_type_size_color_opacity() {
        assert_eq!(
            HOTKEYS,
            [
                ("F1", "style"),
                ("F2", "size"),
                ("F3", "color"),
                ("F4", "opacity"),
            ]
        );
    }

    #[test]
    fn lua_escape_protects_quotes_and_backslashes() {
        assert_eq!(lua_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(lua_escape(r"a\b"), r"a\\b");
    }
}
