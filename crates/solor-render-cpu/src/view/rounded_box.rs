pub(super) struct RoundedBox {
    pub(super) bounds: [f64; 4],       // left, top, right, bottom
    pub(super) corners: [[f64; 2]; 4], // top left clockwise
}

impl RoundedBox {
    pub(super) fn span(&self, y: f64) -> Option<(f64, f64)> {
        let [l, t, r, b] = self.bounds;
        if y < t || y >= b || l >= r {
            return None;
        }
        let inset = |[rx, ry]: [f64; 2], dy: f64| {
            if dy >= ry {
                0.0
            } else {
                let k = 1.0 - dy / ry;
                rx * (1.0 - (1.0 - k * k).sqrt())
            }
        };
        let [tl, tr, br, bl] = self.corners;
        let start = l + inset(tl, y - t).max(inset(bl, b - y));
        let end = r - inset(tr, y - t).max(inset(br, b - y));
        (start < end).then_some((start, end))
    }
}
