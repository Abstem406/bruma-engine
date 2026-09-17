//! Integration test for the display-entry feedback path (the
//! `water-cursor` simulation shader).
//!
//! Reproduces the PRODUCTION wiring offline: the creator source + the
//! internal `BLIT_DISPLAY_APPEND` compiled as one module, the display
//! blit pipeline through `fs_display`, the sim pipeline through the
//! creator's own `fs_main`, both over a real GPU (skips without one).
//!
//! What is asserted, from the shader's deterministic math:
//! 1. The blit calls the creator's `display()`: the screen shows the
//!    REFRACTED photo, not the raw sim state (display ≠ pass-through).
//! 2. The pointer drop (u_mouse) injects energy into the sim: the sim
//!    output differs under a mouse position vs. mouse unknown.
//!
//! The photo asset is embedded as raw RGBA so the test does not depend
//! on the crate's asset path at runtime.

use bruma_renderer_wgpu::{
    BLIT_DISPLAY_APPEND, build_quad_pipeline_entry, compile_wgsl, prev_frame_bind_group_layout,
};
use image::ImageReader;

const WATER_CURSOR_WGSL: &str = include_str!("../../bruma/src/templates/water-cursor.wgsl");
const LAKE_JPG: &[u8] = include_bytes!("../../bruma/src/templates/assets/water-cursor.jpg");

const W: u32 = 192;
const H: u32 = 120;

#[test]
fn display_blit_runs_the_creator_display_and_the_drop_injects() {
    let Some((device, queue)) = offline_device() else {
        eprintln!("skip: no GPU adapter; the full test runs on dev machines");
        return;
    };

    // ---- The photo as a GPU texture (group 0, binding 1/2) ----
    let photo = ImageReader::new(std::io::Cursor::new(LAKE_JPG))
        .with_guessed_format()
        .expect("jpg reader")
        .decode()
        .expect("embedded jpg decodes")
        .to_rgba8();
    let photo_view = create_texture_view(&device, &queue, &photo, photo.dimensions());

    // ---- Group 1: the "previous frame" texture the passes read ----
    let prev_layout = prev_frame_bind_group_layout(&device);
    let init = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-prev-init"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let calm = vec![128u8; (W * H * 4) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &init,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &calm,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(W * 4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    let prev_view = init.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let prev_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test-prev-bg"),
        layout: &prev_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&prev_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    // ---- Group 0: uniforms ----
    let group0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("test-group0"),
        entries: &[
            uniform_entry(),
            texture_entry(1),
            sampler_entry(2),
            texture_entry(3),
            sampler_entry(4),
            texture_entry(5),
            sampler_entry(6),
            texture_entry(7),
            sampler_entry(8),
        ],
    });

    // ---- Display blit: creator source + append, entry fs_display ----
    let combined = format!("{WATER_CURSOR_WGSL}\n{BLIT_DISPLAY_APPEND}");
    let display_module = compile_wgsl(&device, &combined, "test-display-blit")
        .expect("creator+append must compile (production guard falls back to plain blit)");
    // Targets mirror the REAL swapchain format (sRGB): the display blit
    // re-encodes what it samples, exactly as in production.
    let display_pipeline = build_quad_pipeline_entry(
        &device,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &display_module,
        &group0,
        Some(&prev_layout),
        "fs_display",
    );

    // ---- Sim pipeline: creator's own fs_main ----
    let sim_module =
        compile_wgsl(&device, WATER_CURSOR_WGSL, "test-sim").expect("creator shader compiles");
    let sim_pipeline = build_quad_pipeline_entry(
        &device,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &sim_module,
        &group0,
        Some(&prev_layout),
        "fs_main",
    );

    let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-uniforms"),
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // ============ 1) display() bends the photo ============
    // Uniforms: mouse unknown, res = frame size.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let dummy_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-dummy"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let dummy_view = dummy_tex.create_view(&wgpu::TextureViewDescriptor::default());
    write_uniforms(&queue, &uniform_buf, [-1.0, -1.0]);
    let group0_m = bind_group0(
        &device,
        &group0,
        &uniform_buf,
        &photo_view,
        &sampler,
        &dummy_view,
    );
    let screen = render_to_pixels(&device, &queue, &display_pipeline, &group0_m, &prev_bg);
    // The screen must NOT be the raw sim encoding (a flat gray frame,
    // r≈0.5): the blit must have gone through display(), which samples
    // the PHOTO and returns a full color image. Photo and sim state are
    // told apart by channel spread: a photo has color variance, the sim
    // encoding (0.5, 0.5, 1) is flat.
    let center = sample3(&screen, W / 2, H / 2);
    let corner = sample3(&screen, 8, 8);
    let chan_range = |rgb: [u8; 3]| -> u8 {
        let (mn, mx) = rgb
            .iter()
            .fold((255u8, 0u8), |(a, b), v| (a.min(*v), b.max(*v)));
        mx - mn
    };
    assert!(
        chan_range(center) > 4 || chan_range(corner) > 4,
        "screen is a flat frame: display() was not called (raw sim state?)"
    );

    // ============ 2) the pointer drop injects energy ============
    // Mouse AT the center vs unknown: the sim output must differ at the
    // center (the Gaussian drop adds height where the cursor is).
    write_uniforms(&queue, &uniform_buf, [W as f32 / 2.0, H as f32 / 2.0]);
    let g = bind_group0(
        &device,
        &group0,
        &uniform_buf,
        &photo_view,
        &sampler,
        &dummy_view,
    );
    let dropped = render_to_pixels(&device, &queue, &sim_pipeline, &g, &prev_bg);
    write_uniforms(&queue, &uniform_buf, [-1.0, -1.0]);
    let g = bind_group0(
        &device,
        &group0,
        &uniform_buf,
        &photo_view,
        &sampler,
        &dummy_view,
    );
    let calm = render_to_pixels(&device, &queue, &sim_pipeline, &g, &prev_bg);

    let center = ((H / 2 * W + W / 2) * 4) as usize;
    let delta = (dropped[center] as i32 - calm[center] as i32).unsigned_abs();
    assert!(
        delta > 0,
        "the pointer drop had no effect on the sim output"
    );
}

fn sample3(data: &[u8], x: u32, y: u32) -> [u8; 3] {
    let o = ((y * W + x) * 4) as usize;
    [data[o], data[o + 1], data[o + 2]]
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(64),
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn create_texture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: &image::RgbaImage,
    (w, h): (u32, u32),
) -> wgpu::TextureView {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-photo"),
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
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
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
    tex.create_view(&wgpu::TextureViewDescriptor::default())
}

#[allow(clippy::too_many_arguments)]
fn bind_group0(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buf: &wgpu::Buffer,
    photo_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    dummy_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("test-group0-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(uniform_buf.as_entire_buffer_binding()),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(photo_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            // Slots 1..4 unused by the shader: 1×1 dummies, like the
            // production renderer fills them.
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(dummy_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(dummy_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(dummy_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn write_uniforms(queue: &wgpu::Queue, buf: &wgpu::Buffer, mouse: [f32; 2]) {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct U {
        time: f32,
        params0: f32,
        mouse: [f32; 2],
        params: [f32; 4],
        res: [f32; 2],
        clock: [f32; 3],
        pad: [f32; 3],
    }
    let u = U {
        time: 0.0,
        params0: 0.6,
        mouse,
        params: [0.6, 0.35, 0.0, 0.0],
        res: [W as f32, H as f32],
        clock: [12.0, 0.0, 0.0],
        pad: [0.0; 3],
    };
    queue.write_buffer(buf, 0, unsafe {
        std::slice::from_raw_parts(&u as *const U as *const u8, 64)
    });
}

/// Renders one pass to an offscreen texture and reads the pixels back.
fn render_to_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    group0: &wgpu::BindGroup,
    prev: &wgpu::BindGroup,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test-display-target"),
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
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("test-display-enc"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("test-display-pass"),
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
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group0, &[]);
        pass.set_bind_group(1, prev, &[]);
        pass.draw(0..4, 0..1);
    }
    let bytes_per_row = W * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test-display-readback"),
        size: (bytes_per_row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
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
        .expect("poll");
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    rx.recv().unwrap().expect("map");
    let data = slice.get_mapped_range().unwrap().to_vec();
    readback.unmap();
    data
}

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
