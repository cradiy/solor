// Horizontal and vertical derivatives of crossing, phase-modulated wind waves.
fn effect(uv: vec2<f32>, time: f32, size: vec2<f32>) -> vec4<f32> {
    let p = uv * size;
    let bend = dot(p, vec2<f32>(0.013, -0.009)) + 0.37 * time;
    let swell = dot(p, vec2<f32>(0.018, 0.026)) - 0.79 * time;
    let a = dot(p, vec2<f32>(0.059, 0.031)) - 1.48 * time + 1.34 * sin(bend);
    let b = dot(p, vec2<f32>(-0.037, 0.076)) + 1.67 * time + 0.8 * sin(swell);
    let c = dot(p, vec2<f32>(0.093, -0.064)) - 2.02 * time + 0.75 * sin(a);
    let da = vec2<f32>(0.059, 0.031) + 1.34 * cos(bend) * vec2<f32>(0.013, -0.009);
    let db = vec2<f32>(-0.037, 0.076) + 0.8 * cos(swell) * vec2<f32>(0.018, 0.026);
    let dc = vec2<f32>(0.093, -0.064) + 0.75 * cos(a) * da;
    let slope = 1.26 * da * cos(a) + 0.95 * db * cos(b) + 0.41 * dc * cos(c);
    // The open sky is visible between the shaded bank and the reflected foliage.
    let reflection = uv + slope * vec2<f32>(0.81, 1.08);
    let skylight = smoothstep(-0.14, 0.24, reflection.x + 0.47*reflection.y - 0.21)
                 * (1.0 - smoothstep(0.78, 1.19, reflection.x - 0.28*reflection.y));
    return vec4<f32>(vec2<f32>(0.5) + slope * 1.9, skylight, 1.0);
}
