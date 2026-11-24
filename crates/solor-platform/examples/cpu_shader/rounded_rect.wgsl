fn rounded_edge(local: vec2<f32>, half_size: vec2<f32>, radius: f32) -> vec2<f32> {
    let q = abs(local) - half_size + vec2<f32>(radius);
    let corner = max(q, vec2<f32>(0.0));
    let dist = length(corner);
    return vec2<f32>(dist + min(max(q.x, q.y), 0.0) - radius, corner.x / max(dist, 0.001));
}

fn effect(uv: vec2<f32>, params: vec4<f32>, size: vec2<f32>) -> vec4<f32> {
    let local = (uv - vec2<f32>(params.x, 0.5)) * size;
    let edge = rounded_edge(local, vec2<f32>(72.0, 44.0), 20.0);
    let d = edge.x;
    // Project this frame's horizontal travel onto the contour normal.
    let width = 0.5 + min(abs(params.y) * size.x * 0.5, 2.0) * edge.y;

    let night = vec3<f32>(0.045, 0.082, 0.10);
    let jade = vec3<f32>(0.11, 0.54, 0.47);
    let mint = vec3<f32>(0.66, 0.96, 0.77);
    let depth = clamp(-d / 24.0, 0.0, 1.0);
    let body = mix(jade, jade * 0.45, depth);
    let fill = mix(night, body, 1.0 - smoothstep(-width, width, d));
    let rim = smoothstep(-width, width, d + 1.0) - smoothstep(-width, width, d - 1.0);
    return vec4<f32>(mix(fill, mint, rim), 1.0);
}
