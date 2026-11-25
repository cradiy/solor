@fragment fn solor_fragment(v: solor_Vertex) -> @location(0) vec4f {
    let q = solor_quads[v.index];
    var pixel = textureLoad(solor_bitmap, vec2i(v.pos.xy), 0);
    if q.params.x > 0. && pixel.a > 0. {
        let luma = clamp(dot(pixel.rgb, vec3f(0.2126, 0.7152, 0.0722)) / pixel.a, 0., 1.);
        let mapped = sample_gradient_ramp(luma, q.ramp, q.params.z, q.params.w != 0., q.params.y != 0.);
        pixel = mix(pixel, vec4f(mapped.rgb, 1.) * (mapped.a * pixel.a), q.params.x);
    }
    return pixel * q.viewport.w;
}
