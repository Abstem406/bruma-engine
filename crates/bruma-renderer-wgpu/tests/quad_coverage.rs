//! Test de cobertura del quad — la lección del bug de la mitad negra.
//!
//! Historia: durante las Fases 2-4 el quad se dibujaba con `draw(0..4)`
//! en `TriangleList`, que solo genera UN triángulo: la mitad inferior
//! derecha de la pantalla nunca se pintó y nadie lo notó, porque todas
//! las verificaciones en vivo medían el píxel (5,5) — dentro del
//! triángulo que sí funcionaba. El usuario lo vio: un corte diagonal
//! exacto con el escritorio asomando detrás.
//!
//! Este test reproduce las condiciones exactas del renderer de
//! producción (mismo `compile_wgsl`, mismo `build_quad_pipeline`, mismo
//! WGSL empaquetado en el crate) pero renderiza **offline** a una
//! textura — sin Wayland, sin superficie — y lee los píxeles de vuelta
//! para afirmar que las 4 esquinas + centro quedan cubiertas.
//!
//! Regla de verificación que motiva las aserciones (registrada en
//! `ai-development-log.md`): las 4 esquinas + centro, nunca un punto de
//! la región sabida-buena.
//!
//! Requiere un adaptador GPU/Vulkan (como el que bruma usa en
//! producción). Sin adaptador, el test se salta con aviso en vez de
//! fallar: el CI corre en runners sin GPU, y un wallpaper no puede
//! exigir más que lo que su propio runtime necesita. En una máquina de
//! desarrollo (la de todos los hits de este proyecto) corre de lleno.

use bruma_renderer_wgpu::{build_quad_pipeline, compile_wgsl};

/// Ruta del shader de producción dentro del crate (incluido vía
/// `include_str!` en el binario; aquí se lee del árbol de fuentes).
const HELLO_WGSL: &str = include_str!("../src/shaders/hello.wgsl");

/// Tamaño del render de prueba. Pequeño (rápido) pero con las esquinas
/// bien separadas de los bordes para el muestreo interior.
const W: u32 = 256;
const H: u32 = 256;

/// Muestra un píxel interior de cada región: 8px hacia adentro de cada
/// esquina y el centro exacto. Con strip roto (solo triángulo 0-1-2),
/// BR cae fuera del quad y queda con el color de clear.
const SAMPLE_INSET: i64 = 8;

#[test]
fn quad_cubre_las_cuatro_esquinas() {
    let Some((device, queue)) = offline_device() else {
        eprintln!(
            "skip: sin adaptador GPU (CI sin vulkan/lavapipe); el test completo corre en dev"
        );
        return;
    };

    // Camino de compilación de producción: naga primero, módulo después.
    let module = compile_wgsl(&device, HELLO_WGSL, "test-hello")
        .expect("el shader de producción debe compilar");

    // Mismo layout de uniforms que el renderer animado (48 bytes,
    // group 0, binding 0) — el shader lo exige.
    let (bgl, bg, _uniform_buf) = make_uniforms(&device, &queue);

    // El pipeline de producción con el formato del render offline.
    let pipeline = build_quad_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb, &module, &bgl);

    // Textura destino: render attachment + origen de copia.
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-quad-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Copy de vuelta: las filas van alineadas a 256 bytes.
    let bytes_per_row = W * 4; // 1024: ya múltiplo de 256
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-quad-readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test-quad-enc"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test-quad-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                // Clear ROJO inconfundible: si una esquina queda roja es
                // que el quad NO la cubrió (el shader nunca pinta rojo).
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::RED),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bg, &[]);
        // Idéntico al draw del renderer animado.
        pass.draw(0..4, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll sin errores");

    // Lectura del buffer: map_async + poll (patrón canónico de los
    // ejemplos de wgpu; el callback corre dentro del poll).
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), wgpu::BufferAsyncError>>();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).expect("el receptor del map vive");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll sin errores");
    rx.recv()
        .expect("callback del map invocado")
        .expect("map del readback");
    let data = {
        let bytes = slice
            .get_mapped_range()
            .expect("readback mapeado tras el poll");
        bytes.to_vec()
    };
    readback.unmap();

    let px = |x: i64, y: i64| -> [u8; 4] {
        let off = (y as usize * bytes_per_row as usize) + (x as usize * 4);
        [data[off], data[off + 1], data[off + 2], data[off + 3]]
    };

    // Si el bug del TriangleList volviera, BR queda en el clear rojo.
    let rojo = [255u8, 0, 0, 255];
    let right = W as i64 - 1 - SAMPLE_INSET;
    let bottom = H as i64 - 1 - SAMPLE_INSET;
    let samples = [
        ("TL", px(SAMPLE_INSET, SAMPLE_INSET)),
        ("TR", px(right, SAMPLE_INSET)),
        ("BL", px(SAMPLE_INSET, bottom)),
        ("BR", px(right, bottom)),
        ("C", px(W as i64 / 2, H as i64 / 2)),
    ];

    // 1) Cobertura: ninguna muestra puede quedar con el clear. Es la
    // aserción que habría cazado el bug de la mitad negra desde el día 1.
    for (name, c) in &samples {
        assert_ne!(
            *c, rojo,
            "la muestra {name} quedó SIN dibujar (color de clear): \
             el quad no la cubre — ¿se perdió el TriangleStrip?"
        );
    }

    // 2) Simetría: la onda del shader solo depende de r = |uv·res|, así
    // que las cuatro muestras de esquina (equidistantes del centro por
    // construcción) deben tener color idéntico. Una desigualdad delata
    // quad espejado, desplazado o con winding al revés.
    let (_, tl) = samples[0];
    let (_, tr) = samples[1];
    let (_, bl) = samples[2];
    let (_, br) = samples[3];
    assert_eq!(tl, tr, "esquinas TL y TR difieren: ¿quad espejado?");
    assert_eq!(tl, bl, "esquinas TL y BL difieren: ¿quad espejado?");
    assert_eq!(
        tl, br,
        "esquinas TL y BR difieren: ¿quad roto o desplazado?"
    );

    // 3) Contraste: con u_time=0, w(centro)=sin(0)=0 y w(esquina)≈0.73
    // (determinista): el centro DEBE diferir de las esquinas. Si fuera
    // igual, la textura no refleja la geometría del shader (p. ej.
    // todo el frame es clear, o el fragment no recibe uv reales).
    let centro = samples[4].1;
    assert_ne!(
        centro, tl,
        "el centro es idéntico a las esquinas: el render no refleja \
         la onda del shader (¿uniforms o uv sin conectar?)"
    );
}

/// Dispositivo wgpu offline (sin superficie): mismo criterio de
/// adaptador que GpuContext pero sin `compatible_surface`. `None` si no
/// hay adaptador (CI sin GPU) — el test se salta en vez de fallar.
fn offline_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: None,
        ..Default::default()
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("bruma-test-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    Some((device, queue))
}

/// Uniform block (48 bytes) y bind group con la MISMA forma que el
/// renderer animado: time, params0, mouse, params, res, pad.
fn make_uniforms(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::BindGroupLayout, wgpu::BindGroup, wgpu::Buffer) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct U {
        time: f32,
        params0: f32,
        mouse: [f32; 2],
        params: [f32; 4],
        res: [f32; 2],
        pad: [f32; 2],
    }
    const _: () = assert!(std::mem::size_of::<U>() == 48);

    let block = U {
        time: 0.0,
        params0: 0.9,
        mouse: [-1.0, -1.0],
        params: [0.9, 0.0, 0.0, 0.0],
        res: [W as f32, H as f32],
        pad: [0.0, 0.0],
    };

    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-uniforms"),
        size: 48,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &buf,
        0,
        // SAFETY: repr(C) de f32 puros, 48 bytes verificados en compile-time.
        unsafe { std::slice::from_raw_parts(&block as *const U as *const u8, 48) },
    );

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test-bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(48),
            },
            count: None,
        }],
    });
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test-bg"),
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buf,
                offset: 0,
                size: None,
            }),
        }],
    });
    (bgl, bg, buf)
}
