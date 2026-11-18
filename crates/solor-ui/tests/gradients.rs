//! Gradient geometry, interpolation and stop ownership.
use anyhow::Result;
use solor_ui::{
    ColorSpace, ColorStop, Gradient, GradientError, GradientKind, color_stop,
    gradient_left_to_right, layout::Size, linear_gradient, rgb,
};

#[test]
fn every_stop_count_uses_the_same_ordering_sharing_and_edit_rules() -> Result<()> {
    for count in [2, 3, 17, 4097] {
        let stops: Vec<_> = (0..count)
            .map(|i| color_stop(rgb(i as u32), i as f32 / (count - 1) as f32))
            .collect();
        let gradient = gradient_left_to_right(stops.iter().copied())?;
        let mut edited = gradient.clone();
        let index = count / 2;
        let original = stops[index];
        assert_eq!(gradient.stops().as_ptr(), edited.stops().as_ptr());
        edited.set_gradient_stop(index, original)?;
        assert_eq!(gradient.stops().as_ptr(), edited.stops().as_ptr());
        let replacement = ColorStop {
            color: rgb(0x91eed3),
            ..original
        };
        edited.set_gradient_stop(index, replacement)?;
        assert_eq!(edited.sample(original.offset)?, replacement.color);
        assert_eq!(gradient.sample(original.offset)?, original.color);
        assert_eq!(gradient.stops(), stops);
        assert_eq!(
            linear_gradient(90.0, stops.into_iter().rev()),
            Err(GradientError::UnorderedStops)
        );
    }
    Ok(())
}

#[test]
fn all_geometries_sample_the_same_multistop_palette_in_box_coordinates() -> Result<()> {
    let colors = [
        rgb(0x0),
        rgb(0xff0000),
        rgb(0x0080ff),
        rgb(0x00ffaa),
        rgb(0xffff00),
    ];
    let size = Size {
        width: solor_core::px(200.0),
        height: solor_core::px(100.0),
    };
    let palette = Gradient::new(
        GradientKind::Linear,
        90.0,
        colors
            .into_iter()
            .enumerate()
            .map(|(i, c)| color_stop(c, i as f32 / 4.0)),
    )?;
    // At (150, 75) normalized coordinates are (0.5, 0.5).
    for (kind, position) in [
        (GradientKind::Linear, 0.75),
        (GradientKind::Radial, 0.5_f32.sqrt()),
        (GradientKind::Angular, 0.125),
        (GradientKind::Diamond, 1.0),
    ] {
        let gradient = palette.clone().gradient_kind(kind);
        assert_eq!(gradient.stops().as_ptr(), palette.stops().as_ptr());
        assert_eq!(
            gradient.sample_at(150.0, 75.0, size)?,
            palette.sample(position)?
        );
    }
    Ok(())
}

#[test]
fn segment_hints_interpolation_space_and_animation_leave_shared_stops_intact() -> Result<()> {
    let gradient = linear_gradient(
        90.0,
        [
            color_stop(rgb(0), 0.0),
            color_stop(rgb(0xffffff), 0.5),
            color_stop(rgb(0xff0000), 0.875),
        ],
    )?;
    assert_eq!(gradient.sample(0.25)?, rgb(0x808080));
    let perceptual = gradient.clone().color_space(ColorSpace::Oklab);
    assert!((98..=100).contains(&perceptual.sample(0.25)?.red));
    let hinted = gradient.clone().gradient_midpoint(0, 0.25)?;
    assert_eq!(hinted.sample(0.125)?, gradient.sample(0.25)?);
    assert_eq!(hinted.sample(0.75)?, gradient.sample(0.75)?);
    assert_eq!(gradient.gradient_midpoint_at(0), Some(0.5));
    assert_eq!(hinted.gradient_midpoint_at(2), None);
    let animated = hinted.clone().phase(-0.25)?.opacity(0.5)?;
    assert_eq!(animated.stops().as_ptr(), gradient.stops().as_ptr());
    assert_eq!(animated, hinted.phase(0.75)?.opacity(0.5)?);
    assert_eq!(animated.sample(0.0)?, animated.sample(1.0)?);
    assert_eq!(animated.sample(0.0)?.alpha, 128);
    assert_ne!(animated.sample(0.0)?, animated.sample(0.5)?);
    let angular = gradient.gradient_kind(GradientKind::Angular);
    assert_ne!(angular.sample(0.0)?, angular.sample(1.0)?);
    let seamless = angular.angular_seam_width(0.125)?;
    assert_eq!(seamless.sample(0.0)?, seamless.sample(1.0)?);
    Ok(())
}
