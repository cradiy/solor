fn effect(uv: vec2<f32>, position: f32, size: vec2<f32>) -> vec4<f32> {
    let local = (uv - vec2<f32>(position, 0.5)) * size;
    let half_size = vec2<f32>(72.0, 44.0);
    let radius = 20.0;
    let q = abs(local) - half_size + vec2<f32>(radius);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;

    let night = vec3<f32>(0.045, 0.082, 0.10);
    let jade = vec3<f32>(0.11, 0.54, 0.47);
    let mint = vec3<f32>(0.66, 0.96, 0.77);
    let depth = clamp(-d / 24.0, 0.0, 1.0);
    let body = mix(jade, jade * 0.45, depth);
    let fill = mix(night, body, 1.0 - smoothstep(-0.5, 0.5, d));
    let rim = 1.0 - smoothstep(0.5, 1.5, abs(d));
    return vec4<f32>(mix(fill, mint, rim), 1.0);
}
