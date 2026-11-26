@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    let p = v.pos.xy / q.viewport.z;
    let local = p - q.rect.xy;
    if solor_outside_clip(v.pos.xy, q) { discard; }
    let coverage = solor_box_coverage(local, q);
    let color = clamp(solor_paint(local, q.rect.zw, q),vec4f(0),vec4f(1));
    return vec4f(color.rgb, color.a * coverage * solor_clip_coverage(v.pos.xy, v.clip));
}
