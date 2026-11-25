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
fn movable_panel_requires_explicit_dimensions_and_unreserved_space() {
    use crate::layer_shell::ExclusiveZone;
    let mut options = LayerShellOptions {
        movable: true,
        ..LayerShellOptions::new("panel", Layer::Top)
    };
    assert!(requested_size(&options).is_err());
    options.width = Some(px(320.0));
    options.height = Some(px(80.0));
    assert_eq!(requested_size(&options).unwrap(), (320, 80));
    options.exclusive_zone = ExclusiveZone::Reserve(px(80.0));
    assert!(requested_size(&options).is_err());
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
    assert_eq!(receiver.take().visible, Some(false));

    handle.set_visible(true)?;
    handle.set_visible(false)?;
    receiver.drain()?;
    assert_eq!(receiver.take().visible, Some(false));
    assert_eq!(receiver.drain().unwrap_err().kind(), ErrorKind::WouldBlock);

    handle.close()?;
    assert_eq!(receiver.take().visible, None);
    assert_eq!(
        handle.set_visible(true).unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
    Ok(())
}

#[test]
fn input_region_rounds_logical_edges_outward_and_ignores_empty_rectangles() -> std::io::Result<()> {
    use crate::surface_control::{InputRect, InputRegion};
    let region = InputRegion::from_rects(Some(&[
        InputRect::new(px(-0.25), px(1.75), px(2.5), px(1.25)),
        InputRect::new(px(2.25), px(3.25), px(0.0), px(1.0)),
        InputRect::new(px(8.0), px(4.0), px(f32::MIN_POSITIVE), px(1.0)),
    ]))?;
    assert_eq!(
        region,
        InputRegion::Rects(vec![[-1, 1, 4, 2], [8, 4, 1, 1]])
    );
    Ok(())
}

#[test]
fn invalid_region_updates_preserve_the_last_complete_request() -> std::io::Result<()> {
    use crate::{
        layer_shell::LayerSurfaceHandle,
        surface_control::{InputRect, InputRegion},
    };
    use std::io::ErrorKind;

    let panel = LayerSurfaceHandle::new(false)?;
    let receiver = panel.claim()?;
    let first = InputRect::new(px(0.0), px(0.0), px(10.0), px(8.0));
    panel.set_input_region(Some(&[first]))?;
    for invalid in [
        InputRect::new(px(f32::NAN), px(1.0), px(1.0), px(1.0)),
        InputRect::new(px(0.0), px(1.0), px(-1.0), px(1.0)),
        InputRect::new(px(128.0), px(1.0), px(2147483520.0), px(1.0)),
    ] {
        assert_eq!(
            panel
                .set_input_region(Some(&[first, invalid]))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }
    assert_eq!(
        receiver.take().region,
        Some(InputRegion::Rects(vec![[0, 0, 10, 8]]))
    );
    panel.close()?;
    assert_eq!(
        panel.set_input_region(None).unwrap_err().kind(),
        ErrorKind::BrokenPipe
    );
    Ok(())
}

#[test]
fn rejected_layer_changes_preserve_pending_updates_on_a_hidden_panel() -> std::io::Result<()> {
    use crate::layer_shell::{
        ExclusiveZone, KeyboardInteractivity, LayerRules, LayerSurfaceHandle,
    };
    use std::io::ErrorKind;

    let handle = LayerSurfaceHandle::new(false)?;
    handle.set_size(Some(px(180.0)), Some(px(40.0)))?;
    let binding = handle.claim()?;
    binding.set_layer_rules(LayerRules {
        anchor: Anchor::Top,
        movable: false,
        version: 3,
    })?;
    handle.set_exclusive_zone(ExclusiveZone::Reserve(px(40.0)))?;
    assert_eq!(
        handle.set_size(None, Some(px(40.0))).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        handle
            .set_keyboard_interactivity(KeyboardInteractivity::OnDemand)
            .unwrap_err()
            .kind(),
        ErrorKind::Unsupported
    );
    let pending = binding.take();
    assert_eq!(pending.layer.size, Some((180, 40)));
    assert_eq!(pending.layer.zone, Some(40));
    assert_eq!(pending.layer.keyboard, None);
    Ok(())
}
