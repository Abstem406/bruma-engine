# bruma 🌫️

> **Free animated wallpaper engine for Wayland** — WGSL shaders,
> efficient, with an open format.

`bruma` is a free engine for animated wallpapers on Linux/Wayland: WGSL
shaders at 60 fps with minimal CPU cost, an open package format
(`.wallpaper`) and tools so anyone can create and share their
wallpapers.

## Project status

Phases 0–5 complete: animated WGSL shaders with hot-reload, image and
solid color, the `.wallpaper` package format (secure installation with
`validate`/`install`), and a full desktop experience — multi-monitor
with a different wallpaper per screen, automatic pause on
fullscreen/lock/battery, per-output HiDPI scale and hot config reload.
In progress: creator tools (Phase 6) — see [PLAN.md](PLAN.md).

```bash
bruma run                        # no flags: uses the persistent config
bruma run 0x3B4252               # hex color of choice (all screens)
bruma run --gpu                  # WGSL triangle (pipeline test)
bruma run --image photo.png      # fullscreen image
bruma run --shader hello.wgsl    # animated shader with hot-reload
bruma run --shader hello.wgsl --fps 60 --param=0.5  # pace and parameters
bruma run --package onda-bruma-demo --param=intensidad=0.9  # installed package
```

## Full desktop (Phase 5)

One surface per output (multi-monitor with a shared GPU), automatic
pause on fullscreen (that screen only), locked session and battery
(global, via logind/UPower). Persistent per-screen config:

```bash
bruma config init                # creates ~/.config/bruma/config.json
bruma config show                # what will run and where
bruma service install            # starts with the session (systemd --user)
bruma service remove             # removes it
```

Example `config.json` — each screen its own thing, different params of
the same package (the animation stays synchronized: a single clock):

```json
{
  "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
  "outputs": {
    "eDP-1":    { "package": "onda-bruma-demo", "params": { "intensidad": 0.15 } },
    "HDMI-A-1": { "color": "#1d2021" }
  },
  "fps": 30
}
```

## Architecture

```
crates/
├── bruma-core/            # base types, no external dependencies
├── bruma-package/         # .wallpaper format (zip) — Phase 4
├── bruma-runtime/         # runtime contract (time, mouse...) — Phase 3
├── bruma-renderer/        # rendering contract — Phase 2
├── bruma-platform/        # Wayland wlr-layer-shell — Phase 1
└── bruma/                 # CLI: run, install, new, validate
```

Golden rule: `core`, `package` and `runtime` will **never** depend on
wgpu, Wayland or Steam (see [DECISIONS.md](DECISIONS.md)).

## Roadmap

| Phase | What | Demo |
|---|---|---|
| 0 | workspace, CI, licenses, docs | ✅ this structure |
| 1 | Wayland background window (niri first) | ✅ solid color behind everything |
| 2 | wgpu + WGSL | ✅ fullscreen image |
| 3 | runtime contract + hot-reload | ✅ animated shader edited live |
| 4 | `.wallpaper` format | ✅ `pack` → `validate` → `install` → `run --package` |
| 5 | multi-monitor, pauses | ✅ wallpaper per screen, HiDPI, hotplug |
| 6 | creator templates | a stranger creates and shares |
| 7 | web gallery (WASM/WebGPU) | live previews in the browser |

## Building

```bash
cargo build          # build everything
cargo test           # run the tests
cargo run -p bruma   # try the CLI
```

## `.wallpaper` packages

A wallpaper ships as a zip with `wallpaper.json`, a preview and its
shaders:

```bash
bruma pack my-wallpaper/ -o my-wallpaper.wallpaper   # pack a directory
bruma validate my-wallpaper.wallpaper                # validate (safe and correct?)
bruma install my-wallpaper.wallpaper                 # install into ~/.local/share/bruma
bruma list                                           # see what's installed
bruma run --package my-wallpaper                     # use it as the wallpaper
bruma run --package my-wallpaper --param=speed=0.8
```

The manifest declares named parameters that the CLI validates and
resolves:

```json
{
  "format": 1,
  "type": "shader",
  "title": "My wallpaper",
  "version": "0.1.0",
  "entry": "shader.wgsl",
  "preview": "preview.png",
  "params": [{ "name": "speed", "label": "Speed", "default": 0.5 }],
  "fps": 30
}
```

Validation rejects malicious packages: path traversal routes, symlinks
inside the zip and decompression bombs.

## License

Dual licensed MIT / Apache-2.0, like the Rust ecosystem.
