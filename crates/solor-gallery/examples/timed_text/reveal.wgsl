fn smootherstep(value: f32) -> f32 {
    let t = clamp(value, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn effect(uv: vec2<f32>, params: vec4<f32>, size: vec2<f32>) -> vec4<f32> {
    let x = uv.x * size.x;
    let reveal = params.x;
    let completed = params.y;
    let trail = mix(1.0, 0.18, smootherstep((x - completed) / max(reveal - completed, 0.0001)));
    let front = 1.0 - smoothstep(reveal - 7.0, reveal + 7.0, x);
    return vec4<f32>(vec3<f32>(103.0, 199.0, 233.0) / 255.0, trail * front);
}
