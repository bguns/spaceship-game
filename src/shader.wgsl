enable dual_source_blending;
// Vertex shader
struct SurfaceDimensionsUniform {
    px: vec2<f32>
}

@group(0) @binding(0)
var<uniform> surface_dimensions: SurfaceDimensionsUniform;

struct GlyphVertexInput {
    @location(0) caret_position: vec3<f32>,
    @location(1) px_bounds_offset: vec2<f32>,
    @location(2) tex_coords: vec2<f32>
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>
}

@vertex
fn vs_main(
    model: GlyphVertexInput,
) -> VertexOutput {
    let offset = vec3<f32>(
        2.0 * model.px_bounds_offset.x / surface_dimensions.px.x, 
        2.0 * model.px_bounds_offset.y / surface_dimensions.px.y,
        0.0
    );

    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    out.clip_position = vec4<f32>(model.caret_position + offset, 1.0);
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

    let x: f32 = (in.tex_coords.x) / f32(dimensions.x);
    let y: f32 = (in.tex_coords.y) / f32(dimensions.y);

    let tex_coords = vec2<f32>(x, y);

    var output : FragmentOutput;
    // text color
    output.color = vec4<f32>(0.88, 0.556, 0.07, 1.0);
    //output.color = vec4<f32>(0.0, 1.0, 0.0, 1.0);
    // subpixel rgb mask
    output.blend = textureSample(t_diffuse, s_diffuse, tex_coords);
    return output;
}