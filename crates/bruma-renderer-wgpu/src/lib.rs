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

use bruma_renderer::FrameRenderer;
use bruma_renderer::FrameState;

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

/// Uniform block of the animated shader (48 bytes, no padding).
///
/// GPU layout (same as `Uniforms` in the shaders):
/// ```text
/// offset 0:  u_time    f32
/// offset 4:  u_params0 f32
/// offset 8:  u_mouse   vec2f
/// offset 16: u_params  vec4f (u_params0..3)
/// offset 32: u_res     vec2f
/// offset 40: (end padding: WGSL rounds a uniform struct's size up to a
///             multiple of 16 → 48 bytes)
/// ```
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct Uniforms {
    time: f32,
    params0: f32,
    mouse: [f32; 2],
    params: [f32; 4],
    res: [f32; 2],
    _pad_end: [f32; 2],
}

// The block is uploaded to the GPU as raw bytes: padding-free by
// construction.
const _: () = assert!(size_of::<Uniforms>() as u64 == UNIFORM_SIZE);

impl Uniforms {
    /// Byte view of the block (for `Queue::write_buffer`).
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Uniforms` is a #[repr(C)] of plain f32s (48 bytes
        // without padding, verified above) and the resulting slice is
        // only read.
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, size_of::<Self>()) }
    }
}

/// Layout constant shared between platform and runtime.
const UNIFORM_SIZE: u64 = 48;

/// Builds the quad pipeline (`TriangleStrip` topology, `REPLACE` blend,
/// vertices generated in the WGSL) for an already compiled module and its
/// bind group layout.
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
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bruma-quad-layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
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
            entry_point: Some("fs_main"),
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
        .map_err(|e| RendererError::Device(e.to_string()))?;
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
    /// Current pipeline (replaced on each valid hot-reload).
    pipeline: wgpu::RenderPipeline,
    /// Last configured size (to repaint after reload without waiting for
    /// a new configure).
    last_size: (u32, u32),
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
    ) -> Result<Self, RendererError> {
        let shared = GpuShared::new()?;
        unsafe { Self::on_shared(&shared, display_ptr, surface_ptr, shader_path) }
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

        let bind_group_layout =
            ctx.device()
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("bruma-anim-bgl"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                        },
                        count: None,
                    }],
                });

        let bind_group = ctx.device().create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bruma-anim-bind"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buf,
                    offset: 0,
                    size: None,
                }),
            }],
        });

        let pipeline = Self::build_pipeline(&ctx, &source, &bind_group_layout)?;

        Ok(Self {
            ctx,
            shader_path: shader_path.to_owned(),
            last_mtime: Self::mtime(shader_path),
            uniform_buf,
            bind_group_layout,
            bind_group,
            pipeline,
            last_size: (0, 0),
            on_reload: None,
            last_error: None,
            param_overrides: Vec::new(),
        })
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

    /// Compiles the shader and builds the pipeline with bruma's standard
    /// layout (uniform block on group 0, binding 0).
    fn build_pipeline(
        ctx: &SurfaceCtx,
        source: &str,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Result<wgpu::RenderPipeline, RendererError> {
        let shader = compile_wgsl(ctx.device(), source, "bruma-anim-shader")?;

        Ok(build_quad_pipeline(
            ctx.device(),
            ctx.format(),
            &shader,
            bind_group_layout,
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
        match Self::build_pipeline(&self.ctx, &source, &self.bind_group_layout) {
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

        {
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
            pass.draw(0..4, 0..1);
        }

        self.ctx.submit_and_present(encoder, frame);
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

        // Uploads the frame's uniforms (32 bytes).
        let uniforms = Uniforms {
            time: state.time,
            // Phase 3: `param0` defaulting to 0. The UI generated from
            // the .wallpaper manifest arrives in Phase 6.
            params0: params.first().copied().unwrap_or(0.0),
            mouse: [state.mouse_x, state.mouse_y],
            params,
            res: [state.width as f32, state.height as f32],
            _pad_end: [0.0; 2],
        };
        self.ctx
            .queue()
            .write_buffer(&self.uniform_buf, 0, uniforms.as_bytes());

        self.last_size = (state.width, state.height);
        self.draw(state.width, state.height);
    }
}
