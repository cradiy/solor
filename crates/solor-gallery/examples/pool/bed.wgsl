fn palm_shadow(p: vec2<f32>, size: vec2<f32>) -> f32 {
    let span = min(size.x * 0.57, size.y * 0.91);
    let local = (p - vec2<f32>(size.x + 16.0, -20.0)) / max(span, 1.0);
    let along = dot(local, vec2<f32>(-0.80, 0.60));
    let side = dot(local, vec2<f32>(0.60, 0.80)) - 0.11 * along * along;
    let root = along - abs(side) * (0.85 + abs(side)) + 0.018 * sin(15.0 * side);
    let fan = sin(3.14159 * clamp(root, 0.0, 1.0));
    let tip = smoothstep(-0.014, 0.024, 0.23 * fan - abs(side));
    let spacing = abs(fract(root * 14.0 + 0.5) - 0.5);
    let leaflet = tip * (1.0 - smoothstep(0.04, 0.36, spacing));
    let spine = (1.0 - smoothstep(0.002, 0.014, abs(side))) * smoothstep(0.0, 0.07, 1.0 - along);
    return max(leaflet, spine) * smoothstep(0.0, 0.03, along);
}

fn effect(uv: vec2<f32>, params: f32, size: vec2<f32>) -> vec4<f32> {
    let p = uv * size;
    let shelf = min(size.x * 0.22, 98.0);
    let edge = p.x - shelf;
    let step = smoothstep(-0.8, 1.0, edge);
    let deep = smoothstep(0.0, 1.0, dot(uv, vec2<f32>(0.68, 0.42)));
    let riser = smoothstep(0.0, 8.0, edge);
    let tile = (vec2<f32>(p.x + 0.045 * p.y, p.y) + vec2<f32>(10.0, 4.0)*riser) / vec2<f32>(32.0, 29.0);
    let seam = abs(fract(tile + vec2<f32>(0.5)) - vec2<f32>(0.5)) * vec2<f32>(32.0, 29.0);
    let grout = 1.0 - smoothstep(0.15, 1.0, min(seam.x, seam.y));
    let glaze = fract(sin(dot(floor(tile),vec2<f32>(19.13,47.33))) * 1289.1);
    let grain = sin(dot(p, vec2<f32>(0.61,0.49))) * sin(p.y * 0.74 - p.x * 0.36);
    let stone = sin(p.x*0.039 + 0.6*sin(p.y*0.13)) * sin(p.y*0.023+p.x*0.017);
    let abyss = mix(vec3<f32>(0.30,0.73,0.68), vec3<f32>(0.09,0.48,0.53), deep);
    let travertine = vec3<f32>(0.72,0.82,0.69) * (0.97 + 0.04*stone);
    let floor_color = mix(travertine, abyss, step);
    let leaf = 0.7 * palm_shadow(p, size) + 0.3 * palm_shadow(p + vec2<f32>(3.0, 2.0), size);
    let occlusion = 0.18 * step * (1.0 - smoothstep(0.0, 15.0, edge));
    let lip = 0.12 * (1.0 - smoothstep(0.0, 2.0, abs(edge + 2.0)));
    let bank = min(min(p.x, size.x-p.x), min(p.y, size.y-p.y));
    let shade = (0.91 + glaze * 0.095 + grain * 0.014 + stone * 0.018 - grout * 0.13 - occlusion + lip)
              * (0.81 + 0.19 * smoothstep(0.0, 18.0, bank));
    return vec4<f32>(floor_color * shade * (1.0 - 0.22 * leaf), 1.0);
}
