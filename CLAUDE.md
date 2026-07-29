# CLAUDE.md — working agreement for foghud

Instructions for Claude Code sessions in this repo. Read before changing
anything.

## What this is

An overlay toolkit for Dead by Daylight, Linux and Windows. v1 is the crosshair.
Owner: georgesotirov. Public, MIT.

## The line this project does not cross

Everything stays **outside the game process**. No memory reading, no injection,
no render hooking, no packet inspection, no input automation keyed off game
state. The crosshair is a desktop overlay that has no idea DBD is running; the
planned stats work reads files Steam already wrote to disk.

Do not implement, suggest, or accept anything that crosses this — it's the
project's whole premise, not a limitation to work around.

## Keep the notes current — this is the standing instruction

**At the end of any session that changes behaviour, update both:**

1. `session/` — dated working notes. Gitignored, so be candid. Record *why*
   decisions were made and any gotcha that cost real time. Append to the current
   file, or start a new dated one for a distinctly new phase of work.
2. `README.md` — only if user-facing behaviour changed (commands, flags,
   defaults, requirements).

The session notes are the reason a later session doesn't rediscover the same
traps. Treat writing them as part of the work, not paperwork after it.

Do not remove the "gotchas" section from the notes to make them tidier. That
section is the most valuable part.

## Architecture, and why

```
src/
  config.rs    widget list, colour parsing, hotkey table    portable
  render.rs    widget + hint rasteriser (tiny-skia)         portable
  text.rs      glyph rasterising (fontdue, embedded font)   portable
  gui.rs       control panel (eframe/egui)                  portable
  daemon.rs    start/stop/find the overlay process
  platform/
    wayland.rs wlr-layer-shell surface + hyprctl hotkeys
    windows.rs layered window + RegisterHotKey
```

Rules that hold this together:

- **The rasteriser is platform-independent and stays that way.** Both backends
  present the same BGRA buffer. Never put platform conditionals in `render.rs`
  or `text.rs`.
- **The config file is the entire IPC layer.** The CLI, the GUI and the hotkeys
  all just write JSON; the running overlay polls mtime every 150ms and redraws.
  No socket, no protocol. The GUI is *not* privileged — it holds no state the
  overlay needs, and it watches the file so a hotkey press updates it too.
- **Config writes are in place, never rename-over.** A rename swaps the inode and
  breaks the overlay's file watch.
- **Settings are a list of widgets.** Each carries its own `monitor`, `anchor`,
  offset and `opacity`; the crosshair is one `Kind`. A clock or timer is a new
  variant plus a draw arm, not a reshuffle. Match on `Kind` exhaustively rather
  than using `if let`, so a new variant fails to compile at every site that
  needs updating.
- **Monitor selection belongs to `render.rs`, not the backends.** Both backends
  create a surface on every output and pass a `Screen { name, index, w, h }`;
  the per-widget filter is applied while drawing. Don't reintroduce
  per-backend monitor logic — it was duplicated and untestable.
- **`config::HOTKEYS` is the only hotkey mapping**, and `config::label` the only
  place a value's wording lives. Both backends and the hint panel derive from
  them so they cannot drift. Remapping a key = editing that array.
- **Anything drawable must be unit-testable.** Tests assert on actual pixels —
  see `render.rs` and `text.rs`. New drawing gets new pixel tests.
- **Crosshair commands live at the top level** (`foghud size 14`), not under a
  `crosshair` noun. Future features get their own noun (`foghud clock`,
  `foghud stats`).
- **Keep `LegacyConfig`** until no pre-widget config files are plausibly in the
  wild. It's what stops an upgrade silently resetting a tuned crosshair.

## Environment traps on this machine

- **Hyprland's config is Lua**, so anything with a shorthand form fails against
  the non-legacy parser. Both `hyprctl keyword` *and* `hyprctl dispatch` are out:
  `hyprctl dispatch setfloating pid:1` becomes `hl.dispatch(setfloating pid:1)`
  and won't parse. Write real Lua through `hyprctl eval`:
  `hl.bind(...)`, `hl.dispatch(hl.dsp.window.float({ window = "pid:1" }))`.
- **Never interpolate a path into a bind command unquoted.** This repo lives
  under `Coding Projects` — a path *with a space*. Unquoted, the shell splits it
  and the hotkey silently does nothing. Use `shell_quote`; the tests in
  `wayland.rs` are the only guard, since the string crosses into an interpreter.
- **`hyprctl binds` will lie to you about Lua binds.** They show as
  `dispatcher: __lua` with an opaque numeric `arg` — the command string is *not*
  in that output. Grepping it for your command finds nothing either way.
- **`hyprctl eval` returning `ok` only means the Lua parsed.** A bind with a
  broken command still answers `ok`; the command fails later, in a child process,
  with nobody reading its stderr. **Verify hotkeys by pressing keys.**
- **`hyprctl eval` discards return values, but errors come back.** To inspect the
  Lua API, raise one:
  `hyprctl eval 'local t={} for k in pairs(hl.dsp) do t[#t+1]=k end error(table.concat(t,","))'`
- **`hl.dsp.window.float` is a toggle**, not a setter. Check `hyprctl clients -j`
  first or you'll tile the window you meant to float.
- **Hyprland stacks duplicate binds.** Always `hl.unbind` before `hl.bind`, or
  the action fires twice.
- **`hyprctl reload` wipes every runtime bind.** The overlay watches
  `$XDG_RUNTIME_DIR/hypr/$HIS/.socket2.sock` for `configreloaded` and
  re-asserts. Any new runtime bind needs the same treatment.
- **No rustup here** (Arch ships plain `rust`), so the Windows backend
  **cannot be compiled locally**. CI is the only check on it. Expect a
  round-trip per Windows change and batch them.
- `~/.local/bin/foghud` symlinks the **release** binary. `cargo build` alone
  won't update what the `foghud` command runs.
- **egui 0.35 differs from most examples.** `eframe::App` has
  `fn ui(&mut self, ui: &mut Ui, frame: &mut Frame)` — no `update`, no `&Context`
  parameter. `SidePanel`/`TopBottomPanel` are replaced by one `Panel` type whose
  `show` takes `&mut Ui`. Check the vendored source, not tutorials.
- **Test JSON containing a colour needs `r##"..."##`.** `"#` closes an `r#"..."#`
  raw string early.

## Before you commit

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

CI enforces all three on both platforms. Don't push past a red local run.

Commits are authored by georgesotirov alone — no `Co-Authored-By` trailers.

## Verify on screen, don't assume

This is a visual tool. A clean build proves nothing about what a user sees.
Changes to drawing get checked with a real screenshot:

```bash
foghud start && grim -g "1230,670 120x120" /tmp/shot.png
```

**A hotkey is only verified by a real key press.** Registering the bind, and even
running its command by hand, both pass while the hotkey is dead — that is exactly
how the F1-F4 breakage survived a "verified" note. Either press the key or say
you didn't.

State plainly what was verified visually and what wasn't. The Windows backend in
particular compiles but **has never been run** — say so rather than implying it
works.
