fn paint(local: vec2f, size: vec2f, q: Quad) -> vec4f {
    let fitted_start = q.params.xy;
    let fitted_size = q.params.zw;
    let half_pixel = vec2f(0.5 / q.viewport.z);
    let lo = max(local - half_pixel, vec2f(0.0));
    let hi = min(local + half_pixel, size);
    let overlap = clamp((min(hi, fitted_start + fitted_size) - max(lo, fitted_start)) /
                        max(hi - lo, vec2f(1e-8)), vec2f(0.0), vec2f(1.0));
    let source = q.ramp.xy + (local - fitted_start) / fitted_size * (q.ramp.zw - q.ramp.xy);
    // Restrict the filter footprint to the selected sprite, including when it is magnified.
    let texel = clamp(source, q.ramp.xy + 0.5, q.ramp.zw - 0.5);
    var color: vec4f;
    if q.viewport.w > 0.0 {
        color = textureLoad(bitmap, vec2i(floor(texel)), 0);
    } else {
        color = textureSampleLevel(bitmap, bitmap_sampler, texel / vec2f(textureDimensions(bitmap)), 0.0);
    }
    return vec4f(color.rgb / max(color.a, 1e-8), color.a * overlap.x * overlap.y);
}
