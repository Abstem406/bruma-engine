//! # bruma-runtime
//!
//! Runtime contract: the API a wallpaper can consume (time, delta,
//! resolution, mouse, parameters, optional audio).
//!
//! Status: **Phase 3**. This crate is **pure**: it defines the contract
//! shared by the native runtime (wgpu) and the future web runtime (WASM,
//! Phase 7), with no dependencies (D6).
//!
//! The native implementation lives in `bruma-renderer-wgpu`; the phase's
//! demo is an animated shader edited live without restarting.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

/// State of a frame: what the runtime hands the renderer on every
/// animation. Same concept as a standard shadertoy/LiveWallpaper uniform
/// block: everything a shader needs to know about the world.
///
/// Defined here (pure crate) so the future web gallery can reuse it as
/// is: they are just numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameState {
    /// Seconds since the wallpaper started.
    pub time: f32,
    /// Seconds since the previous frame (for stable physics).
    pub delta: f32,
    /// Width of the drawing area, in buffer pixels.
    pub width: u32,
    /// Height of the drawing area, in buffer pixels.
    pub height: u32,
    /// Cursor X position in the drawing area (pixels; `-1.0` = unknown).
    pub mouse_x: f32,
    /// Cursor Y position in the drawing area (pixels; `-1.0` = unknown).
    pub mouse_y: f32,
    /// Flat values of the first declared parameters (in WGSL:
    /// `u_params0..15`). The manifest and
    /// [`WallpaperRuntime::params`] carry the names; the GPU only sees
    /// numbers.
    pub params: [f32; 16],
    /// Real-time clock for shaders (`u_clock`): `[hours, minutes,
    /// seconds]` of the local day, supplied by the platform on every
    /// animated frame. `[0, 0, 0]` when no provider is installed.
    pub clock: [f32; 3],
}

impl Default for FrameState {
    fn default() -> Self {
        FrameState {
            time: 0.0,
            delta: 0.0,
            width: 0,
            height: 0,
            mouse_x: -1.0,
            mouse_y: -1.0,
            params: [0.0; 16],
            clock: [0.0; 3],
        }
    }
}

/// Parameters declared by a wallpaper and adjustable by the user.
///
/// Phase 3 defines them; the UI generated from the manifest arrives with
/// Phase 6 (creator tools). `value` is a continuous 0..=1 slider: enough
/// to drive shaders without a big type system.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamValue {
    pub name: String,
    pub value: f32,
}

/// Runtime contract of an animated wallpaper.
///
/// The engine calls [`Self::begin_frame`] before painting each frame and
/// [`Self::end_frame`] after presenting it. The runtime accumulates time
/// and decides the pace (FPS cap); the renderer only paints the frame the
/// runtime asks for.
///
/// Implementations: native in `bruma-renderer-wgpu`, WASM in Phase 7.
pub trait WallpaperRuntime {
    /// Advances the state to the next frame.
    ///
    /// `now` is the current monotonic instant; the runtime computes
    /// `delta` and `time` and applies the FPS cap. Returns `false` if, by
    /// pace (FPS cap), it is not yet time to paint and the caller may
    /// sleep until the next deadline.
    fn begin_frame(&mut self, now: std::time::Instant) -> FrameDecision;

    /// Target frames per second (0 = uncapped, runs at vblank/swapchain
    /// pace).
    fn target_fps(&self) -> u32;

    /// State of the current frame (valid after `begin_frame`).
    fn state(&self) -> FrameState;

    /// Current adjustable parameters (by name).
    fn params(&self) -> &[ParamValue];

    /// Updates a parameter by name; `false` if it does not exist.
    fn set_param(&mut self, name: &str, value: f32) -> bool;

    /// A second of work after presenting (stats, pauses...).
    /// The default does nothing.
    fn end_frame(&mut self) {}
}

/// Result of [`WallpaperRuntime::begin_frame`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameDecision {
    /// Time to paint: the state is already updated.
    Draw,
    /// Not yet: sleep until `deadline` at the latest.
    Skip {
        /// Absolute instant the next frame falls due.
        deadline: std::time::Instant,
    },
}

/// Test and user helper: minimum duration between frames for a target FPS
/// (0 fps => uncapped => `Duration::ZERO`).
pub fn frame_interval(target_fps: u32) -> Duration {
    if target_fps == 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(1_000_000_000 / u64::from(target_fps))
    }
}

/// General-purpose runtime: accumulated clock, FPS cap, mouse position and
/// adjustable parameters. Covers the vast majority of wallpapers; special
/// cases (scene with its own physics, visibility pause...) implement
/// [`WallpaperRuntime`] directly.
///
/// Always declares at least one `"param0"` parameter, the same one the
/// demo shader exposes (`u_params0`): so the CLI can animate it without
/// knowing the specific wallpaper.
#[derive(Debug, Clone)]
pub struct BasicRuntime {
    fps: u32,
    last: Option<Instant>,
    state: FrameState,
    params: Vec<ParamValue>,
    paused: bool,
}

impl BasicRuntime {
    /// New runtime with the given FPS cap (0 = uncapped).
    pub fn new(fps: u32) -> Self {
        BasicRuntime {
            fps,
            last: None,
            state: FrameState::default(),
            params: vec![ParamValue {
                name: "param0".to_owned(),
                value: 0.0,
            }],
            paused: false,
        }
    }

    /// Marks the runtime as paused from the start.
    pub fn paused(mut self) -> Self {
        self.paused = true;
        self
    }

    /// Pauses or resumes the animation. While paused, time does NOT
    /// advance and the runtime only wakes once per second (the Wayland
    /// socket wakes the loop anyway on any event).
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Sets an initial value for a parameter (e.g. from the CLI).
    /// Silently ignores unknown names: `set_param` does report, this
    /// constructor has nobody to report to.
    pub fn with_param(mut self, name: &str, value: f32) -> Self {
        let _ = self.set_param(name, value);
        self
    }

    /// Replaces the declared parameters with the given ones (max 4: what
    /// fits in the uniform block). Used by the CLI when loading a
    /// package, whose names come from the manifest.
    pub fn set_params(&mut self, mut params: Vec<ParamValue>) {
        params.truncate(16);
        log::info!(
            "runtime params set: {}",
            params
                .iter()
                .map(|p| format!("{}={:.2}", p.name, p.value))
                .collect::<Vec<_>>()
                .join(", ")
        );
        self.params = params;
    }

    /// Sets the value of the parameter at `index` (0..3), which is how it
    /// reaches the GPU (`u_params0..3`).
    pub fn set_param_at(&mut self, index: usize, value: f32) {
        if let Some(p) = self.params.get_mut(index) {
            p.value = value.clamp(0.0, 1.0);
        }
    }

    /// Updates the cursor position (called by the platform). Lands in
    /// `state.mouse_x/y`, which the renderer copies to the uniforms.
    pub fn set_mouse(&mut self, x: f32, y: f32) {
        self.state.mouse_x = x;
        self.state.mouse_y = y;
    }

    /// Updates the resolution of the drawing area (called by the platform
    /// after every configure).
    pub fn set_resolution(&mut self, width: u32, height: u32) {
        self.state.width = width;
        self.state.height = height;
    }
}

impl WallpaperRuntime for BasicRuntime {
    fn begin_frame(&mut self, now: Instant) -> FrameDecision {
        // While paused, time freezes: we don't touch `last` nor `time`.
        if self.paused {
            return FrameDecision::Skip {
                deadline: now + Duration::from_secs(1),
            };
        }
        let interval = frame_interval(self.fps);
        if let Some(last) = self.last
            && now.duration_since(last) < interval
        {
            return FrameDecision::Skip {
                deadline: last + interval,
            };
        }
        let delta = self
            .last
            .map_or(0.0, |l| now.duration_since(l).as_secs_f32());
        self.state.time += delta;
        self.state.delta = delta;
        self.last = Some(now);
        FrameDecision::Draw
    }

    fn target_fps(&self) -> u32 {
        self.fps
    }

    fn state(&self) -> FrameState {
        FrameState {
            params: std::array::from_fn(|i| self.params.get(i).map_or(0.0, |p| p.value)),
            ..self.state
        }
    }

    fn params(&self) -> &[ParamValue] {
        &self.params
    }

    fn set_param(&mut self, name: &str, value: f32) -> bool {
        match self.params.iter_mut().find(|p| p.name == name) {
            Some(p) => {
                p.value = value.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal runtime to validate the contract with tests: clock with an
    /// FPS cap and time accumulation.
    struct TestRuntime {
        last: Option<Instant>,
        time: f32,
        fps: u32,
    }

    impl WallpaperRuntime for TestRuntime {
        fn begin_frame(&mut self, now: Instant) -> FrameDecision {
            let interval = frame_interval(self.fps);
            if let Some(last) = self.last
                && now.duration_since(last) < interval
            {
                return FrameDecision::Skip {
                    deadline: last + interval,
                };
            }
            self.delta_applied(now);
            FrameDecision::Draw
        }

        fn target_fps(&self) -> u32 {
            self.fps
        }

        fn state(&self) -> FrameState {
            FrameState {
                time: self.time,
                ..FrameState::default()
            }
        }

        fn params(&self) -> &[ParamValue] {
            &[]
        }

        fn set_param(&mut self, _name: &str, _value: f32) -> bool {
            false
        }
    }

    impl TestRuntime {
        fn delta_applied(&mut self, now: Instant) {
            let delta = self
                .last
                .map_or(0.0, |l| now.duration_since(l).as_secs_f32());
            self.time += delta;
            self.last = Some(now);
        }
    }

    #[test]
    fn frame_interval_math() {
        assert_eq!(frame_interval(60), Duration::from_nanos(16_666_666));
        assert_eq!(frame_interval(30), Duration::from_nanos(33_333_333));
        assert_eq!(frame_interval(0), Duration::ZERO);
    }

    #[test]
    fn default_state_has_unknown_mouse() {
        let s = FrameState::default();
        assert_eq!((s.mouse_x, s.mouse_y), (-1.0, -1.0));
        assert_eq!(s.time, 0.0);
    }

    #[test]
    fn runtime_caps_fps_and_accumulates_time() {
        let mut rt = TestRuntime {
            last: None,
            time: 0.0,
            fps: 10,
        };
        let t0 = Instant::now();

        // First frame: always draws.
        assert_eq!(rt.begin_frame(t0), FrameDecision::Draw);

        // 30 ms later at 10 fps (interval 100 ms): skip with deadline.
        let t1 = t0 + Duration::from_millis(30);
        match rt.begin_frame(t1) {
            FrameDecision::Skip { deadline } => {
                assert_eq!(deadline - t0, Duration::from_millis(100));
            }
            other => panic!("expected Skip, got {other:?}"),
        }

        // 150 ms later: draws and accumulates time since the last frame.
        let t2 = t0 + Duration::from_millis(150);
        assert_eq!(rt.begin_frame(t2), FrameDecision::Draw);
        assert!((rt.state().time - 0.15).abs() < 1e-6);
    }

    #[test]
    fn basic_runtime_pause_freezes_time() {
        let mut rt = BasicRuntime::new(30).paused();
        let t0 = Instant::now();

        // Paused: skip with a far deadline (~1 s) and time untouched.
        match rt.begin_frame(t0) {
            FrameDecision::Skip { deadline } => {
                assert_eq!(deadline - t0, Duration::from_secs(1));
            }
            other => panic!("expected Skip, got {other:?}"),
        }

        let t1 = t0 + Duration::from_millis(2500);
        match rt.begin_frame(t1) {
            FrameDecision::Skip { .. } => {}
            other => panic!("expected Skip, got {other:?}"),
        }

        // Resume: the first frame is immediate and time starts at 0 (the
        // clock did not accumulate while paused).
        rt.set_paused(false);
        assert_eq!(rt.begin_frame(t1), FrameDecision::Draw);
        assert_eq!(rt.state().time, 0.0);

        let t2 = t1 + Duration::from_millis(100);
        assert_eq!(rt.begin_frame(t2), FrameDecision::Draw);
        assert!((rt.state().time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn basic_runtime_params_set_by_name() {
        let mut rt = BasicRuntime::new(60);
        assert!(rt.set_param("param0", 0.7));
        assert_eq!(rt.params()[0].value, 0.7);
        // Clamped to the declared 0..=1 range.
        assert!(rt.set_param("param0", 5.0));
        assert_eq!(rt.params()[0].value, 1.0);
        // Unknown name: false.
        assert!(!rt.set_param("does_not_exist", 0.5));
        // The fluent constructor applies it too.
        assert_eq!(
            BasicRuntime::new(60).with_param("param0", 0.25).params()[0].value,
            0.25
        );
    }

    #[test]
    fn basic_runtime_resolution_and_mouse() {
        let mut rt = BasicRuntime::new(60);
        rt.set_resolution(1920, 1200);
        rt.set_mouse(10.0, 20.0);
        let s = rt.state();
        assert_eq!((s.width, s.height), (1920, 1200));
        assert_eq!((s.mouse_x, s.mouse_y), (10.0, 20.0));
    }
}
