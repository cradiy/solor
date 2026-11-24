fn effect(uv: vec2<f32>, position: f32, size: vec2<f32>) -> vec4<f32> {
    let center = vec2<f32>(position, 0.5);
    let radius = distance(uv * size, center * size);
    let halo = 1.0 - smoothstep(0.0, 96.0, radius);
    let core = 1.0 - smoothstep(0.0, 36.0, radius);
    let night = vec3<f32>(0.055, 0.080, 0.140);
    let blue = vec3<f32>(0.20, 0.51, 0.78);
    let pearl = vec3<f32>(0.87, 0.97, 0.92);
    return vec4<f32>(mix(mix(night, blue, halo), pearl, core), 1.0);
}
