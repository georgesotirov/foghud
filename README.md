# foghud

Overlay toolkit for Dead by Daylight, on Linux and Windows. A crosshair for now,
with room for more.

Everything stays **outside the game process** — no memory reading, no injection,
no render hooking, no packet inspection, no input automation. It's an ordinary
desktop overlay that has no idea the game is running.

## Using it

```bash
foghud
```

Opens the control panel, and starts the overlay if it isn't already up. Sliders,
colour pickers and a live preview rendered by the same rasteriser that draws the
real thing.

### Hotkeys

While the overlay is running:

| Key | Steps |
|-----|-------|
| F1  | type — cross, tcross, circle, dot |
| F2  | size |
| F3  | colour |
| F4  | opacity |

Each press shows what changed. These are grabbed globally, so they won't reach
other applications while the overlay is up; turn them off with
`foghud hotkeys false` or the checkbox in the panel.

### Command line

The panel and the CLI are equals — both just write the settings file, and a
running overlay picks up either within 150ms.

```bash
foghud start / stop / restart / toggle / status
foghud size 14
foghud color cyan            # #rrggbb, #aarrggbb, or a name
foghud style tcross          # cross, tcross, circle, dot
foghud opacity 0.7
foghud thickness 2
foghud gap 4
foghud dot 3                 # centre dot radius, 0 for none
foghud outline 1
foghud anchor topLeft        # where the offset is measured from
foghud offset 120 120        # pixels from the anchor
foghud monitor DP-3          # all, primary, or a display name
foghud hotkeys false
foghud config path / show / reset
```

Position is an **anchor plus an offset** rather than absolute coordinates, so a
widget stays where you put it when the resolution changes.

## Requirements

- **Linux:** a Wayland compositor supporting `wlr-layer-shell`. Hotkeys go
  through the compositor and are currently wired for Hyprland.
- **Windows:** nothing extra. *Untested — it builds and passes CI, but has never
  been run.*

## Settings

`~/.config/foghud/config.json`, a list of widgets. Hand-editable; the overlay
reloads on change. Older flat config files are migrated automatically.

## Building

```bash
cargo build --release
```

## Licence

MIT.
