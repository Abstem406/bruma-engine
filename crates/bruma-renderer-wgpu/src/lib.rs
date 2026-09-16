//! # bruma-renderer-wgpu
//!
//! Implementación del contrato de renderizado con **wgpu + WGSL** (D3).
//!
//! Estado: **Fase 3**. Progresión: triángulo ✅ → imagen ✅ → shader
//! animado con límite de FPS y hot-reload de WGSL.
//!
//! Frontera con la plataforma: este crate no sabe nada de Wayland. Recibe
//! punteros crudos (`wl_display`, `wl_surface`) y los traduce a handles de
//! `raw-window-handle` para wgpu. `bruma-platform` es quien sabe sacarlos.
//!
//! # Seguridad
//!
//! Este crate contiene el único bloque `unsafe` del motor: crear la
//! superficie de wgpu a partir de punteros crudos. Los invariantes son:
//! - El `wl_display` y el `wl_surface` deben permanecer vivos mientras
//!   exista la `wgpu::Surface` (garantizado por el dueño de la conexión:
//!   `BackgroundWindow` vive más que el renderer en la composición).
//! - Los punteros deben ser válidos (salen de proxies vivos de
//!   wayland-client).

#![forbid(unsafe_op_in_unsafe_fn)]

use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use bruma_renderer::FrameRenderer;
use bruma_renderer::FrameState;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

/// Errores del renderer.
#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    /// No hay adaptador GPU compatible (¿Vulkan disponible?).
    #[error("no se encontró adaptador GPU compatible: {0}")]
    NoAdapter(String),
    /// No se pudo crear el dispositivo lógico.
    #[error("no se pudo crear el dispositivo wgpu: {0}")]
    Device(String),
    /// La superficie no soporta ningún formato de textura.
    #[error("la superficie no soporta ningún formato")]
    NoSurfaceFormats,
    /// No se pudo cargar la imagen.
    #[error("error cargando imagen: {0}")]
    Image(String),
    /// No se pudo leer el archivo de shader.
    #[error("error leyendo el shader {}: {0}", path.display())]
    ShaderIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// El shader WGSL no compila (mensaje de naga/wgpu).
    #[error("error compilando el shader: {0}")]
    ShaderCompile(String),
}

/// Bloque de uniforms del shader animado (48 bytes, sin relleno).
///
/// Layout en GPU (igual que `Uniforms` en los shaders):
/// ```text
/// offset 0:  u_time    f32
/// offset 4:  u_params0 f32
/// offset 8:  u_mouse   vec2f
/// offset 16: u_params  vec4f (u_params0..3)
/// offset 32: u_res     vec2f
/// offset 40: (relleno final: WGSL redondea el tamaño de una struct
///             de uniform a múltiplo de 16 → 48 bytes)
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

// El bloque se sube a GPU como bytes crudos: sin relleno, por construcción.
const _: () = assert!(size_of::<Uniforms>() as u64 == UNIFORM_SIZE);

impl Uniforms {
    /// Vista de bytes del bloque (para `Queue::write_buffer`).
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: `Uniforms` es #[repr(C)] de f32 puros (32 bytes sin
        // padding, verificado arriba) y el slice resultante solo se lee.
        unsafe { std::slice::from_raw_parts(self as *const Self as *const u8, size_of::<Self>()) }
    }
}

/// Layout de la configuración compartida entre plataforma y runtime.
const UNIFORM_SIZE: u64 = 48;

/// Estado GPU compartido por todos los renderers de esta fase: instancia,
/// adaptador, dispositivo, cola y la superficie ya asociada al adaptador.
///
/// Elección de adaptador: `LowPower` deliberado — bruma corre 24/7 y en
/// sistemas híbridos (iGPU + dGPU) conviene que la dedicada duerma.
/// FUTURO (Fase 5): selección explícita por config/CLI en lugar de dejar
/// la decisión al driver.
pub(crate) struct GpuContext {
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    configured_size: (u32, u32),
}

// SAFETY: los punteros crudos deben ser válidos y el display/surface
// deben sobrevivir a la wgpu::Surface resultante; el caller (bruma CLI)
// garantiza el orden de dropeo: conexión Wayland > renderer.
impl GpuContext {
    /// Crea el contexto sobre la superficie Wayland indicada.
    ///
    /// # Safety
    ///
    /// Igual que [`wgpu::Instance::create_surface_unsafe`]: los punteros
    /// deben ser válidos y el display/surface deben sobrevivir a la
    /// `wgpu::Surface` resultante. El caller (`bruma`) garantiza el orden
    /// de dropeo: conexión Wayland > renderer.
    pub(crate) unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
    ) -> Result<Self, RendererError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display_ptr));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(surface_ptr));
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: Some(display_handle),
                raw_window_handle: window_handle,
            })
        }
        .map_err(|e| RendererError::NoAdapter(e.to_string()))?;

        // El adaptador debe ser compatible con ESTA superficie (no vale
        // cualquiera): pide uno que pueda presentar en ella.
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
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

        let adapter_info = adapter.get_info();
        log::info!(
            "wgpu: adaptador {} ({:?}), backend {:?}",
            adapter_info.name,
            adapter_info.device_type,
            adapter_info.backend
        );

        let surface_format = surface
            .get_capabilities(&adapter)
            .formats
            .first()
            .copied()
            .ok_or(RendererError::NoSurfaceFormats)?;
        log::debug!("formato de superficie: {surface_format:?}");

        Ok(Self {
            _instance: instance,
            _adapter: adapter,
            device,
            queue,
            surface,
            surface_format,
            configured_size: (0, 0),
        })
    }

    /// Configura (o reconfigura) la superficie de wgpu al tamaño dado.
    pub(crate) fn configure(&mut self, width: u32, height: u32) {
        if self.configured_size == (width, height) {
            return;
        }
        self.surface.configure(
            &self.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.surface_format,
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

    /// Obtiene la textura del frame actual, gestionando los estados
    /// transitorios (reconfigura tras Outdated/Lost en el próximo frame).
    pub(crate) fn acquire_frame(&mut self) -> Option<wgpu::SurfaceTexture> {
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configured_size = (0, 0);
                None
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => None,
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("wgpu: validación fallida al obtener textura");
                None
            }
        }
    }

    pub(crate) fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub(crate) fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub(crate) fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    fn submit_and_present(&self, encoder: wgpu::CommandEncoder, frame: wgpu::SurfaceTexture) {
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}

/// Renderer de la Fase 2, paso 1: un triángulo WGSL a pantalla completa.
pub struct WgpuRenderer {
    ctx: GpuContext,
    pipeline: wgpu::RenderPipeline,
}

impl WgpuRenderer {
    /// Crea el renderer del triángulo sobre la superficie Wayland dada.
    ///
    /// # Safety
    ///
    /// Igual que [`GpuContext::new_wayland`]: los punteros deben ser
    /// válidos y sobrevivir a la superficie; el caller garantiza el orden
    /// de dropeo conexión > renderer.
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
    ) -> Result<Self, RendererError> {
        let ctx = unsafe { GpuContext::new_wayland(display_ptr, surface_ptr)? };

        let shader = ctx
            .device()
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("bruma-triangulo"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/triangle.wgsl").into()),
            });

        let pipeline_layout =
            ctx.device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("bruma-layout"),
                    bind_group_layouts: &[],
                    immediate_size: 0,
                });

        let pipeline = ctx
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
                        format: ctx.surface_format(),
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self { ctx, pipeline })
    }
}

impl FrameRenderer for WgpuRenderer {
    fn render_frame(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
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

        self.ctx.submit_and_present(encoder, frame);
    }
}

/// Renderer de la Fase 2, paso 2: imagen (PNG/JPEG) a pantalla completa.
///
/// La imagen se sube una única vez como textura RGBA8 (sRGB); un quad
/// cubre la pantalla y el shader la muestrea. La relación de aspecto NO
/// se corrige aún: la imagen se estira al tamaño de la pantalla (decisión
/// pendiente para el manifiesto de `.wallpaper`: cover/contain/estirar).
pub struct ImageRenderer {
    ctx: GpuContext,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl ImageRenderer {
    /// Crea el renderer de imagen sobre la superficie Wayland dada.
    ///
    /// # Safety
    ///
    /// Igual que [`GpuContext::new_wayland`]: los punteros deben ser
    /// válidos y sobrevivir a la superficie; el caller garantiza el orden
    /// de dropeo conexión > renderer.
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        image_path: &Path,
    ) -> Result<Self, RendererError> {
        let ctx = unsafe { GpuContext::new_wayland(display_ptr, surface_ptr)? };

        let img = image::open(image_path).map_err(|e| RendererError::Image(e.to_string()))?;
        let rgba = img.to_rgba8();
        let (iw, ih) = (rgba.width(), rgba.height());
        log::info!("textura de imagen: {iw}x{ih}px");

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

        // Subida única de la imagen (RGBA8, 4 bytes por píxel).
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
                        format: ctx.surface_format(),
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        Ok(Self {
            ctx,
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

        self.ctx.submit_and_present(encoder, frame);
    }
}

/// Carga y valida un módulo WGSL, devolviendo errores de compilación con
/// el mensaje de naga (independiente de la feature `fragile-send-sync-non-atomic-wgpu`).
///
/// La validación es síncrona: se valida el fuente directamente con naga
/// (la dependencia de compilación de wgpu, ya en el árbol) ANTES de
/// crear el módulo GPU. Un shader inválido nunca llega a la GPU.
fn compile_wgsl(
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

    // Con el fuente ya validado, la creación del módulo GPU no puede
    // fallar por compilación (los error scopes cubrirían errores de
    // validación de la API, que aquí no aplican).
    Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    }))
}

/// Renderer de la Fase 3: shader WGSL animado a pantalla completa.
///
/// - **Uniforms**: tiempo, parámetro 0, posición del mouse y resolución,
///   actualizados en cada frame (`render_animated`).
/// - **Hot-reload**: el archivo se relee si cambió su mtime (poll
///   perezoso, una vez por frame). Un shader con errores NO mata el
///   wallpaper: se conserva el pipeline anterior y se reporta por log
///   (el próximo archivo válido se aplicará solo).
pub struct AnimatedRenderer {
    ctx: GpuContext,
    /// Ruta del shader y último mtime visto (para el hot-reload).
    shader_path: PathBuf,
    last_mtime: Option<std::time::SystemTime>,
    /// Uniform buffer + bind group (layout fijo, compartido por todos los
    /// pipelines que se creen en hot-reload).
    uniform_buf: wgpu::Buffer,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    /// Pipeline actual (reemplazado en cada hot-reload válido).
    pipeline: wgpu::RenderPipeline,
    /// Último tamaño configurado (para repintar tras el reload sin
    /// esperar un configure nuevo).
    last_size: (u32, u32),
}

impl AnimatedRenderer {
    /// Crea el renderer animado a partir de un archivo `.wgsl`.
    ///
    /// # Safety
    ///
    /// Igual que [`GpuContext::new_wayland`]: los punteros deben ser
    /// válidos y sobrevivir a la superficie; el caller garantiza el orden
    /// de dropeo conexión > renderer.
    ///
    /// # Errores
    ///
    /// Devuelve [`RendererError::ShaderCompile`] si el shader inicial no
    /// compila: sin pipeline no hay wallpaper. (Tras el arranque, los
    /// errores de hot-reload se toleran conservando el pipeline viejo.)
    pub unsafe fn new_wayland(
        display_ptr: NonNull<std::ffi::c_void>,
        surface_ptr: NonNull<std::ffi::c_void>,
        shader_path: &Path,
    ) -> Result<Self, RendererError> {
        let ctx = unsafe { GpuContext::new_wayland(display_ptr, surface_ptr)? };

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
        })
    }

    /// Mtime del archivo, si se puede stat-ear.
    fn mtime(path: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Compila el shader y construye el pipeline con el layout estándar de
    /// bruma (uniform block en group 0, binding 0).
    fn build_pipeline(
        ctx: &GpuContext,
        source: &str,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Result<wgpu::RenderPipeline, RendererError> {
        let shader = compile_wgsl(ctx.device(), source, "bruma-anim-shader")?;

        let pipeline_layout =
            ctx.device()
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("bruma-anim-layout"),
                    bind_group_layouts: &[Some(bind_group_layout)],
                    immediate_size: 0,
                });

        Ok(ctx
            .device()
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bruma-anim-pipeline"),
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
                        format: ctx.surface_format(),
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }))
    }

    /// Recarga el shader si el archivo cambió desde la última carga.
    ///
    /// Estrategia: mtime perezoso (una stat por frame, ~1µs) + validación
    /// síncrona con naga. Si el archivo nuevo no compila, el pipeline
    /// anterior se conserva y el error queda en el log; cuando el archivo
    /// vuelva a ser válido, se aplicará en el próximo frame.
    fn maybe_reload(&mut self, width: u32, height: u32) {
        if Self::mtime(&self.shader_path) == self.last_mtime {
            return;
        }
        self.last_mtime = Self::mtime(&self.shader_path);

        let Ok(source) = std::fs::read_to_string(&self.shader_path) else {
            log::warn!("hot-reload: no se pudo leer {}", self.shader_path.display());
            return;
        };
        match Self::build_pipeline(&self.ctx, &source, &self.bind_group_layout) {
            Ok(pipeline) => {
                log::info!("hot-reload: shader aplicado");
                self.pipeline = pipeline;
                // Repinta inmediatamente con el shader nuevo.
                self.draw(width, height);
            }
            Err(e) => log::warn!("hot-reload ignorado: {e}"),
        }
    }

    /// Renderiza un frame al tamaño dado con el estado del runtime.
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
        // Sin runtime en el primer configure, renderiza con estado por
        // defecto (tiempo 0); el bucle animado lo releva enseguida.
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
        // Hot-reload perezoso: solo si el mtime cambió.
        self.maybe_reload(state.width, state.height);

        // Sube los uniforms del frame (32 bytes).
        let uniforms = Uniforms {
            time: state.time,
            // Fase 3: `param0` con valor por defecto 0. La UI generada
            // desde el manifiesto .wallpaper llega en la Fase 6.
            params0: state.params.first().copied().unwrap_or(0.0),
            mouse: [state.mouse_x, state.mouse_y],
            params: state.params,
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
