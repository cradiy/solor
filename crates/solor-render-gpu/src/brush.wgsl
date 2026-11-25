if q.ramp.y == 0.0 { return q.params; }
let first = u32(q.ramp.x);
let end = first + u32(q.ramp.y);
let t = clamp(0.5 + dot((uv - 0.5)*size,q.params.xy),0.,1.);
var lo = first;
var hi = end;
while lo < hi {
    let mid = lo + (hi-lo)/2u;
    if stops[mid].segment.x <= t { lo = mid + 1u; } else { hi = mid; }
}
var color = stops[first].color;
if lo > first {
    let left = stops[lo-1u];
    color = left.color;
    if lo < end {
        let right = stops[lo];
        let f = pow((t-left.segment.x)/(right.segment.x-left.segment.x),left.segment.y);
        color = mix(left.color,right.color,f);
    }
}
return vec4f(color.rgb / max(color.a, 1e-8), color.a*q.params.z);
