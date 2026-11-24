fn effect(uv: vec2<f32>, phase: f32) -> vec4<f32> {
    let angle = phase * 6.2831853;
    let point = (uv - vec2<f32>(0.5)) * vec2<f32>(10.0, 6.0);
    let waves = sin(point + cos(point.yx - vec2<f32>(angle)) + vec2<f32>(angle));
    let light = 0.5 + 0.5 * cos(waves.x + waves.y + angle);
    let color = mix(vec3<f32>(0.08, 0.13, 0.30), vec3<f32>(0.33, 0.88, 0.78), light);
    return vec4<f32>(color, 1.0);
}
