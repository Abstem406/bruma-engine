//! # bruma-renderer-wgpu
//!
//! Rendering contract implementation with **wgpu + WGSL** (D3).
//!
//! Status: **Phase 3**. Progression: triangle ✅ → image ✅ → animated
//! shader with FPS cap and WGSL hot-reload.
//!
//! Platform boundary: this crate knows nothing about Wayland. It receives
//! raw pointers (`wl_display`, `wl_surface`) and turns them into
//! `raw-window-handle` handles for wgpu. `bruma-platform` is the one that
//! knows how to extract them.
//!
//! # Safety
//!
//! This crate contains the engine's only `unsafe` block: creating the
//! wgpu surface from raw pointers. The invariants are:
//! - The `wl_display` and `wl_surface` must stay alive while the
//!   `wgpu::Surface` exists (guaranteed by the connection's owner:
//!   `BackgroundWindow` outlives the renderer in the composition).
//! - The pointers must be valid (they come from live wayland-client
//!   proxies).

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::Arc;

use bruma_renderer::FrameRenderer;
use bruma_renderer::FrameState;
use image::ImageReader;

/// Alias of the frame contract for crate consumers (the CLI annotates
/// types with this without depending directly on bruma-renderer).
pub use bruma_renderer::FrameRenderer as FrameRendererAlias;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

/// Renderer errors.
#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    /// No compatible GPU adapter (is Vulkan available?).
    #[error("no compatible GPU adapter found: {0}")]
    NoAdapter(String),
    /// The logical device could not be created.
    #[error("could not create the wgpu device: {0}")]
    Device(String),
    /// The surface supports no texture format.
    #[error("the surface supports no formats")]
    NoSurfaceFormats,
    /// The image could not be loaded.
    #[error("error loading image: {0}")]
    Image(String),
    /// The shader file could not be read.
    #[error("error reading shader {}: {0}", path.display())]
    ShaderIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The WGSL shader does not compile (naga/wgpu message).
    #[error("error compiling shader: {0}")]
    ShaderCompile(String),
}

/// Uniform block of the animated shader (64 bytes, no padding).
///
/// GPU layout (same as `Uniforms` in the shaders):
/// ```text
/// offset 0:  u_time    f32
/// offset 4:  u_params0 f32
/// offset 8:  u_mouse   vec2f
/// offset 16: u_params  vec4f (u_params0..3)
/// offset 32: u_res     vec2f
/// offset 40: u_clock   vec3f (local h/m/s for day/night and clocks)
/// offset 52: (end padding: WGSL rounds a uniform struct's size up to a
///             multiple of 16 → 64 bytes)
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Uniforms {
    time: f32,
    params0: f32,
    mouse: [f32; 2],
    params: [f32; 4],
    res: [f32; 2],
    /// Cursor speed in buffer px/s (0 = still or unknown): sim shaders
    /// inject energy proportional to it (a continuous wake, not
    /// repeated plops).
    mouse_speed: f32,
    /// WGSL alignment: vec3 sits at offset 48 (Rust's repr(C) would put
    /// the array at 44 — this pad holds the WGSL position).
    _pad44: f32,
    /// Real-time clock `[h, m, s]` (WGSL offset 48, ends at 60).
    clock: [f32; 3],
    /// 8-byte alignment pad so `mouse_prev` sits at 64, where the WGSL
    /// struct reads it (vec2 alignment; the mirror of `_pad60` there).
    _pad60: f32,
    /// Previous frame's cursor position (buffer px; (-1,-1) unknown):
    /// stroke shaders inject along the moved segment (prev -> mouse).
    /// WGSL offset 64.
    mouse_prev: [f32; 2],
    /// Tail pad to the 80-byte block (WGSL struct round-up).
    _pad_end: [f32; 2],
}

// The block is uploaded to the GPU as raw bytes: padding-free by
// construction.
const _: () = assert!(size_of::<Uniforms>() as u64 == UNIFORM_SIZE);

impl Uniforms {
    /// Byte view of the block (for `Queue::write_buffer`).
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Uniforms` is a #[repr(C)] of plain f32s (80 bytes
        // with the explicit WGSL pads, verified above) and the resulting
        // slice is only read.
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, size_of::<Self>()) }
    }
}

/// Layout constant shared between platform and runtime.
const UNIFORM_SIZE: u64 = 80;

/// Texture slots of the fixed group-0 layout (matches the manifest's
/// `textures` cap): slot i occupies bindings 2i+1 (texture) and 2i+2
/// (sampler).
const TEXTURE_SLOTS: usize = 4;

/// Offscreen target format for feedback wallpapers (the creator's
/// pipeline renders here and reads the result back next frame). fp16
/// WITHOUT an sRGB gamma curve: the stored value IS the sim state, and
/// an 8-bit sRGB target quantizes it into banding that the wave math
/// then freezes in place (rings that never fade). fp16 keeps the state
/// continuous frame to frame.
pub const SIM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Bind group layout of group 1: the wallpaper's PREVIOUS frame
/// (`feedback` permission). One texture + one sampler; pipelines always
/// carry it so hot-reload can switch between feedback and non-feedback
/// shaders without rebuilding the layout (a group the shader doesn't
/// declare is simply unused).
pub fn prev_frame_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bruma-prev-frame-bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Blit shader for the feedback path: copies the offscreen frame where
/// the creator's shader just painted onto the swapchain texture. Reads
/// group 1 (the same layout the feedback shader uses for its input).
pub const BLIT_WGSL: &str = r#"
@group(1) @binding(0)
var prev_tex: texture_2d<f32>;

@group(1) @binding(1)
var prev_samp: sampler;

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
    );
    let uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    var out: VsOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    // Same orientation contract as the creator shaders' vertices
    // (uv.y = 0 at the top of the target): a blit of an upright frame
    // must stay upright.
    out.uv = uvs[idx];
    return out;
}

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    return textureSample(prev_tex, prev_samp, in.uv);
}
"#;

/// Appended to the CREATOR's shader source to build the display blit: a
/// WGSL module is one compilation unit, so the only way for the blit to
/// call the creator's `display(uv, frame, u)` (and see its `U` uniform
/// block and group-1 `prev_tex`/`prev_samp`) is to compile the combined
/// source. Adds its own vertex entry with bruma-prefixed names so it
/// cannot collide with the creator's declarations. If the combined
/// module fails to compile, the engine falls back to the plain copy
/// blit (standalone, always valid).
pub const BLIT_DISPLAY_APPEND: &str = r#"
struct BrumaDisplayOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn bruma_display_vs(@builtin(vertex_index) idx: u32) -> BrumaDisplayOut {
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
    );
    let uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    var out: BrumaDisplayOut;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    // Same orientation contract as the creator shaders' vertices.
    out.uv = uvs[idx];
    return out;
}

@fragment
fn fs_display(in: BrumaDisplayOut) -> @location(0) vec4<f32> {
    return display(in.uv, textureSample(prev_tex, prev_samp, in.uv), U);
}
"#;

/// Builds the quad pipeline (`TriangleStrip` topology, `REPLACE` blend,
/// vertices generated in the WGSL) for an already compiled module and its
/// bind group layouts: group 0 (uniforms + textures), group 1 (previous
/// frame; unused by shaders without the `feedback` permission).
///
/// `fragment_entry` selects the fragment entry point ("fs_main", or
/// "fs_display" for the internal display blit).
///
/// Pure with respect to surfaces: it only knows the device and the target
/// format. So [`crate::AnimatedRenderer`] and the coverage tests use the
/// SAME code — the test validates the production pipeline, not a copy.
///
/// Geometry (contract with the shaders): 4 vertices in strip order (TL,
/// TR, BL, BR → triangles 0-1-2 and 1-2-3); `draw(0..3)` for the triangle
/// demo is identical under strip. History: with `TriangleList` only the
/// first triangle was drawn — half the screen unpainted during Phases
/// 2-4, invisible to verifications that only measured (5,5).
pub fn build_quad_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    module: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    prev_frame_layout: Option<&wgpu::BindGroupLayout>,
) -> wgpu::RenderPipeline {
    build_quad_pipeline_entry(
        device,
        format,
        module,
        bind_group_layout,
        prev_frame_layout,
        "fs_main",
    )
}

/// [`build_quad_pipeline`] with an explicit fragment entry point (the
/// internal display blit uses `fs_display`).
#[allow(clippy::too_many_arguments)]
pub fn build_quad_pipeline_entry(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    module: &wgpu::ShaderModule,
    bind_group_layout: &wgpu::BindGroupLayout,
    prev_frame_layout: Option<&wgpu::BindGroupLayout>,
    fragment_entry: &str,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bruma-quad-layout"),
        bind_group_layouts: &[Some(bind_group_layout), prev_frame_layout],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bruma-quad-pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// GPU context **shared across outputs** (Phase 5): instance, adapter,
/// device and queue. `Device`/`Queue`/`Instance`/`Adapter` are inner
/// `Arc`s (`Clone`): a single GPU serves all surfaces — the opposite (one
/// device per output) multiplies driver VRAM.
///
/// Adapter choice: `LowPower` deliberately — bruma runs 24/7 and on
/// hybrid systems (iGPU + dGPU) the discrete one is better left asleep.
/// FUTURE: explicit selection via config/CLI instead of leaving it to the
/// driver.
#[derive(Clone)]
pub struct GpuShared {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuShared {
    /// Discovers the adapter and creates the shared device (no surface:
    /// it serves any output arriving later).
    pub fn new() -> Result<Self, RendererError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bruma-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .map_err(|e| RendererError::Device(e.to_string()))?; // Uncaptured validation errors are logged, not fatal: a broken
        // frame must not take the daemon down (same philosophy as shader
        // hot-reload). Without this handler wgpu panics on first error.
        device.on_uncaptured_error(Arc::new(move |error| {
            log::error!("wgpu: {error}");
        }));
        let info = adapter.get_info();
        log::info!(
            "wgpu: adapter {} ({:?}), backend {:?} (shared across outputs)",
            info.name,
            info.device_type,
            info.backend
        );
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }
}

impl Default for GpuShared {
    fn default() -> Self {
        Self::new().expect("GPU adapter available (is Vulkan up?)")
    }
}

/// wgpu surface of ONE output, on the shared [`GpuShared`].
///
/// Joins the raw Wayland surface with its configuration (per-output
/// chosen format — they may differ — and size). It is the only per-output
/// thing; device/queue are shared.
pub struct SurfaceCtx {
    shared: GpuShared,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    configured_size: (u32, u32),
}

// SAFETY: the raw pointers must be valid and the display/surface must
// outlive the resulting wgpu::Surface; the caller (bruma CLI) guarantees
// the drop order: Wayland connection > renderers.
impl SurfaceCtx {
    /// Creates the surface over the given Wayland connection and picks
    /// the swapchain format (the first one this output supports).
    ///
    /// # Safety
    ///
    /// Same as [`wgpu::Instance::create_surface_unsafe`]: the pointers
    /// must be valid and the display/surface must outlive the resulting
    /// `wgpu::Surface`. The caller guarantees the drop order: Wayland
    /// connection > `SurfaceCtx`.
    pub unsafe fn new_wayland(
        shared: &GpuShared,
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
    ) -> Result<Self, RendererError> {
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display_ptr));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_ptr));
        let surface = unsafe {
            shared
                .instance
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display_handle),
                    raw_window_handle: window_handle,
                })
        }
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;

        let format = surface
            .get_capabilities(&shared.adapter)
            .formats
            .first()
            .copied()
            .ok_or(RendererError::NoSurfaceFormats)?;
        log::debug!("surface format (output): {format:?}");

        Ok(Self {
            shared: shared.clone(),
            surface,
            format,
            configured_size: (0, 0),
        })
    }

    /// Configures (or reconfigures) the surface at the given size.
    pub fn configure(&mut self, width: u32, height: u32) {
        if self.configured_size == (width, height) {
            return;
        }
        self.surface.configure(
            &self.shared.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width,
                height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
        self.configured_size = (width, height);
    }

    /// Acquires the current frame's texture, handling transient states
    /// (reconfigures after Outdated/Lost on the next frame).
    pub fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configured_size = (0, 0);
                None
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => None,
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("wgpu: validation failed while acquiring the texture");
                None
            }
        }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.shared.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.shared.queue
    }

    /// Presents the frame (submit + present).
    pub fn submit_and_present(&self, encoder: wgpu::CommandEncoder, frame: wgpu::SurfaceTexture) {
        self.shared.queue.submit([encoder.finish()]);
        self.shared.queue.present(frame);
    }
}

/// Phase 2 renderer, step 1: a fullscreen WGSL triangle.
pub struct WgpuRenderer {
    surface: SurfaceCtx,
    pipeline: wgpu::RenderPipeline,
}

impl WgpuRenderer {
    /// Creates the triangle renderer over the given Wayland surface.
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`]: the pointers must be valid
    /// and outlive the surface; the caller guarantees the drop order
    /// connection > renderer.
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
    ) -> Result<Self, RendererError> {
        let shared = GpuShared::new()?;
        unsafe { Self::on_shared(&shared, display_ptr, surface_ptr) }
    }

    /// Creates the triangle renderer on a shared GPU (Phase 5:
    /// multi-output). `new_wayland` is the single-renderer wrapper.
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`].
    pub unsafe fn on_shared(
        shared: &GpuShared,
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
    ) -> Result<Self, RendererError> {
        let surface = unsafe { SurfaceCtx::new_wayland(shared, display_ptr, surface_ptr)? };

        let shader = surface
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("bruma-triangle"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/triangle.wgsl").into()),
            });

        let pipeline_layout =
            surface
                .device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("bruma-layout"),
                    bind_group_layouts: &[],
                    immediate_size: 0,
                });

        let pipeline = surface
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bruma-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface.format(),
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                // Shader quads are 4 vertices in strip order (TL, TR, BL,
                // BR → triangles 0-1-2 and 1-2-3); the triangle demo
                // (draw 0..3) is identical under strip. With list, only
                // the first triangle was drawn: half the screen black.
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self { surface, pipeline })
    }
}

impl FrameRenderer for WgpuRenderer {
    fn render_frame(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface.configure(width, height);
        let Some(frame) = self.surface.acquire_frame() else {
            return;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.surface
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bruma-frame"),
                });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bruma-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }

        self.surface.submit_and_present(encoder, frame);
    }
}

/// Phase 2 renderer, step 2: fullscreen image (PNG/JPEG).
///
/// The image is uploaded once as an RGBA8 (sRGB) texture; a quad covers
/// the screen and the shader samples it. The aspect ratio is NOT
/// corrected yet: the image is stretched to the screen size (pending
/// decision for the `.wallpaper` manifest: cover/contain/stretch).
pub struct ImageRenderer {
    surface: SurfaceCtx,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl ImageRenderer {
    /// Creates the image renderer over the given Wayland surface.
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`]: the pointers must be valid
    /// and outlive the surface; the caller guarantees the drop order
    /// connection > renderer.
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        image_path: &Path,
    ) -> Result<Self, RendererError> {
        let shared = GpuShared::new()?;
        unsafe { Self::on_shared(&shared, display_ptr, surface_ptr, image_path) }
    }

    /// Creates the image renderer on a shared GPU (Phase 5:
    /// multi-output): the texture is uploaded per output (it is small
    /// next to the duplicated device we avoid).
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`].
    pub unsafe fn on_shared(
        shared: &GpuShared,
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        image_path: &Path,
    ) -> Result<Self, RendererError> {
        let ctx = unsafe { SurfaceCtx::new_wayland(shared, display_ptr, surface_ptr)? };

        let img = image::open(image_path).map_err(|e| RendererError::Image(e.to_string()))?;
        let rgba = img.to_rgba8();
        let (iw, ih) = (rgba.width(), rgba.height());
        log::info!("image texture: {iw}x{ih}px");

        let texture = ctx.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("bruma-image"),
            size: wgpu::Extent3d {
                width: iw,
                height: ih,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Single image upload (RGBA8, 4 bytes per pixel).
        ctx.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * iw),
                rows_per_image: Some(ih),
            },
            wgpu::Extent3d {
                width: iw,
                height: ih,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = ctx.device().create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bruma-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout =
            ctx.device()
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("bruma-image-bgl"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let bind_group = ctx.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bruma-image-bind"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout =
            ctx.device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("bruma-image-layout"),
                    bind_group_layouts: &[Some(&bind_group_layout)],
                    immediate_size: 0,
                });

        let shader = ctx
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("bruma-image-shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image.wgsl").into()),
            });

        let pipeline = ctx
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bruma-image-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format(),
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                // Shader quads are 4 vertices in strip order (TL, TR, BL,
                // BR → triangles 0-1-2 and 1-2-3); the triangle demo
                // (draw 0..3) is identical under strip. With list, only
                // the first triangle was drawn: half the screen black.
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self {
            surface: ctx,
            pipeline,
            bind_group,
        })
    }
}

impl FrameRenderer for ImageRenderer {
    fn render_frame(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.surface.configure(width, height);
        let Some(frame) = self.surface.acquire_frame() else {
            return;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.surface
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bruma-image-frame"),
                });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bruma-image-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..4, 0..1);
        }

        self.surface.submit_and_present(encoder, frame);
    }
}

/// Loads and validates a WGSL module, returning compile errors with
/// naga's message (independent of the
/// `fragile-send-sync-non-atomic-wgpu` feature). Compiles WGSL with
/// synchronous naga validation and useful error messages (line/column).
/// Public so tests use the same path as production.
///
/// Validation is synchronous: the source is validated directly with naga
/// (wgpu's compile-time dependency, already in the tree) BEFORE creating
/// the GPU module. An invalid shader never reaches the GPU.
pub fn compile_wgsl(
    device: &wgpu::Device,
    source: &str,
    label: &str,
) -> Result<wgpu::ShaderModule, RendererError> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| RendererError::ShaderCompile(e.emit_to_string(source)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    let info = validator
        .validate(&module)
        .map_err(|e| RendererError::ShaderCompile(format!("{e}")))?;
    let _ = info;

    // With the source already validated, GPU module creation cannot fail
    // on compilation (error scopes would cover API validation errors,
    // which do not apply here).
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    }))
}

/// Reload event callback; see [`AnimatedRenderer::set_reload_callback`].
type ReloadCallback = Box<dyn FnMut(&ReloadEvent)>;

/// Phase 3 renderer: fullscreen animated WGSL shader.
///
/// - **Uniforms**: time, parameter 0, mouse position and resolution,
///   updated every frame (`render_animated`).
/// - **Hot-reload**: the file is re-read if its mtime changed (lazy poll,
///   once per frame). A broken shader does NOT kill the wallpaper: the
///   previous pipeline is kept and the failure is logged (the next valid
///   file applies on its own).
pub struct AnimatedRenderer {
    ctx: SurfaceCtx,
    /// Shader path and last seen mtime (for hot-reload).
    shader_path: PathBuf,
    last_mtime: Option<std::time::SystemTime>,
    /// Uniform buffer + bind group (fixed layout, shared by every
    /// pipeline created in hot-reload).
    uniform_buf: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /// Group-1 placeholder (1×1 dummy texture): every pipeline carries
    /// the prev-frame layout, so non-feedback draws bind this to satisfy
    /// the layout without feeding the shader a real frame.
    dummy_prev: wgpu::BindGroup,
    /// Current pipeline (replaced on each valid hot-reload).
    pipeline: wgpu::RenderPipeline,
    /// Last configured size (to repaint after reload without waiting for
    /// a new configure).
    last_size: (u32, u32),
    /// Previous pointer position (buffer px) for the speed term.
    last_mouse: [f32; 2],
    /// Wake telemetry state: is a stroke in progress, and its peak
    /// speed so far. One log line per stroke (start/end), never per
    /// frame.
    wake_active: bool,
    wake_peak: f32,
    /// Last logged effective params: log on CHANGE (the first frame is
    /// the configure-time draw with defaults-zero; the animated frames
    /// carry the manifest values — the log must show the transition,
    /// not just the misleading first shot).
    logged_params: [f32; 4],
    /// Optional reload event callback (e.g. to turn a shader rejection
    /// into a desktop notification). Invoked on the loop's thread, never
    /// on the render path.
    on_reload: Option<ReloadCallback>,
    /// Last hot-reload compile error: dedup of autosaves rewriting the
    /// same broken content, and recovery notice with the pipeline already
    /// active.
    last_error: Option<String>,
    /// THIS output's parameter overrides: position in `params[]` → value.
    /// Resolved by NAME against the manifest in the CLI (one output can
    /// have `intensidad=0.2` and another `0.9` with the same shader and
    /// shared runtime → synchronized animation).
    param_overrides: Vec<(usize, f32)>,
    /// Texture and sampler views for the shader's texture slots, in
    /// binding order (slot i → bindings 2i+1 / 2i+2). Initialized to
    /// 1×1 dummies; [`Self::set_textures`] replaces them from the
    /// package's `assets/`.
    texture_views: Vec<wgpu::TextureView>,
    texture_samplers: Vec<wgpu::Sampler>,
    /// Previous-frame input requested (manifest `feedback` permission).
    feedback: bool,
    /// The creator shader's CURRENT source (re-read on every hot-reload
    /// and used to compile the display blit, which concatenates the
    /// creator code + the internal append).
    creator_source: String,
    /// The shader declares `fn display(uv, frame, u)` — the feedback blit
    /// runs it through `fs_display` so the creator controls how the
    /// offscreen state reaches the screen (water over a photo).
    display_entry: bool,
    /// Group-1 layout for the previous frame. EVERY pipeline carries it
    /// (created in the constructor): a shader declaring group 1 needs it
    /// from the first pipeline on, and hot-reload can flip between
    /// feedback and non-feedback variants without rebuilding anything.
    prev_frame_layout: wgpu::BindGroupLayout,
    /// Offscreen ping-pong, built lazily on the first frame (the swapchain
    /// format is known by then). `None` while no frame ran yet.
    ping_pong: Option<PingPong>,
}

/// The two offscreen targets of a feedback wallpaper plus the blit
/// pipeline that copies the just-painted one to the swapchain. Rebuilt
/// when the frame size changes.
struct PingPong {
    size: (u32, u32),
    /// Kept alive: a dropped texture invalidates its views and bind
    /// groups (the fields above only hold the views).
    _textures: [wgpu::Texture; 2],
    views: [wgpu::TextureView; 2],
    /// Group-1 bind groups: `groups[i]` binds `views[i]` + the sampler.
    /// The feedback shader reads `groups[read]`; the blit copies from
    /// `groups[write]`.
    groups: [wgpu::BindGroup; 2],
    blit: wgpu::RenderPipeline,
    /// Even frame → read 0 / write 1; toggles each frame.
    flip: bool,
}

/// Shader reload event for [`AnimatedRenderer::set_reload_callback`]'s
/// callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReloadEvent {
    /// The shader was applied (first load or valid hot-reload).
    Applied,
    /// The new shader does not compile; the previous pipeline stays on
    /// screen. `error` is the clipped naga message.
    Rejected { error: String },
    /// After one or more rejections, the shader compiled again.
    Recovered,
}

impl AnimatedRenderer {
    /// Cursor speed in buffer px/s for this frame, with per-output
    /// continuity: a mouse state (x >= 0) that followed an unknown one
    /// ((-1, -1)) starts a fresh trail instead of one huge jump across
    /// the gap (output switch, cursor left the background).
    fn mouse_speed(&mut self, state: &FrameState) -> f32 {
        let now = [state.mouse_x, state.mouse_y];
        let speed = if state.mouse_x >= 0.0 && self.last_mouse[0] >= 0.0 && state.delta > 0.0 {
            let dx = now[0] - self.last_mouse[0];
            let dy = now[1] - self.last_mouse[1];
            (dx * dx + dy * dy).sqrt() / state.delta
        } else {
            0.0
        };
        self.last_mouse = now;
        // Sanity clamp: a teleport-scale spike (output switch measured
        // across the gap, a stalled frame) is not a wake.
        speed.min(20_000.0)
    }

    /// Wake telemetry: one INFO line when a stroke begins (cursor speed
    /// and what the shader will actually inject) and one when it ends
    /// (its peak). Hysteresis keeps mid-gesture pauses from flapping.
    /// This is the creator's window into the wave: if these lines
    /// appear, the pointer path works — what you SEE is then pure
    /// shading.
    fn log_wake(&mut self, speed: f32, params: &[f32; 4]) {
        const START: f32 = 25.0;
        const END: f32 = 12.0;
        if !self.wake_active {
            if speed >= START {
                self.wake_active = true;
                self.wake_peak = speed;
                log::info!(
                    "wake START: cursor {:.0} px/s → injecting {:.0}% (intensity {:.2}, damping {:.2}, ambient {:.2})",
                    speed,
                    if speed > 5.0 {
                        (speed / 450.0).clamp(0.35, 1.0) * 100.0
                    } else {
                        0.0
                    },
                    params[0],
                    params[1],
                    params[2],
                );
            }
        } else {
            self.wake_peak = self.wake_peak.max(speed);
            if speed < END {
                self.wake_active = false;
                log::info!("wake END: peak {:.0} px/s", self.wake_peak);
            }
        }
    }
    /// Creates the animated renderer from a `.wgsl` file.
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`]: the pointers must be valid
    /// and outlive the surface; the caller guarantees the drop order
    /// connection > renderer.
    ///
    /// # Errors
    ///
    /// Returns [`RendererError::ShaderCompile`] if the initial shader
    /// does not compile: without a pipeline there is no wallpaper. (After
    /// startup, hot-reload errors are tolerated keeping the old
    /// pipeline.)
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        shader_path: &Path,
        feedback: bool,
        display_entry: bool,
    ) -> Result<Self, RendererError> {
        let shared = GpuShared::new()?;
        unsafe {
            Self::on_shared(
                &shared,
                display_ptr,
                surface_ptr,
                shader_path,
                feedback,
                display_entry,
            )
        }
    }

    /// Creates the animated renderer on a shared GPU (Phase 5: one GPU
    /// serves all outputs; each output has its own surface, uniforms and
    /// pipeline — pipelines are cheap, the device is not).
    ///
    /// # Safety
    ///
    /// Same as [`SurfaceCtx::new_wayland`].
    pub unsafe fn on_shared(
        shared: &GpuShared,
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        shader_path: &Path,
        feedback: bool,
        display_entry: bool,
    ) -> Result<Self, RendererError> {
        let ctx = unsafe { SurfaceCtx::new_wayland(shared, display_ptr, surface_ptr)? };

        let source =
            std::fs::read_to_string(shader_path).map_err(|source| RendererError::ShaderIo {
                path: shader_path.to_owned(),
                source,
            })?;

        let uniform_buf = ctx.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("bruma-uniforms"),
            size: UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Fixed group-0 layout: uniform + 4 texture/sampler pairs. A
        // layout entry the shader doesn't use is fine; the bind group
        // always fills every entry (dummies for undeclared slots), so
        // hot-reloaded shader variants share this one layout.
        let mut layout_entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
            },
            count: None,
        }];
        for i in 0..TEXTURE_SLOTS {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: (2 * i + 1) as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: (2 * i + 2) as u32,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let bind_group_layout =
            ctx.device()
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("bruma-anim-bgl"),
                    entries: &layout_entries,
                });
        // Group 1 (previous frame) exists for every pipeline: unused by
        // non-feedback shaders, ready when a feedback shader lands via
        // hot-reload.
        let prev_frame_layout = prev_frame_bind_group_layout(ctx.device());

        // 1x1 opaque black dummies: declared-but-unbound slots sample
        // black instead of failing validation.
        let device = ctx.device();
        let dummy = ctx.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("bruma-texture-dummy"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let dummy_view = dummy.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = ctx.device().create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bruma-texture-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Initial bind group: every slot bound to the 1×1 dummy (a
        // shader that samples undeclared textures gets black, not an
        // error). `set_textures` swaps in real views from the package.
        let initial_views: Vec<wgpu::TextureView> =
            (0..TEXTURE_SLOTS).map(|_| dummy_view.clone()).collect();
        let initial_samplers: Vec<wgpu::Sampler> =
            (0..TEXTURE_SLOTS).map(|_| sampler.clone()).collect();
        let view_refs: Vec<&wgpu::TextureView> = initial_views.iter().collect();
        let sampler_refs: Vec<&wgpu::Sampler> = initial_samplers.iter().collect();
        let dummy_prev = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bruma-prev-dummy-bg"),
            layout: &prev_frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&dummy_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bind_group = Self::make_bind_group(
            &uniform_buf,
            device,
            &bind_group_layout,
            &view_refs,
            &sampler_refs,
        );
        // A feedback shader's fs_main is a SIM pass into the fp16
        // offscreen targets: its pipeline must target SIM_FORMAT, never
        // the swapchain format (rendering a sim into an 8-bit sRGB
        // target quantizes the state into rings that never fade — the
        // frozen-water bug). The mode flags are constructor arguments so
        // this decision happens BEFORE the first pipeline exists.
        let shader_format = if feedback { SIM_FORMAT } else { ctx.format() };
        let pipeline = Self::build_pipeline(
            &ctx,
            shader_format,
            &source,
            &bind_group_layout,
            Some(&prev_frame_layout),
        )?;

        let renderer = Self {
            ctx,
            shader_path: shader_path.to_owned(),
            last_mtime: Self::mtime(shader_path),
            uniform_buf,
            bind_group_layout,
            bind_group,
            dummy_prev,
            pipeline,
            last_size: (0, 0),
            last_mouse: [-1.0, -1.0],
            wake_active: false,
            wake_peak: 0.0,
            logged_params: [-1.0; 4],
            on_reload: None,
            last_error: None,
            param_overrides: Vec::new(),
            texture_views: initial_views,
            texture_samplers: initial_samplers,
            feedback,
            display_entry,
            creator_source: source.clone(),
            prev_frame_layout,
            ping_pong: None,
        };
        Ok(renderer)
    }

    /// Sets THIS output's parameter overrides (Phase 5).
    ///
    /// `overrides` comes as (parameter_position, value) pairs; the
    /// name→position resolution is done by the CLI against the manifest
    /// (the renderer knows nothing about manifests). Values outside
    /// 0..=1 are clamped; positions outside 0..4 are dropped.
    pub fn set_param_overrides(&mut self, overrides: Vec<(usize, f32)>) {
        self.param_overrides = overrides
            .into_iter()
            .filter(|(i, _)| *i < 4)
            .map(|(i, v)| (i, v.clamp(0.0, 1.0)))
            .collect();
    }

    /// Installs the reload event callback (e.g. to turn a shader
    /// rejection into a desktop notification).
    pub fn set_reload_callback(&mut self, cb: ReloadCallback) {
        self.on_reload = Some(cb);
    }

    /// File mtime, if it can be stat'ed.
    fn mtime(path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Builds a bind group from the given texture views and samplers
    /// (one pair per slot, in binding order). Slot i occupies bindings
    /// 2i+1 (texture) and 2i+2 (sampler); binding 0 is the uniform block.
    fn make_bind_group(
        uniform_buf: &wgpu::Buffer,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        views: &[&wgpu::TextureView],
        samplers: &[&wgpu::Sampler],
    ) -> wgpu::BindGroup {
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(uniform_buf.as_entire_buffer_binding()),
        }];
        for (i, (view, sampler)) in views.iter().zip(samplers.iter()).enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: (2 * i + 1) as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: (2 * i + 2) as u32,
                resource: wgpu::BindingResource::Sampler(sampler),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bruma-anim-bg"),
            layout,
            entries: &entries,
        })
    }

    /// Uploads the package's textures into the fixed layout slots, in
    /// declaration order (slot i = bindings 2i+1 / 2i+2).
    ///
    /// Called by the CLI after construction, BEFORE the first frame: the
    /// decoder (`image` crate, already a dependency) hands over an RGBA8
    /// buffer that goes to the GPU with `write_texture` (native path; the
    /// external-image copy is web-only). Failure degrades to black
    /// textures and is logged: a missing image must not kill the
    /// wallpaper.
    pub fn set_textures(&mut self, paths: &[String]) {
        if paths.is_empty() {
            return;
        }
        for (i, path) in paths.iter().enumerate() {
            if i >= TEXTURE_SLOTS {
                break;
            }
            let Ok(reader) = ImageReader::open(path) else {
                log::warn!("texture {}: could not open {}", i, path);
                continue;
            };
            let Ok(img) = reader.decode() else {
                log::warn!("texture {}: could not decode {}", i, path);
                continue;
            };
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            if w == 0 || h == 0 {
                log::warn!("texture {}: empty image {}", i, path);
                continue;
            }
            let texture = self.ctx.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("bruma-package-texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.ctx.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            self.texture_views[i] = texture.create_view(&wgpu::TextureViewDescriptor::default());
            log::info!("texture {} loaded: {} ({}x{})", i, path, w, h);
        }
        self.rebuild_bind_group();
    }

    /// Recreates the bind group from the current views and samplers
    /// (after a texture slot changes).
    fn rebuild_bind_group(&mut self) {
        let views: Vec<&wgpu::TextureView> = self.texture_views.iter().collect();
        let samplers: Vec<&wgpu::Sampler> = self.texture_samplers.iter().collect();
        self.bind_group = Self::make_bind_group(
            &self.uniform_buf,
            self.ctx.device(),
            &self.bind_group_layout,
            &views,
            &samplers,
        );
    }

    /// Compiles the shader and builds the pipeline with bruma's standard
    /// layout (uniform block on group 0, binding 0). `format` is the
    /// pass's target: [`SIM_FORMAT`] for the sim pass of a feedback
    /// shader, the output's swapchain format otherwise.
    fn build_pipeline(
        ctx: &SurfaceCtx,
        format: wgpu::TextureFormat,
        source: &str,
        bind_group_layout: &wgpu::BindGroupLayout,
        prev_frame_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<wgpu::RenderPipeline, RendererError> {
        let shader = compile_wgsl(ctx.device(), source, "bruma-anim-shader")?;

        Ok(build_quad_pipeline(
            ctx.device(),
            format,
            &shader,
            bind_group_layout,
            prev_frame_layout,
        ))
    }

    /// Builds the display blit pipeline for a creator shader that
    /// declares `display(uv, frame, u)`: the creator source gets the
    /// `bruma_display_*` append (its own vertex entry + the `fs_display`
    /// fragment that calls into the creator's code). If the combined
    /// module does not compile, the plain copy blit is used instead —
    /// the wallpaper never goes black over a broken display entry.
    fn build_display_blit(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        creator_source: &str,
        group0_layout: &wgpu::BindGroupLayout,
        prev_layout: &wgpu::BindGroupLayout,
    ) -> Option<wgpu::RenderPipeline> {
        let combined = format!("{creator_source}\n{BLIT_DISPLAY_APPEND}");
        let Ok(module) = compile_wgsl(device, &combined, "bruma-display-blit") else {
            return None;
        };
        Some(build_quad_pipeline_entry(
            device,
            format,
            &module,
            group0_layout,
            Some(prev_layout),
            "fs_display",
        ))
    }

    /// Emits an event through the callback if one is installed.
    fn emit(&mut self, event: &ReloadEvent) {
        if let Some(cb) = self.on_reload.as_mut() {
            cb(event);
        }
    }

    /// Reloads the shader if the file changed since the last load.
    ///
    /// Strategy: lazy mtime (one stat per frame, ~1µs) + synchronous
    /// naga validation. If the new file does not compile, the previous
    /// pipeline is kept, the error recorded and [`ReloadEvent::Rejected`]
    /// emitted (with dedup: same bytes = a single event). When the file
    /// becomes valid again, [`ReloadEvent::Recovered`] is emitted if
    /// there were previous errors.
    fn maybe_reload(&mut self, width: u32, height: u32) {
        if Self::mtime(&self.shader_path) == self.last_mtime {
            return;
        }
        self.last_mtime = Self::mtime(&self.shader_path);

        let Ok(source) = std::fs::read_to_string(&self.shader_path) else {
            log::warn!("hot-reload: could not read {}", self.shader_path.display());
            return;
        };
        self.creator_source = source.clone();
        match Self::build_pipeline(
            &self.ctx,
            if self.feedback {
                SIM_FORMAT
            } else {
                self.ctx.format()
            },
            &source,
            &self.bind_group_layout,
            Some(&self.prev_frame_layout),
        ) {
            Ok(pipeline) => {
                log::info!("hot-reload: shader applied");
                self.pipeline = pipeline;
                if self.last_error.take().is_some() {
                    // Recovery after rejection(s): the new pipeline is
                    // already on screen; we notify the configured channel.
                    self.emit(&ReloadEvent::Recovered);
                } else {
                    self.emit(&ReloadEvent::Applied);
                }
                // Repaint immediately with the new shader.
                self.draw(width, height);
            }
            Err(e) => {
                let msg = e.to_string();
                log::warn!("hot-reload ignored: {e}");
                // Dedup: if the error matches the previous one, do not
                // re-emit (editors rewrite the file identically).
                if self.last_error.as_ref() != Some(&msg) {
                    self.last_error = Some(msg.clone());
                    self.emit(&ReloadEvent::Rejected { error: msg });
                }
            }
        }
    }

    /// Renders one frame at the given size with the runtime's state.
    fn draw(&mut self, width: u32, height: u32) {
        self.ctx.configure(width, height);
        let Some(frame) = self.ctx.acquire_frame() else {
            return;
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.ctx
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bruma-anim-frame"),
                });

        let format = self.ctx.format();

        // Feedback path: two passes inside ONE encoder and submission —
        // the creator's shader paints offscreen (reading the previous
        // frame), then a blit copies the result to the swapchain.
        if self.feedback {
            let device = self.ctx.device();
            let Some(pp) = Self::ensure_ping_pong(
                &mut self.ping_pong,
                device,
                self.ctx.queue(),
                &self.bind_group_layout,
                &self.prev_frame_layout,
                self.display_entry,
                &self.creator_source,
                width,
                height,
                format,
            ) else {
                log::warn!("feedback: offscreen targets unavailable; frame skipped");
                self.ctx.submit_and_present(encoder, frame);
                return;
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bruma-feedback-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &pp.views[1 - pp.flip as usize],
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_bind_group(1, &pp.groups[pp.flip as usize], &[]);
                pass.draw(0..4, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bruma-blit-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // The blit declares BOTH groups: it shares the pipeline
                // layout with the feedback shader (group 0 must be
                // satisfied even unused).
                pass.set_pipeline(&pp.blit);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_bind_group(1, &pp.groups[1 - pp.flip as usize], &[]);
                pass.draw(0..4, 0..1);
            }
            pp.flip = !pp.flip;
            self.ctx.submit_and_present(encoder, frame);
            return;
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bruma-anim-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &self.dummy_prev, &[]);
        pass.draw(0..4, 0..1);
        drop(pass);

        self.ctx.submit_and_present(encoder, frame);
    }

    /// Returns the ping-pong pool for the given size, allocating or
    /// re-allocating it when missing or resized.
    ///
    /// Takes the pool slot, device and group-1 layout as separate
    /// arguments (not `&mut self`): the caller keeps borrowing other
    /// fields (`pipeline`, `bind_group`) for the render passes while
    /// `pp` stays alive — field-disjoint borrows, impossible through a
    /// whole-`self` method call.
    #[allow(clippy::too_many_arguments)]
    fn ensure_ping_pong<'a>(
        slot: &'a mut Option<PingPong>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        group0_layout: &wgpu::BindGroupLayout,
        prev_layout: &wgpu::BindGroupLayout,
        display_entry: bool,
        creator_source: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Option<&'a mut PingPong> {
        // The offscreen targets are fp16 LINEAR (SIM_FORMAT): the stored
        // value is the creator's sim state and must stay continuous — an
        // 8-bit sRGB target quantizes it into bands the wave math then
        // freezes. The creator pipeline targets this same format; only
        // the display blit writes the swapchain.
        let rebuild = match &*slot {
            Some(pp) => pp.size != (width, height),
            None => true,
        };
        if rebuild {
            let desc = wgpu::TextureDescriptor {
                label: Some("bruma-feedback-target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SIM_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            };
            let textures = [device.create_texture(&desc), device.create_texture(&desc)];
            // GPU-zeroed memory is NOT the sim's rest state: the state
            // encoding centers at 0.5 (see the templates' header), so a
            // zero target reads as height -0.5 and the wave equation
            // drives the whole field into the -1 clamp — a pinned fixed
            // point where the cursor's drop gate can never open (the
            // wallpaper dies as a static over-bright image). Initialize
            // both targets to CALM (0.5, 0.5, 0.5, 1) as the contract
            // says.
            let calm: Vec<u8> = [0x00u8, 0x38, 0x00, 0x38, 0x00, 0x38, 0x00, 0x3C]
                .repeat((width * height) as usize);
            for t in &textures {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: t,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &calm,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 8),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let views = [
                textures[0].create_view(&wgpu::TextureViewDescriptor::default()),
                textures[1].create_view(&wgpu::TextureViewDescriptor::default()),
            ];
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("bruma-feedback-sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let groups = [
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bruma-feedback-read0"),
                    layout: prev_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&views[0]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                }),
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bruma-feedback-read1"),
                    layout: prev_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&views[1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                }),
            ];
            let Ok(blit_module) = compile_wgsl(device, BLIT_WGSL, "bruma-feedback-blit") else {
                // The blit is built from a const string: failure would
                // be a programming error, not a runtime condition.
                log::error!("feedback: internal blit shader failed to compile");
                return None;
            }; // The blit shares the creator shader's two-group layout: it
            // declares only group 1 (its source frame), but group 0 must
            // still be present in the pipeline layout (and bound in the
            // pass, even unused).
            //
            // Display mode compiles the CREATOR source + the append so
            // the blit can call `display(uv, frame, U)`; a combined
            // module that does not compile falls back to the plain copy
            // blit (creator display code with a bug = plain presentation,
            // never a dead wallpaper).
            let blit = if display_entry {
                Self::build_display_blit(device, format, creator_source, group0_layout, prev_layout)
                    .unwrap_or_else(|| {
                        log::warn!(
                            "feedback: display entry failed to compile; falling back to plain blit"
                        );
                        build_quad_pipeline_entry(
                            device,
                            format,
                            &blit_module,
                            group0_layout,
                            Some(prev_layout),
                            "fs_main",
                        )
                    })
            } else {
                build_quad_pipeline_entry(
                    device,
                    format,
                    &blit_module,
                    group0_layout,
                    Some(prev_layout),
                    "fs_main",
                )
            };
            *slot = Some(PingPong {
                size: (width, height),
                _textures: textures,
                views,
                groups,
                blit,
                flip: false,
            });
        }
        slot.as_mut()
    }
}

impl FrameRenderer for AnimatedRenderer {
    fn wants_animation(&self) -> bool {
        true
    }

    fn render_frame(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.last_size = (width, height);
        // No runtime at the first configure: render with default state
        // (time 0); the animated loop takes over right away.
        let state = FrameState {
            width,
            height,
            ..FrameState::default()
        };
        self.render_animated(&state);
    }

    fn render_animated(&mut self, state: &FrameState) {
        if state.width == 0 || state.height == 0 {
            return;
        }
        // Lazy hot-reload: only if the mtime changed.
        self.maybe_reload(state.width, state.height);

        // Applies THIS output's overrides over the global state.
        let mut params = state.params;
        for (idx, value) in &self.param_overrides {
            if let Some(p) = params.get_mut(*idx) {
                *p = *value;
            }
        }

        // Uploads the frame's uniforms. NOTE the order: mouse_speed()
        // updates last_mouse, so the previous position must be captured
        // BEFORE calling it.
        let mouse_prev = self.last_mouse;
        let speed = self.mouse_speed(state);
        self.log_wake(speed, &params);
        if params != self.logged_params {
            self.logged_params = params;
            log::info!(
                "effective params (u_params0..3): [{:.2}, {:.2}, {:.2}, {:.2}]",
                params[0],
                params[1],
                params[2],
                params[3],
            );
        }
        let uniforms = Uniforms {
            time: state.time,
            // Phase 3: `param0` defaulting to 0. The UI generated from
            // the .wallpaper manifest arrives in Phase 6.
            params0: params.first().copied().unwrap_or(0.0),
            mouse: [state.mouse_x, state.mouse_y],
            params,
            res: [state.width as f32, state.height as f32],
            mouse_speed: speed,
            _pad44: 0.0,
            clock: state.clock,
            _pad60: 0.0,
            mouse_prev,
            _pad_end: [0.0; 2],
        };
        self.ctx
            .queue()
            .write_buffer(&self.uniform_buf, 0, uniforms.as_bytes());

        self.last_size = (state.width, state.height);
        self.draw(state.width, state.height);
    }
}
