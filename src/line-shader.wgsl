// Vertex shader

struct LineVertexInput {
    [[location(0)]] line_start: vec3<f32>;
    [[location(1)]] line_end: vec3<f32>;
    [[location(2)]] thickness: f32;
};

struct LineVertexOutput {
    [[builtin(position)]] clip_position: vec4<f32>;
    [[location(0)]] line_start: vec3<f32>;
    [[location(1)]] line_end: vec3<f32>;
    [[location(2)]] line_normalized: vec3<f32>;
    [[location(3)]] thickness: f32;
};


[[stage(vertex)]]
fn vs_main(
    model: LineVertexInput,
) -> LineVertexOutput {
    var out: LineVertexOutput;
    out.clip_position = vec4<f32>(model.line_start, 1.0);
    out.line_start = model.line_start;
    out.line_end = model.line_end;
    out.thickness = model.thickness;
    out.line_normalized = normalize(model.line_end - model.line_start);
    return out;
}

// Fragment shader
fn distance_squared_from_line(line_start: vec2<f32>, line_normalized: vec2<f32>, point: vec2<f32>) -> f32 {
    let proj = line_start + line_normalized * dot(point - line_start, line_normalized);
    return proj.x * proj.x + proj.y * proj.y;
}

struct SurfaceDimensionsUniform {
    px: vec2<f32>;
};

[[group(0), binding(0)]]
var<uniform> surface_dimensions: SurfaceDimensionsUniform;


[[stage(fragment)]]
fn fs_main(
    in: LineVertexOutput,
) -> [[location(0)]] vec4<f32> {
    //var uv = in.clip_position.xy / surface_dimensions.px;
    //uv.x = (uv.x - 0.5) * 2.0;
    //uv.y = ((1.0 - uv.y) - 0.5) * 2.0;
    //let c = 1.0 * f32(distance_squared_from_line(in.line_start.xy, in.line_normalized.xy, uv) < in.thickness * in.thickness);
    let c = 1.0 * f32(distance_squared_from_line(vec2<f32>(0.0, 0.0), normalize(vec2<f32>(200.0,200.0) - vec2<f32>(0.0,0.0)), in.clip_position.xy) < 10.0);
    
    return vec4<f32>(0.0, c, c, 1.0);
}