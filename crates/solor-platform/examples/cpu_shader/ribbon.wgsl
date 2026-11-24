fn effect(uv: vec2<f32>, position: f32) -> vec4<f32> {
    let ridge = 1.0 - smoothstep(0.0, 0.24, abs(uv.x - position));
    let base = vec3<f32>(0.10, 0.12, 0.23);
    let iris = vec3<f32>(0.47, 0.38, 0.80);
    let pearl = vec3<f32>(0.96, 0.84, 0.78);
    let color = mix(base, iris, ridge);
    return vec4<f32>(mix(color, pearl, ridge * ridge * ridge), 1.0);
}
