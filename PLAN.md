# PLAN — Free Wallpaper Engine for Linux ("bruma")

> Project's operating roadmap. Design decisions live in
> [DECISIONS.md](DECISIONS.md); the AI work log in
> [ai-development-log.md](ai-development-log.md).

## AI working principles

1. **One task = one runnable demo.** Never call a phase done without a
   capture, video or log proving it.
2. **Small, verifiable commits**: each one compiles and passes tests.
3. **Never accept code that can't be run.** When in doubt: reproduce it
   or drop it.
4. **Audit dependencies** whenever one is added (`cargo audit`, check
   maintenance and weight).
5. **Keep evidence** (captures/logs) next to the demo in every phase.

## Defining milestone

> "A stranger downloads a `.wallpaper` file, drags it, and sees an
> animated wallpaper." — the project is not done until this happens.

## Phases

### Phase 0 — Foundations (1-2 weeks) ✅
- Git repo, Cargo workspace with crates: `core`, `package`, `runtime`,
  `renderer`, `platform`, CLI `bruma`.
- MIT + Apache-2.0 licenses, CI (GitHub Actions), `.gitignore`.
- `PLAN.md`, `DECISIONS.md`, `ai-development-log.md`.
- Demo: `cargo run -p bruma` compiles and greets; core tests green.

### Phase 1 — Wayland background window (2-5 weeks) ✅
- `wlr-layer-shell` via `smithay-client-toolkit` (NOT winit).
- Reference test bench: **niri** (+ DankMaterialShell).
- No wgpu yet: solid color buffer.
- Demo: a color rectangle behind all windows.
  (`bruma run [color]`; evidence in `demos/fase1/`).
- Acceptance criterion: survives niri config reload ✅ (verified) and
  output disconnect/reconnect (pending verification with real
  hardware; surface re-configuration is already covered).

### Phase 2 — wgpu rendering (3-6 weeks) ✅
- Triangle → quad → texture → PNG/JPEG fullscreen image.
- Crates: `bruma-renderer` (contract) + `bruma-renderer-wgpu` (impl).
- Demo: static image as wallpaper ✅ (`bruma run --image IMG`;
  evidence in `demos/fase2/`).
- Criterion: stable FPS (n/a static), CPU nearly idle ✅ (measured:
  fully idle, renders only on configure), contained memory ✅
  (~137 MB with the Vulkan driver).

### Phase 3 — Runtime contract + hot-reload ✅ (2026-09-16)
- `WallpaperRuntime` trait: time, delta, resolution, mouse, flat
  parameters (`[f32; 4]` → `u_params0..3`; names arrive with the
  Phase 4 manifest). Optional audio: pending (off by default, to be
  evaluated with the first wallpaper that needs it).
- ✅ Uniform buffers (48 bytes), WGSL hot-reload (mtime + naga
  validation; broken shader = previous pipeline kept), configurable
  FPS cap (`--fps`, 30 by default).
- ✅ Demo verified on niri: pixel animation between captures, live
  hot-reload (wine → green → broken shader rejected → recovered),
  CPU 4% at 30 fps on 2560x1440, survives niri config reload.
- Extra (D11): rejected/recovered shader notifies via desktop
  notification over D-Bus (verified with quickshell/DMS in the demo;
  visible bubble in a capture and Notify messages captured on the
  bus).

### Phase 4 — .wallpaper format (2-4 weeks) — ✅
- ✅ ZIP with `wallpaper.json`, `preview.png`, shaders, assets (`zip`
  crate 8.6, minimal features; `serde` for the manifest with
  `deny_unknown_fields`).
- ✅ Schema: `format/type/title/entry/preview/permissions/min_engine` +
  package `version` (for the versioned install layout) and `params`
  with name/label/default (Phase 3's `u_params0..3` gain identity).
  `type: video|web` reserved, not implemented.
- ✅ Validation: anti path-traversal (`enclosed_name`), symlink
  rejection, size limits (50 MB package, 20 MB file, 96 MB
  decompressed), single entry and install with staging + atomic
  rename.
- ✅ Commands: `bruma validate`, `install`, `list`, `pack` and
  `run --package name[:version]` with `--param=name=value` resolved
  against the manifest (params take the default; CLI ones override;
  unknown name = error).
- ✅ Demo verified on niri: pack → validate → install → list → run
  (live animation by pixels, `--param=intensidad=0.9` changes the
  wallpaper, nonexistent name rejected with an error).
- ✅ Security demonstrated: malicious packages (path traversal
  `../../.bashrc`, symlink, 40×4 MB zip bomb) rejected in `validate`
  and `install` with a precise message; home intact.
- Minor deviation from the original schema: `version` field added (it
  is what orders the install directory `~/.local/share/bruma/
  wallpapers/NAME/VERSION`).

### Phase 5 — Full desktop (4-8 weeks) — ✅ (2026-09-17)
- ✅ Multi-monitor: ONE layer-shell surface per output (with the
  concrete output in `create_layer_surface`), created on hotplug
  (`new_output`), destroyed when the output goes away
  (`output_destroyed`/`closed`).
- ✅ Shared GPU (`GpuShared`, cloneable device/queue) + `SurfaceCtx` per
  output (swapchain format chosen per output); renderer factory
  injected from the CLI (the platform knows nothing about GPUs).
- ✅ Demo verified on niri: eDP-1 1920x1200 + HDMI-A-1 2560x1440 with
  the same animated shader (AMD 660M over Vulkan), pixel animation on
  both, survival to config reload and the DPMS cycle.
- ✅ Latent bug FIX (since Phase 3): after drawing, the loop waited for
  events indefinitely — the animation only advanced with compositor
  events (masked by desktop traffic). Now the deadline ALWAYS bounds
  the poll; CPU measured with a quiet desktop: ~5.3% of one core for
  two outputs at 30 fps (~2.6% per output).
- ✅ Per-output fullscreen pause (D12): optional bind of
  `wlr-foreign-toplevel-management`; a fullscreen window on an output
  freezes ONLY its wallpaper (the last buffer stays on screen in the
  compositor's hands; global time is not paused → no jump on return).
  Without the protocol, degrades to "never pause". Measured on niri:
  43→~4 CPU ticks/8-10s when pausing (per output), ~1 tick with both
  paused, no-jump resume. `maximized` does NOT pause (D12 policy).
- ✅ Global pause: session lock (logind `Lock`/`Unlock` + `LockedHint`
  backup) and battery (UPower `State==2`), over the system D-Bus,
  best-effort (no bus → never pauses). Graphical session located with
  `ListSessions`+`Type=wayland` (GetSessionByPID useless: compositors
  run as user services) and escaped paths (`_34`) used as is.
  Measured: 35→1-2 CPU ticks/8s with the machine on battery;
  lock/unlock verified with transitions in the log. Threadless signal
  draining: `process(ZERO)` capped per frame.
- ✅ Different wallpaper per screen + persistent config: strict JSON
  config (`~/.config/bruma/config.json`, deny_unknown_fields,
  validated params) with `default` + entries by output name. Extended
  factory (`FactoryRenderer`: renderer and/or color per output).
  Per-OUTPUT params resolved by name against the manifest and applied
  in each renderer (a single runtime → synchronized animation). CLI:
  `bruma config init|show`, bare `bruma run` loads the config.
- ✅ systemd user service (`bruma service install|remove`): unit
  generated with the real exe, `Restart=always` (verified: SIGTERM →
  revives in 2 s), starts with `graphical-session.target`.
- ✅ DPI/scale per output: `set_buffer_scale` with the scale announced
  by the compositor; buffers (SHM and wgpu) are drawn in physical
  pixels (logical × scale), sharp on HiDPI. Hot scale change
  (`scale_factor_changed`) verified on niri: eDP-1 1920x1200 scale
  1→2→1, continuous animation and quad covering the new buffer's 4
  corners (CPU 8 ticks/5s at both scales).
- ✅ Output hotplug verified on niri via IPC (`niri msg output X
  off/on`): surface destroyed on power-off ("remaining: 1") and a new
  surface with its config's source on reconnection; the fullscreen
  pause (D12) correctly follows the window migrating between outputs
  (CPU: 0 ticks with the only output fullscreen, 7 ticks on
  reconnection). SIGHUP after hotplug stable.
- Known limits (they do not block the milestone): fractional scale
  (`wp_fractional_scale` — niri rounds to an integer, the wallpaper is
  slightly over-scanned), real cursor position for `u_mouse`
  (reactive wallpapers will ask for it; to be evaluated with the first
  real case), config FPS only applied on restart.
- Verified compatibility: niri (reference), Hyprland, sway, KWin.
- GNOME/Mutter remains a v1 non-goal.

### Phase 6 — Creator tools (partially complete; see breakdown)
- ✅ Parameter UI (2026-09-18): `bruma params WALLPAPER [list | set NAME VALUE [--adopt] | reset [NAME]] [--output NAME]` — live tuning over an installed wallpaper: reads the manifest with the real parser (unknown names and out-of-range values rejected), persists overrides into `config.json` (per-output or default), SIGHUPs the running daemon and the change is visible instantly (verified on niri: `set intensity 0.9` → `[0.90, ...]` in the daemon log without restart; `--adopt` points a config section at the tuned package so CLI runs can be tuned too). No TUI by design (D8): a scriptable CLI over the config file, zero new dependencies (libc was already in the workspace).
- ✅ `u_clock`: real local time (h/m/s) in the uniform block — day/night
  tints and clock wallpapers.
- ✅ `bruma new <name> --template <t>`: scaffolding from zero to an
  installable package (manifest self-checked with the real parser,
  naga-validated templates).
- ✅ Templates: waves, fog (FBM + day/night), water (procedural),
  water-photo (image behind water), trail (feedback), parallax
  (mouse-reactive aurora).
- ✅ `textures` in the manifest: package assets exposed to the shader as
  GPU textures (binding slots 1..8, max 4).
- ✅ `feedback` permission: previous-frame ping-pong (trails,
  reaction-diffusion, simulations) with an internal blit pass.
- ✅ `mouse` permission wired: wl_pointer position feeds `u_mouse` while
  the cursor hovers the background (parallax works on an empty desktop).
- Pending: parameter UI generated from the manifest (CLI/TUI),
  cover/contain aspect handling for textures, MIME association for
  drag-and-drop install.
- Milestone (pending the UI): a person who does not code creates, packs
  and shares.

### Phase 7 — Public presence (3-6 weeks)
- Releases on GitHub Releases (binaries per target).
- Docs on GitHub Pages; static gallery with direct download.
- Live previews: the same WGSL runs in the browser via WASM/WebGPU
  (reuses `core`/`runtime` compiled to WASM).
- Deferred decision: Steam (with the SDK isolated from the core,
  `steamworks-rs`).

## v1 non-goals

- GNOME/Mutter (no wlr-layer-shell; to be re-evaluated with extensions
  or Mutter's future layer-shell support).
- Video (mpv/GStreamer) and audio (PipeWire): future phases, not v1.
- Wallpaper Engine asset compatibility (`.pkg`): the project is a
  reimagining with its own format, not an emulator.
- Desktop web widgets: future experiment behind the reserved
  `type: web`, not v1.

## Time expectations

- Visible prototype: weeks-months.
- Reliable personal tool: 6-18 months.
- Ecosystem with community: years.
