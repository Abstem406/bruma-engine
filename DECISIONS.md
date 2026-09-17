# DECISIONS — bruma

> Design decision log, with the why. Order: fundamental ones first. If
> a decision changes, a new entry is added at the end and the old one is
> marked superseded (history is never erased).

## D1 — Project, not product
An open personal project others will be able to use; not a product with
users to serve. This frees us from support promises and release cadence.
**Consequence:** no announcements until there are maintainable releases.

## D2 — Native Rust first, web later
The implementation is native (Rust). The web experiment (HTML/CSS/JS
widgets) is **deferred**: webviews on Linux (WebKitGTK/CEF) are heavy for
24/7 and dilute the performance advantage, which is the reason the project
exists. **Cheap decisions taken today to avoid rewrites:** (a)
`type: web` reserved in the manifest, (b) core separated from the
renderer, (c) runtime API as a trait, (d) WGSL already runs in the
browser via WebGPU.

## D3 — wgpu + WGSL, no OpenGL
Render with **wgpu** (Vulkan/Metal/DX12/GL fallback) and shaders in
**WGSL** exclusively. Reasons: WGSL is identical on desktop and browser
(the web gallery reuses the shaders as is), wgpu is the Rust ecosystem
standard and avoids legacy OpenGL. Note: the original Wallpaper Engine
uses DirectX 10/11 (not OpenGL, which is only its fallback);
linux-wallpaperengine emulates with OpenGL, but this project does NOT
emulate — it is a reimagining.

## D4 — Wayland only; niri first
v1 is Wayland only via `wlr-layer-shell` with **smithay-client-toolkit**
(winit does not support layer-shell; swww uses sctk for that reason).
**niri is the reference test bench** (the author's environment, with
DankMaterialShell); Hyprland/sway/KWin best-effort. **GNOME/Mutter is a
v1 non-goal** (no layer-shell) — it will be documented as a requirement
to avoid issues.

## D5 — Open .wallpaper format, unbranded
The package is called `.wallpaper` (zip with `wallpaper.json`,
`preview.png`, shaders, assets) — a generic ecosystem name, like
`.lively`. The brand name (bruma) stays with the engine. Precedent:
Lively proved community content does not need Steam Workshop
(drag-and-drop, forums).

## D6 — Core dependency boundary
`bruma-core`, `bruma-package` and `bruma-runtime` will **never** depend
on wgpu, Wayland or Steam. Reasons: (1) compile to WASM for the gallery,
(2) test without GPU or compositor, (3) keep the Steam SDK out of the
FOSS if a Steam version ever exists (`steamworks-rs` in an optional
crate). In Phase 0 the `renderer` and `platform` crates are also born
without dependencies (empty contracts); their heavy implementations go
in separate crates.

## D7 — GitHub first, Steam maybe later
Initial distribution is GitHub only. Steam (~$100/app fee, proprietary
SDK) is a possibly future accelerator, not a goal. If it arrives: same
`.wallpaper` format on both channels to avoid splitting the community.

## D8 — Workflow model: AI-implements / human-reviews
The author does not review line by line: they review demos. Rules: one
task = one demo, small commits, never accept code that does not run,
keep captures/logs, audit dependencies. Session details live in
[ai-development-log.md](ai-development-log.md).

## D9 — Name: bruma
- Repo/project: **bruma-engine** (free on crates.io and GitHub).
- Binary/CLI: **bruma** (`bruma run`, `bruma install`, `bruma new`).
- Crates: `bruma-core`, `bruma-package`, `bruma-runtime`,
  `bruma-renderer(-wgpu)`, `bruma-platform`.
- Format: `.wallpaper` (unbranded, see D5).
- Tagline: "Free animated wallpaper engine for Wayland".
- Evokes: mist/atmosphere — fits ambient shader wallpapers.

## D10 — Video and audio out of v1
Video (mpv/GStreamer) and audio capture (PipeWire) drag in huge
dependencies and different use cases. Reserved in the manifest
(`type: video`), future implementation. Audio as shader input: optional
and off by default (privacy) when it arrives.

## D11 — Desktop notifications via D-Bus, best-effort
- What: user notices (shader rejected/recovered on hot-reload) go over
  `org.freedesktop.Notifications` (mako, dunst, quickshell/DMS...) with
  the `dbus` 0.9 crate (libdbus dlopen: no bindgen, no build dep).
- Policy: **strict best-effort**. Without a session bus, the channel
  degrades to no-op; sends have a 300 ms timeout; no notification error
  may affect the render or the process. A wallpaper does not fail
  because of its notice channel.
- Architecture: the renderer emits typed events (`ReloadEvent`:
  Applied/Rejected/Recovered with dedup of identical autosaves); the CLI
  decides what to do with them (today: notify with a 2 s debounce). That
  way the graphics layer knows nothing about D-Bus and the future UI
  (Phase 6) can consume the same events.
- "Normal" urgency always: bruma notices are never critical.

## D12 — Engine pauses: per-output fullscreen, global lock and battery
- What: the engine stops repainting when the wallpaper is not visible or
  is not worth the GPU/battery. Three sources, two scopes:
  - **Per output**: fullscreen window over that output (the
    `wlr-foreign-toplevel-management` protocol). `maximized` does NOT
    pause: on several compositors it does not cover the wallpaper and it
    would pause something visible.
  - **Global**: session locked (logind: `Lock`/`Unlock` signals +
    `LockedHint` as a second source) and on battery (UPower
    `DisplayDevice.State == 2`). They pause ALL outputs.
- Policy: **strict best-effort** (same spirit as D11). Without a system
  bus or without logind/UPower, that source degrades to "never pauses";
  the engine does not change. The runtime is NEVER paused: global time
  goes on and on unpause the animation resumes without a jump.
- Key discovery (verified on the system, not from memory): logind
  ESCAPES IDs in object paths (session "4" →
  `/org/freedesktop/login1/session/_34`) and signals are emitted on the
  escaped path. Also `GetSessionByPID` is useless for this: Wayland
  compositors run as user services, outside logind's session scope.
  Correct graphical session: `ListSessions` + `Type="wayland"`.
- Threadless signal draining: `SyncConnection::process(Duration::ZERO)`
  capped at 32 iterations per frame (bursts absorbed, spinning
  impossible); the watcher lives on the frame loop's thread.
- Verification by CPU (/proc ticks), not captures: fullscreen and
  windows change the image; the process's CPU is the clean signal.
  35 ticks/8s → 1-2 ticks/8s when paused; exact resume.

## D13 — Language: everything documented in English
- What: all code and public documentation is written in English —
  doc-comments, inline comments, error messages, CLI text, README, PLAN
  and DECISIONS. The AI development log (`ai-development-log.md`) stays
  in Spanish: it is the author's working journal, not a public surface.
- Why: D1 makes this an open project others will use; English maximizes
  who can read, file issues and contribute. Error messages are an
  observable contract; mixed languages in an open codebase age badly.
- Consequence: manifest `param` names and other user-facing identifiers
  in packages may be any language (they are content, not documentation);
  the engine's own surfaces stay English-only.
