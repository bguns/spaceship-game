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
        model.px_bounds_offset.x / surface_dimensions.px.x, 
        model.px_bounds_offset.y / surface_dimensions.px.y,
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
    //return vec4<f32>(0.8, 0.8, 0.8, textureSample(t_diffuse, s_diffuse, in.tex_coords).r);
    /*let s = textureSample(t_diffuse, s_diffuse, in.tex_coords).rgb;
    if (s.r < 0.1 && s.g < 0.1 && s.b < 0.1) {
            return vec4<f32>(s, 0.0);
    }
    return vec4<f32>(s, 1.0);*/

    var output : FragmentOutput;
    output.color = vec4f(0.3, 0.8, 0.0, 1.0);
    let subpixel_mask = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    //let subpixel_alpha = subpixel_mask.r + subpixel_mask.g + subpixel_mask.b;
    output.blend = vec4f(subpixel_mask.rgb, 1.0);
    return output;
}