@group(1) @binding(2) var rows: texture_2d<u32>;

fn read_sample(p: vec2i) -> vec4f {
    return unpack4x8unorm(textureLoad(rows, p, 0).x);
}

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    if any(v.pos.xy < q.clip.xy) || any(v.pos.xy >= q.clip.zw) { discard; }
    let p = vec2i(v.pos.xy) - vec2i(q.params.xy);
    let original = unpack4x8unorm(textureLoad(rows, p, 0).y);
    let filtered = box_filter(p, vec2i(0, 1), i32(q.params.w), q.params.z);
    let coverage = solor_box_coverage(v.pos.xy / q.viewport.z - q.rect.xy, q);
    return mix(original, filtered, coverage * solor_clip_coverage(v.pos.xy, v.clip));
}
