//! # bruma-renderer-wgpu
//!
//! Implementación del contrato de renderizado con **wgpu + WGSL** (D3).
//!
//! Estado: **Fase 2, paso 1** — triángulo de bienvenida. La progresión de
//! la fase es: triángulo → quad → textura → imagen a pantalla completa.
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

use std::ptr::NonNull;

use bruma_renderer::FrameRenderer;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

/// Renderer de la Fase 2, paso 1: un triángulo WGSL a pantalla completa.
///
/// Más adelante (esta misma fase) evolucionará a quad + textura + imagen;
/// la estructura (instancia, adaptador, dispositivo, superficie) ya es la
/// definitiva.
pub struct WgpuRenderer {
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    configured_size: (u32, u32),
}

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
}

impl WgpuRenderer {
    /// Crea el renderer sobre la superficie Wayland indicada.
    ///
    /// # Safety
    ///
    /// Igual que [`wgpu::Instance::create_surface_unsafe`]: los punteros
    /// deben ser válidos y el display/surface deben sobrevivir a la
    /// `wgpu::Surface` resultante. El caller (`bruma`) garantiza el orden
    /// de dropeo: conexión Wayland > renderer.
    ///
    /// # Argumentos
    ///
    /// - `display_ptr`: puntero a `wl_display` de la conexión.
    /// - `surface_ptr`: puntero a `wl_surface` de la ventana de fondo.
    pub unsafe fn new_wayland(
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

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bruma-triangulo"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/triangle.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bruma-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: surface_format,
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
            _instance: instance,
            _adapter: adapter,
            device,
            queue,
            surface,
            surface_format,
            pipeline,
            configured_size: (0, 0),
        })
    }

    /// Configura (o reconfigura) la superficie de wgpu al tamaño dado.
    fn configure(&mut self, width: u32, height: u32) {
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
}

impl FrameRenderer for WgpuRenderer {
    fn render_frame(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.configure(width, height);

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // Superficie desactualizada: el próximo configure la arregla.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configured_size = (0, 0);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("wgpu: validación fallida al obtener textura");
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
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

        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }
}
