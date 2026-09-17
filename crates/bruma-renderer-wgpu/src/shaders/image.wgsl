// Fullscreen image quad — Phase 2, step 2.
//
// The quad covers the whole viewport (clip-space positions generated from
// vertex_index) and the UVs reach the fragment shader to sample the image
// texture. The image is STRETCHED to the screen size; cover/contain will
// come with the .wallpaper manifest (pending decision).

struct VsOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VsOutput {
    // 4 corners in clip space; the order forms two triangles:
    // (0,1,2) and (2,1,3).
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0,  1.0), // top-left
        vec2<f32>( 1.0,  1.0), // top-right
        vec2<f32>(-1.0, -1.0), // bottom-left
        vec2<f32>( 1.0, -1.0), // bottom-right
    );
    let uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );

    var out: VsOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = uvs[idx];
    return out;
}

@group(0) @binding(0)
var tex: texture_2d<f32>;

@group(0) @binding(1)
var samp: sampler;

@fragment
fn fs_main(in: VsOutput) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
