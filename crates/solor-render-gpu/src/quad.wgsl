struct Quad {
    rect: vec4f,
    radii: vec4f,
    clip: vec4f,
    params: vec4f,
    viewport: vec4f,
    ramp: vec4f,
}
struct Stop { color: vec4f, segment: vec4f }
@group(0) @binding(0) var<storage, read> quads: array<Quad>;
@group(0) @binding(1) var<storage, read> stops: array<Stop>;
@group(1) @binding(0) var bitmap: texture_2d<f32>;
@group(1) @binding(1) var bitmap_sampler: sampler;
struct Vertex {
    @builtin(position) pos: vec4f,
    @location(0) @interpolate(flat) index: u32,
}
@vertex fn vertex(@builtin(vertex_index) v: u32, @builtin(instance_index) i: u32) -> Vertex {
    let corners = array(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let q = quads[i];
    let p = q.rect.xy*q.viewport.z - 1.0 + corners[v]*(q.rect.zw*q.viewport.z + 2.0);
    return Vertex(vec4f(p / q.viewport.xy * vec2f(2,-2) + vec2f(-1,1), 0, 1), i);
}
// PAINT
@fragment fn fragment(v: Vertex) -> @location(0) vec4f {
    let q = quads[v.index];
    let p = v.pos.xy / q.viewport.z;
    let local = p - q.rect.xy;
    let far = q.rect.zw - local;
    let corners = array(local,vec2f(far.x,local.y),far,vec2f(local.x,far.y));
    var dist = min(min(local.x,local.y), min(far.x,far.y));
    for (var i=0u; i<4u; i++) {
        let r = q.radii[i];
        let c = corners[i];
        if all(c < vec2f(r)) { dist = min(dist, r - length(vec2f(r)-c)); }
    }
    let clip = floor(q.clip * q.viewport.z + 0.5);
    if any(v.pos.xy < clip.xy) || any(v.pos.xy >= clip.zw) { discard; }
    let pixel_min = local * q.viewport.z - 0.5;
    let pixel_max = pixel_min + 1.0;
    let overlap = clamp(min(pixel_max, q.rect.zw * q.viewport.z) - max(pixel_min, vec2f(0.0)),
                        vec2f(0.0), vec2f(1.0));
    let coverage = min(overlap.x * overlap.y, clamp(dist * q.viewport.z + 0.5, 0., 1.));
    let color = clamp(paint(local, q.rect.zw, q),vec4f(0),vec4f(1));
    return vec4f(color.rgb, color.a * coverage);
}
