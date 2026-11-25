fn studio(p: vec2<f32>, offset: f32) -> vec3<f32> {
    let band = p.y + 0.32 * p.x - 0.30 * sin(1.4 * p.x + offset) - offset * 0.22;
    let halo = 1.0 / (1.0 + 12.0 * band * band);
    let line = 1.0 / (1.0 + 1200.0 * band * band);
    let warmth = smoothstep(-0.9, 1.0, p.x + offset);
    let hue = mix(vec3<f32>(0.12, 0.49, 0.59), vec3<f32>(0.98, 0.52, 0.28), warmth);
    return vec3<f32>(0.035, 0.055, 0.085) + hue * (0.38 * halo + 0.55 * line);
}

fn effect(uv: vec2<f32>, position: f32, size: vec2<f32>) -> vec4<f32> {
    let radius = max(1.0, min(size.x, size.y) * 0.38);
    let p = (uv - vec2<f32>(0.5)) * size / radius;
    let sweep = position * 2.0 - 1.0;
    let r2 = dot(p, p);
    let z = sqrt(max(0.0, 1.0 - r2));
    let backdrop = studio(p, sweep) * 0.52;
    // Sample the same light strip at the coordinates displaced by the lens.
    let refracted = studio((0.87 - 2.35 * z) * p + vec2<f32>(0.15, -0.22) * z, sweep);

    let n = vec3<f32>(p, z);
    let reflected = 2.0 * z * n - vec3<f32>(0.0, 0.0, 1.0);
    let softbox = normalize(vec3<f32>(-0.66 + 0.30 * sweep, -0.75, -0.23));
    let arc = dot(reflected, softbox);
    let strip = reflected.y + 0.66 - 0.16 * reflected.x;
    let silk = smoothstep(0.4, 0.94, arc) / (1.0 + 56.0 * strip * strip);
    let rim = 1.0 - z;
    let glass = refracted + vec3<f32>(0.56, 0.65, 0.71) * silk
                          + vec3<f32>(0.20, 0.32, 0.37) * rim * rim;
    let edge = 1.0 / radius;
    let coverage = 1.0 - smoothstep(1.0 - edge, 1.0 + edge, r2);
    return vec4<f32>(mix(backdrop, glass, coverage), 1.0);
}
