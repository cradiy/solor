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

#[test]
fn a_single_surface_consumes_the_latest_visibility_and_close_is_final() -> std::io::Result<()> {
    use crate::layer_shell::LayerSurfaceHandle;
    use std::io::ErrorKind;

    let handle = LayerSurfaceHandle::new(false)?;
    let receiver = handle.claim()?;
    assert!(handle.clone().claim().is_err());
    assert_eq!(receiver.visibility(), Some(false));

    handle.set_visible(true)?;
    handle.set_visible(false)?;
    receiver.drain()?;
    assert_eq!(receiver.visibility(), Some(false));
    assert_eq!(receiver.drain().unwrap_err().kind(), ErrorKind::WouldBlock);

    handle.close()?;
    assert_eq!(receiver.visibility(), None);
    assert_eq!(
        handle.set_visible(true).unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
    Ok(())
}
