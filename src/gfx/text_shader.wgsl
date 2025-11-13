enable dual_source_blending;
// Vertex shader
struct SurfaceDimensionsUniform {
    width: u32,
    height: u32,
    scale_factor: f32
}

@group(0) @binding(0)
var<uniform> surface_dimensions: SurfaceDimensionsUniform;

struct GlyphVertexInput {
    @location(0) caret_position: vec2<i32>,
    @location(1) px_bounds_offset: vec2<i32>,
    @location(2) tex_coords: vec2<u32>
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>
}

fn to_clip_coords(px: vec2<i32>) -> vec2<f32> {
    return vec2<f32>(
        2.0 * f32(px.x) * surface_dimensions.scale_factor / f32(surface_dimensions.width),
        2.0 * f32(px.y) * surface_dimensions.scale_factor / f32(surface_dimensions.height)
    );
}

@vertex
fn vs_main(
    model: GlyphVertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vec2<f32>(model.tex_coords);
    out.clip_position = vec4<f32>(to_clip_coords(model.caret_position + model.px_bounds_offset), 0.0, 1.0);
    return out;
}

// Fragment shader
@group(1) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(1) @binding(1)
var s_diffuse: sampler;

struct FragmentOutput {
    @location(0) @blend_src(0) color : vec4f,
    @location(0) @blend_src(1) blend : vec4f,
}

@fragment
fn fs_main(
    in: VertexOutput,
) -> FragmentOutput {
    let dimensions = textureDimensions(t_diffuse);

    let x: f32 = f32(in.tex_coords.x) / f32(dimensions.x);
    let y: f32 = f32(in.tex_coords.y) / f32(dimensions.y);

    let tex_coords = vec2<f32>(x, y);

    var output : FragmentOutput;
    // text color
    output.color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    // subpixel rgb mask
    output.blend = textureSample(t_diffuse, s_diffuse, tex_coords);
    return output;
}