fn effect(uv: vec2<f32>, params: vec4<f32>, size: vec2<f32>) -> vec4<f32> {
    let kind = params.x;
    let lift = params.y;
    let local = (uv - vec2<f32>(0.5)) * 2.0;
    let q = vec2<f32>(local.x + 0.22 * kind * local.y, local.y - 0.27 * kind * local.x);
    let shape = vec2<f32>(q.x + 0.27 * q.y, q.y - 0.18 * q.x) * vec2<f32>(1.40, 1.94)
              * (1.0 + 0.085 * q.x - 0.14 * q.y + 0.035 * sin(6.0 * q.y));
    let r2 = dot(shape, shape);
    let aa = 4.0 / max(size.x, 1.0);
    let coverage = 1.0 - smoothstep(1.0 - aa, 1.0 + aa, r2);
    let n = vec3<f32>(shape, sqrt(max(0.0, 1.0 - r2)));
    let light = clamp(dot(n, vec3<f32>(-0.46, -0.56, 0.69)), 0.0, 1.0);
    let vein = abs(q.y + q.x * 0.40 + 0.07 * sin(q.x * 8.0 + kind * 3.0) - 0.06);
    let fleck = sin(q.x * 93.0 + q.y * 38.0) * sin(q.y * 67.0 - q.x * 54.0);
    let grain = vec3<f32>(0.12, 0.105, 0.080) * (1.0 - smoothstep(0.009, 0.04, vein));
    let mineral = sin(25.0*q.y + 11.0*q.x + 2.7*sin(4.6*q.x + kind) + 0.5*sin(12.7*q.y));
    let crystals = fract(sin(dot(floor(q*74.0),vec2<f32>(17.3,21.1)))*2971.3);
    let body = mix(vec3<f32>(0.39, 0.42, 0.40), vec3<f32>(0.72, 0.54, 0.36), kind);
    let stone = body * (0.48 + light * 0.56 + fleck * 0.052 + mineral * 0.048 + crystals * 0.075) + grain;
    let shadow_p = (local - vec2<f32>(0.10, 0.12) * (1.0 + lift)) * vec2<f32>(1.4, 2.0);
    let shadow = (1.0 - smoothstep(0.33, 1.27 + 0.22 * lift, length(shadow_p))) * 0.39;
    let alpha = coverage + (1.0 - coverage) * shadow;
    return vec4<f32>(stone * coverage / max(alpha, 0.0001), alpha);
}
