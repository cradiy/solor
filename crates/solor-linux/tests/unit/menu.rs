use super::*;

#[test]
fn fractional_anchor_is_clipped_to_the_panel_before_applying_its_offset() -> Result<()> {
    let panel = [40, 12, 100, 30];
    let item = SurfaceRect::new(px(-1.5), px(25.25), px(8.2), px(12.0));
    assert_eq!(anchor_bounds(item, panel)?, [40, 37, 7, 5]);

    let outside = SurfaceRect::new(px(100.), px(5.), px(3.), px(2.));
    assert!(anchor_bounds(outside, panel).is_err());
    Ok(())
}
