fn effect(uv: vec2<f32>, params: f32, size: vec2<f32>) -> vec4<f32> {
    let p = uv * size;
    let aggregate = fract(sin(dot(floor(p * 0.7), vec2<f32>(17.17, 9.31))) * 235.17);
    let stratum = sin(0.019 * p.x + 0.094 * p.y + 1.4 * sin(p.x * 0.03));
    let joint = abs(fract((p.x + 27.0) / 186.0 + 0.5) - 0.5) * 186.0;
    let shade = 0.88 + 0.06 * aggregate + 0.024 * stratum + 0.04 * smoothstep(0.2, 1.25, joint);
    let lip = (1.0 - smoothstep(1.0, 3.8, p.y)) * 0.14;
    return vec4<f32>(vec3<f32>(0.89, 0.86, 0.78) * (shade - lip), 1.0);
}
