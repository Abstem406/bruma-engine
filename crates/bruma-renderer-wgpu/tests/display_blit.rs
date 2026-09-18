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
    BLIT_DISPLAY_APPEND, BLIT_WGSL, SIM_FORMAT, build_quad_pipeline_entry, compile_wgsl,
    prev_frame_bind_group_layout,
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
    // fp16 calm state (h = 0.5, v = 0.5): EXACTLY what a fresh sim
    // target looks like (fp16 0.5 = 0x3800, 1.0 = 0x3C00).
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
        format: SIM_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let calm_texels: Vec<u8> =
        [0x00u8, 0x38, 0x00, 0x38, 0x00, 0x38, 0x00, 0x3C].repeat((W * H) as usize);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &init,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &calm_texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(W * 8),
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

    // ---- Sim pipeline: creator's own fs_main, into the fp16 offscreen
    // format (exactly what the production renderer does) ----
    let sim_module =
        compile_wgsl(&device, WATER_CURSOR_WGSL, "test-sim").expect("creator shader compiles");
    let sim_pipeline = build_quad_pipeline_entry(
        &device,
        SIM_FORMAT,
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
    let screen = render_to_pixels(
        &device,
        &queue,
        &display_pipeline,
        &group0_m,
        &prev_bg,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    // The screen must show the PHOTO (not the flat sim encoding, and not
    // black): with a calm state the refraction is ~0, so a colorful photo
    // pixel must land at the same screen coordinate, channels preserved.
    // (The lake's center is gray water and the corner is gray sky — a
    // variance probe THERE is meaningless; find a genuinely colorful
    // pixel in the photo instead.)
    let (px, py) = {
        let mut best = (0u32, 0u32, 0i32);
        let raw = photo.as_raw();
        let pw = photo.dimensions().0;
        for y in (0..photo.dimensions().1).step_by(7) {
            for x in (0..pw).step_by(7) {
                let o = ((y * pw + x) * 4) as usize;
                let range = [raw[o], raw[o + 1], raw[o + 2]]
                    .iter()
                    .fold((255i32, 0i32), |(a, b), &v| {
                        (a.min(v as i32), b.max(v as i32))
                    });
                if range.1 - range.0 > best.2 {
                    best = (x * W / pw, y * H / photo.dimensions().1, range.1 - range.0);
                }
            }
        }
        (best.0, best.1)
    };
    let got = sample3(&screen, px.max(2), py.max(2));
    let chan_range = |rgb: [u8; 3]| -> u8 {
        let (mn, mx) = rgb
            .iter()
            .fold((255u8, 0u8), |(a, b), v| (a.min(*v), b.max(*v)));
        mx - mn
    };
    assert!(
        chan_range(got) > 15,
        "screen at the photo's most colorful pixel ({px},{py}) is flat {got:?}: display() was not called"
    );

    // ============ 2) the pointer drop injects energy WHERE THE CURSOR IS
    // (not mirrored): cursor at (W/4, 3H/4) vs unknown.
    write_uniforms(&queue, &uniform_buf, [W as f32 / 4.0, 3.0 * H as f32 / 4.0]);
    let g = bind_group0(
        &device,
        &group0,
        &uniform_buf,
        &photo_view,
        &sampler,
        &dummy_view,
    );
    let dropped = render_to_pixels(&device, &queue, &sim_pipeline, &g, &prev_bg, SIM_FORMAT);
    write_uniforms(&queue, &uniform_buf, [-1.0, -1.0]);
    let g = bind_group0(
        &device,
        &group0,
        &uniform_buf,
        &photo_view,
        &sampler,
        &dummy_view,
    );
    let calm = render_to_pixels(&device, &queue, &sim_pipeline, &g, &prev_bg, SIM_FORMAT);

    // The state is fp16: compare height halves (u16). The Gaussian drop
    // at the cursor moves h by ~0.03 (~44 fp16 ULPs around 0.5); the
    // ambient shimmer alone stays within ±20.
    let h_at = |data: &[u8], x: u32, y: u32| -> i32 {
        let o = ((y * W + x) * 8) as usize;
        i32::from(u16::from_le_bytes([data[o], data[o + 1]]))
    };
    let drop_here = h_at(&dropped, W / 4, 3 * H / 4) - h_at(&calm, W / 4, 3 * H / 4);
    let drop_mirror = h_at(&dropped, W / 4, H / 4) - h_at(&calm, W / 4, H / 4);
    assert!(
        drop_here.abs() > 36,
        "the ripple did not land under the cursor (delta {drop_here})"
    );
    assert!(
        drop_mirror.abs() <= 30,
        "the ripple landed mirrored into the opposite half (delta {drop_mirror})"
    );
}

/// Orientation contract, pinned offline: the internal blits render the
/// frame UPRIGHT — the creator shaders' vertices flip uv.y (`1 - y`),
/// so the blits must do the same. Regression for the upside-down water
/// demo (blit without the flip vs. creators with it).
#[test]
fn blit_preserves_orientation() {
    let Some((device, queue)) = offline_device() else {
        return;
    };

    // A half-red / half-green source in group 1 (red = row 0).
    let mut frame = image::RgbaImage::new(W, H);
    for y in 0..H / 2 {
        for x in 0..W {
            frame.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
        }
    }
    for y in H / 2..H {
        for x in 0..W {
            frame.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
        }
    }
    let view = create_texture_view(&device, &queue, &frame, (W, H));
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let prev_layout = prev_frame_bind_group_layout(&device);
    let prev_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("orient-prev"),
        layout: &prev_layout,
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

    // Group 0: uniform + dummies (the blits declare it but don't use it).
    let (bgl0, bg0, _buf) = {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("orient-g0"),
            entries: &[uniform_entry()],
        });
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("orient-uniforms"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        write_uniforms(&queue, &buf, [-1.0, -1.0]);
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orient-g0-bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(buf.as_entire_buffer_binding()),
            }],
        });
        (layout, bg, buf)
    };

    let module = compile_wgsl(&device, BLIT_WGSL, "orient-blit").expect("blit compiles");
    let pipeline = build_quad_pipeline_entry(
        &device,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        &module,
        &bgl0,
        Some(&prev_layout),
        "fs_main",
    );
    let screen = render_to_pixels(
        &device,
        &queue,
        &pipeline,
        &bg0,
        &prev_bg,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );

    let top = sample3(&screen, W / 2, 4);
    let bottom = sample3(&screen, W / 2, H - 5);
    assert!(
        top[0] > 200 && top[1] < 60,
        "top row must stay the source's top (red), got {top:?}"
    );
    assert!(
        bottom[1] > 200 && bottom[0] < 60,
        "bottom row must stay the source's bottom (green), got {bottom:?}"
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
    format: wgpu::TextureFormat,
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
        format,
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
    let bytes_per_row = W * format
        .block_copy_size(Some(wgpu::TextureAspect::All))
        .unwrap();
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
