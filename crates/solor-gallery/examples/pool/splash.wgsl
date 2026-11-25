// A thin sheet rises from the displaced rim, breaks into fingers and collapses.
fn effect(uv: vec2<f32>, age: f32, size: vec2<f32>) -> vec4<f32> {
    let p = uv - vec2<f32>(0.5, 0.62);
    let t = clamp(age / 0.23, 0.0, 1.0);
    let lift = 4.0 * t * (1.0 - t);
    let radius = 0.115 + 0.17 * t;
    let side = p.x / radius;
    let fingers = 0.60 + 0.20 * sin(8.0 * side + 0.4) + 0.12 * cos(17.0 * side);
    let rise = lift * (0.085 + 0.13 * fingers);
    let ring = vec2<f32>(side, (p.y + rise) / (radius * 0.48));
    let distance = length(ring);
    let aa = 1.35 / max(1.0, size.x * radius * 0.48);
    let rim = 1.0 - smoothstep(0.025, 0.025 + aa, abs(distance - 1.0));
    let skirt = length(vec2<f32>(p.x / (radius * 0.88), p.y / (radius * 0.42)));
    let wall = smoothstep(1.0 - aa, 1.0 + aa, distance)
             * (1.0 - smoothstep(0.8, 1.07, skirt)) * smoothstep(-0.02, 0.04, -p.y);
    let sun = clamp(0.48 - 0.31 * side - 0.28 * ring.y, 0.0, 1.0);
    let color = mix(vec3<f32>(0.13, 0.43, 0.43), vec3<f32>(0.92, 0.97, 0.86), sun);
    let opacity = smoothstep(0.0, 0.12, t) * (1.0 - smoothstep(0.58, 1.0, t));
    return vec4<f32>(color, opacity * (rim * 0.76 + (1.0 - rim) * wall * 0.24));
}
