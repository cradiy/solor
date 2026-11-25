fn effect(uv: vec2<f32>, position: f32, size: vec2<f32>) -> vec4<f32> {
    let height = max(size.y, 1.0);
    let point = (uv - vec2<f32>(0.5)) * size;
    let p = point / height;
    let cell_size = 4.5;
    let screen = point / cell_size;
    let grid = vec2<f32>(0.96 * screen.x + 0.28 * screen.y,
                        -0.28 * screen.x + 0.96 * screen.y);
    let center = floor(grid) + vec2<f32>(0.5);
    let cell_y = (0.28 * center.x + 0.96 * center.y) * cell_size / height;
    let dot_distance = length(fract(grid) - vec2<f32>(0.5));
    let paper = vec3<f32>(0.967, 0.928, 0.831);
    let aa = 0.55 / height;

    let travel = position - 0.5;
    let aspect = size.x / height;
    let sun = vec2<f32>(0.27 * min(aspect, 1.8) * travel, -0.095);
    let sun_radius = min(0.247, 0.32 * aspect);
    let disc = smoothstep(-1.2, 1.2, (sun_radius - length(p - sun)) * height);
    let vermilion = mix(vec3<f32>(0.99, 0.54, 0.31), vec3<f32>(0.87, 0.22, 0.15),
                        smoothstep(-0.34, 0.12, p.y));
    let sky = mix(paper, vermilion, disc);

    let swell = 0.075 + 0.068 * sin(4.7 * p.x + travel) + 0.033 * sin(9.3 * p.x - travel);
    let sea = smoothstep(swell - aa, swell + aa, p.y);
    let shoal = 0.24 + 0.084 * sin(5.0 * p.x - 0.9 + travel);
    let depth = smoothstep(shoal - 0.14, shoal + 0.10, cell_y);
    let radius = mix(0.15, 0.63, depth);
    let feather = 0.5 / cell_size;
    let ink = 1.0 - smoothstep(radius - feather, radius + feather, dot_distance);
    let water = mix(vec3<f32>(0.65, 0.79, 0.74), vec3<f32>(0.055, 0.19, 0.22), ink);
    return vec4<f32>(mix(sky, water, sea), 1.0);
}
