use super::*;

#[test]
fn audit_preserves_pixels_outside_the_surface_damage() {
    let size = DeviceSize {
        width: 5,
        height: 3,
    };
    let image = |edits: &[(usize, usize)]| {
        let mut rgba = vec![255; 5 * 3 * 4];
        for &(x, y) in edits {
            rgba[(5 * y + x) * 4] = 0;
        }
        rgba
    };
    let region = Clip {
        left: 1,
        top: 1,
        right: 4,
        bottom: 2,
    };
    let mut audit = DamageAudit::default();
    let mut check = |rgba: Vec<u8>, damage: Option<Clip>| {
        audit.check(&RasterFrame {
            size,
            rgba: &rgba,
            damage: damage.map(Damage::from_rect).unwrap_or_default(),
            request_callback: false,
        })
    };
    assert!(check(image(&[]), None).is_err());
    check(image(&[]), Some(Clip::canvas(size))).unwrap();
    for outside in [(0, 1), (4, 1), (2, 0), (2, 2)] {
        assert!(check(image(&[outside]), Some(region)).is_err());
    }
    let inside = [(1, 1), (3, 1)];
    check(image(&inside), Some(region)).unwrap();
    check(image(&inside), None).unwrap();
    assert!(check(image(&[]), None).is_err());
}
