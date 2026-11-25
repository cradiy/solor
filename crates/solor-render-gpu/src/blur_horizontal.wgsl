fn read_sample(p: vec2i) -> vec4f {
    return textureLoad(solor_bitmap, p, 0);
}

@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec2u {
    let q = solor_quads[v.index];
    let p = vec2i(v.pos.xy) + vec2i(q.params.xy);
    let filtered = box_filter(p, vec2i(1, 0), i32(textureDimensions(solor_bitmap).x), q.params.z);
    // Keep the original premultiplied pixel for fractional coverage at the mask edge.
    return vec2u(pack4x8unorm(filtered), pack4x8unorm(read_sample(p)));
}
