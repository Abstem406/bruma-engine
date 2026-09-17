//! Quad coverage test — the lesson from the black-half bug.
//!
//! Story: during Phases 2-4 the quad was drawn with `draw(0..4)` under
//! `TriangleList`, which only generates ONE triangle: the bottom-right
//! half of the screen never got painted and nobody noticed, because every
//! live verification measured pixel (5,5) — inside the triangle that did
//! work. The user saw it: an exact diagonal cut with the desktop showing
//! through.
//!
//! This test reproduces the production renderer's exact conditions (same
//! `compile_wgsl`, same `build_quad_pipeline`, same WGSL packaged in the
//! crate) but renders **offline** to a texture — no Wayland, no surface —
//! and reads the pixels back to assert that the 4 corners + center are
//! covered.
//!
//! Verification rule behind the assertions (recorded in
//! `ai-development-log.md`): the 4 corners + center, never a point in the
//! known-good region.
//!
//! Requires a GPU/Vulkan adapter (like the one bruma uses in production).
//! Without an adapter, the test skips with a notice instead of failing:
//! CI runs on GPU-less runners, and a wallpaper cannot demand more than
//! its own runtime needs. On a dev machine (where every hit of this
//! project landed) it runs fully.

use bruma_renderer_wgpu::{build_quad_pipeline, compile_wgsl};

/// Production shader's path inside the crate (included via `include_str!`
/// in the binary; read from the source tree here).
const HELLO_WGSL: &str = include_str!("../src/shaders/hello.wgsl");

/// Test render size. Small (fast) but with the corners well away from the
/// edges for the interior sampling.
const W: u32 = 256;
const H: u32 = 256;

/// Samples one interior pixel per region: 8px inward from each corner and
/// the exact center. With a broken strip (triangle 0-1-2 only), BR falls
/// outside the quad and keeps the clear color.
const SAMPLE_INSET: i64 = 8;

#[test]
fn quad_covers_the_four_corners() {
    let Some((device, queue)) = offline_device() else {
        eprintln!("skip: no GPU adapter (CI without vulkan/lavapipe); the full test runs on dev");
        return;
    };

    // Production compile path: naga first, module after.
    let module = compile_wgsl(&device, HELLO_WGSL, "test-hello")
        .expect("the production shader must compile");

    // Same uniform layout as the animated renderer (48 bytes, group 0,
    // binding 0) — the shader demands it.
    let (bgl, bg, _uniform_buf) = make_uniforms(&device, &queue);

    // The production pipeline with the offline render's format.
    let pipeline = build_quad_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb, &module, &bgl);

    // Destination texture: render attachment + copy source.
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

    // Readback: rows are aligned to 256 bytes.
    let bytes_per_row = W * 4; // 1024: already a multiple of 256
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
                // Unmistakable RED clear: if a corner stays red, the quad
                // did NOT cover it (the shader never paints red).
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
        // Identical to the animated renderer's draw.
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
        .expect("poll without errors");

    // Buffer read: map_async + poll (canonical pattern from the wgpu
    // examples; the callback runs inside the poll).
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), wgpu::BufferAsyncError>>();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        tx.send(r).expect("the map receiver lives");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll without errors");
    rx.recv()
        .expect("map callback invoked")
        .expect("readback map");
    let data = {
        let bytes = slice
            .get_mapped_range()
            .expect("readback mapped after poll");
        bytes.to_vec()
    };
    readback.unmap();

    let px = |x: i64, y: i64| -> [u8; 4] {
        let off = (y as usize * bytes_per_row as usize) + (x as usize * 4);
        [data[off], data[off + 1], data[off + 2], data[off + 3]]
    };

    // If the TriangleList bug ever came back, BR would hold the red
    // clear.
    let red = [255u8, 0, 0, 255];
    let right = W as i64 - 1 - SAMPLE_INSET;
    let bottom = H as i64 - 1 - SAMPLE_INSET;
    let samples = [
        ("TL", px(SAMPLE_INSET, SAMPLE_INSET)),
        ("TR", px(right, SAMPLE_INSET)),
        ("BL", px(SAMPLE_INSET, bottom)),
        ("BR", px(right, bottom)),
        ("C", px(W as i64 / 2, H as i64 / 2)),
    ];

    // 1) Coverage: no sample may hold the clear color. It is the
    // assertion that would have caught the black-half bug on day one.
    for (name, c) in &samples {
        assert_ne!(
            *c, red,
            "sample {name} left UNDRAWN (clear color): \
             the quad does not cover it — was TriangleStrip lost?"
        );
    }

    // 2) Symmetry: the shader's wave only depends on r = |uv·res|, so the
    // four corner samples (equidistant from the center by construction)
    // must have identical color. Any inequality betrays a mirrored,
    // shifted or wrongly wound quad.
    let (_, tl) = samples[0];
    let (_, tr) = samples[1];
    let (_, bl) = samples[2];
    let (_, br) = samples[3];
    assert_eq!(tl, tr, "TL and TR corners differ: mirrored quad?");
    assert_eq!(tl, bl, "TL and BL corners differ: mirrored quad?");
    assert_eq!(tl, br, "TL and BR corners differ: broken or shifted quad?");

    // 3) Contrast: with u_time=0, w(center)=sin(0)=0 and w(corner)≈0.73
    // (deterministic): the center MUST differ from the corners. If it
    // were equal, the texture does not reflect the shader's geometry
    // (e.g. the whole frame is clear, or the fragment gets no real uv).
    let center = samples[4].1;
    assert_ne!(
        center, tl,
        "the center is identical to the corners: the render does not \
         reflect the shader's wave (uniforms or uv unwired?)"
    );
}

/// Offline wgpu device (no surface): same adapter criterion as GpuContext
/// but without `compatible_surface`. `None` if there is no adapter
/// (GPU-less CI) — the test skips instead of failing.
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

/// Uniform block (48 bytes) and bind group with the SAME shape as the
/// animated renderer: time, params0, mouse, params, res, pad.
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
        // SAFETY: repr(C) of plain f32s, 48 bytes verified at compile time.
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
