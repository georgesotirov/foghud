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
  config.rs    settings, colour parsing, hotkey cycles     portable
  render.rs    crosshair + hint rasteriser (tiny-skia)     portable
  text.rs      glyph rasterising (fontdue, embedded font)  portable
  daemon.rs    start/stop/find the overlay process
  platform/
    wayland.rs wlr-layer-shell surface + hyprctl hotkeys
    windows.rs layered window + RegisterHotKey
```

Rules that hold this together:

- **The rasteriser is platform-independent and stays that way.** Both backends
  present the same BGRA buffer. Never put platform conditionals in `render.rs`
  or `text.rs`.
- **The config file is the entire IPC layer.** The CLI writes JSON; the running
  overlay polls mtime every 150ms and redraws. No socket, no protocol.
- **Config writes are in place, never rename-over.** A rename swaps the inode and
  breaks the overlay's file watch.
- **Anything drawable must be unit-testable.** Tests assert on actual pixels —
  see `render.rs` and `text.rs`. New drawing gets new pixel tests.
- **Crosshair commands live at the top level** (`foghud size 14`), not under a
  `crosshair` noun. Future features get their own noun (`foghud stats`).

## Environment traps on this machine

- **Hyprland's config is Lua**, so `hyprctl keyword` fails with "keyword can't
  work with non-legacy parsers". Use `hyprctl eval 'hl.bind(...)'`.
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

State plainly what was verified visually and what wasn't. The Windows backend in
particular compiles but **has never been run** — say so rather than implying it
works.
