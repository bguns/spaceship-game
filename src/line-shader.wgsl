// Vertex shader
struct SurfaceDimensionsUniform {
    px: vec2<f32>;
};

[[group(0), binding(0)]]
var<uniform> surface_dimensions: SurfaceDimensionsUniform;

struct LineVertexInput {
    [[location(0)]] position: vec3<f32>;
    [[location(1)]] previous_point: vec3<f32>;
    [[location(2)]] next_point: vec3<f32>;
    [[location(3)]] miter_dir: f32;
    [[location(4)]] thickness: f32;
};

struct LineVertexOutput {
    [[builtin(position)]] clip_position: vec4<f32>;
};


[[stage(vertex)]]
fn vs_main(
    model: LineVertexInput,
) -> LineVertexOutput {
    let no_previous = f32(i32(model.previous_point.x < -1.0));
    let no_next = f32(i32(model.next_point.x > 1.0));

    let prev = (1.0 - no_previous) * model.previous_point + no_previous * (model.position + (model.position - model.next_point));
    let next = (1.0 - no_next) * model.next_point + no_next * (model.position + (model.position - model.previous_point));

    // see https://blog.scottlogic.com/2019/11/18/drawing-lines-with-webgl.html

    let prev_this = normalize(normalize(model.position.xy - prev.xy) * surface_dimensions.px);
    let this_next = normalize(normalize(next.xy - model.position.xy) * surface_dimensions.px);

    let tangent = normalize(prev_this + this_next);

    let miter = vec2<f32>(-tangent.y, tangent.x);
    let normalA = vec2<f32>(-prev_this.y, prev_this.x);

    let miter_length = 1.0 / dot(miter, normalA);

    let out_pos = model.position.xy + (model.miter_dir * miter * model.thickness * miter_length) / surface_dimensions.px;

    var out: LineVertexOutput;
    out.clip_position = vec4<f32>(out_pos, 0.0, 1.0);

    return out;
}

// Fragment shader

[[stage(fragment)]]
fn fs_main(
    in: LineVertexOutput,
) -> [[location(0)]] vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}