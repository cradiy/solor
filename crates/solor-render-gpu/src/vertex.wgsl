struct solor_Quad {
    rect: vec4f,
    radii: vec4f,
    clip: vec4f,
    params: vec4f,
    viewport: vec4f,
    ramp: vec4f,
}
struct solor_Stop { color: vec4f, segment: vec4f }
@group(0) @binding(0) var<storage, read> solor_quads: array<solor_Quad>;
@group(0) @binding(1) var<storage, read> solor_stops: array<solor_Stop>;
@group(1) @binding(0) var solor_bitmap: texture_2d<f32>;
@group(1) @binding(1) var solor_bitmap_sampler: sampler;
struct solor_Vertex {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) index: u32,
    @location(1) @interpolate(flat) clip: u32,
}
@vertex fn solor_vertex(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> solor_Vertex {
    let corners = array(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let q = solor_quads[i];
    let p = q.rect.xy*q.viewport.z - 1.0 + corners[v % 6u]*(q.rect.zw*q.viewport.z + 2.0);
    return solor_Vertex(vec4f(p / q.viewport.xy * vec2f(2,-2) + vec2f(-1,1), 0, 1), i, v / 6u);
}

fn solor_outside_clip(point: vec2f, q: solor_Quad) -> bool {
    let clip = floor(q.clip * q.viewport.z + 0.5);
    return any(point < clip.xy) || any(point >= clip.zw);
}
