use super::{ToplevelSizing, WindowOptions, px};

#[test]
fn fractional_bounds_constrain_the_initial_allocation_before_fixing_the_size() {
    let options = WindowOptions::new("Player")
        .size(px(120.25), px(800.5))
        .min_size(px(320.25), px(180.0))
        .max_size(px(640.0), px(360.75));
    let sizing = ToplevelSizing::new(&options).unwrap();
    assert_eq!(sizing.min, Some((321, 180)));
    assert_eq!(sizing.max, Some((640, 360)));
    assert_eq!(
        (sizing.initial.width, sizing.initial.height),
        (px(321.0), px(360.0))
    );
    let fixed = ToplevelSizing::new(&options.resizable(false)).unwrap();
    assert_eq!(fixed.min, Some((321, 360)));
    assert_eq!(fixed.max, fixed.min);
    assert!(!fixed.resizable);
}

#[test]
fn unusable_bounds_fail_before_creating_a_native_window() {
    let options = WindowOptions::new("Player");
    for invalid in [
        options
            .min_size(px(1.25), px(1.0))
            .max_size(px(1.75), px(5.0)),
        options.max_size(px(0.5), px(50.0)),
        options.min_size(px(-0.5), px(50.0)),
        options.min_size(px(f32::NAN), px(50.0)),
        options.max_size(px(i32::MAX as f32), px(50.0)),
    ] {
        assert!(ToplevelSizing::new(&invalid).is_err());
    }
}
