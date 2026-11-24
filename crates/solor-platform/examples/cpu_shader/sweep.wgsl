fn effect(uv: vec2<f32>, position: f32) -> vec4<f32> {
    let distance = abs(uv.x + (uv.y - 0.5) * 0.12 - position);
    let light = 1.0 - smoothstep(0.0, 0.16, distance);
    let base = vec3<f32>(0.075, 0.12, 0.22);
    let glow = vec3<f32>(0.58, 0.90, 0.84);
    return vec4<f32>(mix(base, glow, light), 1.0);
}
