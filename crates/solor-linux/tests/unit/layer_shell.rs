use super::{requested_size, units};
use crate::layer_shell::{Anchor, Layer, LayerShellOptions};
use solor::px;

#[test]
fn stretch_requires_opposite_anchors_only_along_the_automatic_axis() {
    let mut options = LayerShellOptions {
        anchor: Anchor::Top | Anchor::Right,
        height: Some(px(35.5)),
        ..LayerShellOptions::new("panel", Layer::Top)
    };
    assert!(requested_size(&options).is_err());
    options.anchor |= Anchor::Left;
    assert_eq!(requested_size(&options).unwrap(), (0, 36));
    options.height = None;
    assert!(requested_size(&options).is_err());
    options.anchor |= Anchor::Bottom;
    assert_eq!(requested_size(&options).unwrap(), (0, 0));
}

#[test]
fn fixed_extents_cannot_turn_into_automatic_sizes_or_overflow_coordinates() {
    let mut options = LayerShellOptions::new("panel", Layer::Top);
    for value in [0.25, -1.0, f32::NAN, f32::INFINITY, i32::MAX as f32] {
        options.width = Some(px(value));
        assert!(requested_size(&options).is_err());
    }
    assert_eq!(units(px(-3.75)).unwrap(), -4);
}
