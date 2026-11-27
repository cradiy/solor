use super::{
    MAX_CPU_SURFACE_PIXELS, anchor_frame_top, blur_rect_to_view_frame, budgeted_raster_target,
    compute_traffic_light_layout, style_mask_for,
};
use objc2_app_kit::NSWindowStyleMask;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use solor::{layout::Size, platform::SurfaceRect, px, raster::RasterScale};

#[test]
fn style_mask_uses_full_size_content_view_for_integrated_titlebar() {
    let integrated = style_mask_for(false, true);
    assert!(integrated.contains(NSWindowStyleMask::Titled));
    assert!(integrated.contains(NSWindowStyleMask::Closable));
    assert!(integrated.contains(NSWindowStyleMask::Miniaturizable));
    assert!(integrated.contains(NSWindowStyleMask::Resizable));
    assert!(integrated.contains(NSWindowStyleMask::FullSizeContentView));
    assert_ne!(integrated, NSWindowStyleMask::Borderless);

    let native = style_mask_for(true, true);
    assert!(native.contains(NSWindowStyleMask::Titled));
    assert!(native.contains(NSWindowStyleMask::Closable));
    assert!(native.contains(NSWindowStyleMask::Miniaturizable));
    assert!(native.contains(NSWindowStyleMask::Resizable));
    assert!(!native.contains(NSWindowStyleMask::FullSizeContentView));

    let fixed = style_mask_for(false, false);
    assert!(!fixed.contains(NSWindowStyleMask::Resizable));
    assert!(fixed.contains(NSWindowStyleMask::FullSizeContentView));
}

#[test]
fn traffic_light_layout_centers_buttons_vertically_and_preserves_gap() {
    let close = NSRect::new(NSPoint::new(7.0, 7.0), NSSize::new(14.0, 14.0));
    let minimize = NSRect::new(NSPoint::new(27.0, 7.0), NSSize::new(14.0, 14.0));
    let titlebar = NSRect::new(NSPoint::new(0.0, 532.0), NSSize::new(800.0, 28.0));

    let layout = compute_traffic_light_layout(560.0, close, minimize, titlebar, 52.0, 16.0)
        .expect("valid LTR button frames");

    assert_eq!(layout.close_origin, NSPoint::new(16.0, 19.0));
    assert_eq!(layout.minimize_origin, NSPoint::new(36.0, 19.0));
    assert_eq!(layout.zoom_origin, NSPoint::new(56.0, 19.0));
    assert_eq!(
        layout.titlebar_frame,
        NSRect::new(NSPoint::new(0.0, 508.0), NSSize::new(800.0, 52.0))
    );
}

#[test]
fn traffic_light_layout_rejects_degenerate_or_rtl_frames() {
    let valid_titlebar = NSRect::new(NSPoint::new(0.0, 532.0), NSSize::new(800.0, 28.0));
    let rtl_close = NSRect::new(NSPoint::new(47.0, 7.0), NSSize::new(14.0, 14.0));
    let rtl_minimize = NSRect::new(NSPoint::new(27.0, 7.0), NSSize::new(14.0, 14.0));
    assert_eq!(
        compute_traffic_light_layout(560.0, rtl_close, rtl_minimize, valid_titlebar, 52.0, 16.0),
        None
    );

    let zero_close = NSRect::new(NSPoint::new(7.0, 7.0), NSSize::ZERO);
    let minimize = NSRect::new(NSPoint::new(27.0, 7.0), NSSize::new(14.0, 14.0));
    assert_eq!(
        compute_traffic_light_layout(560.0, zero_close, minimize, valid_titlebar, 52.0, 16.0),
        None
    );
}

#[test]
fn anchor_frame_top_preserves_top_left_corner_across_frame_toggles() {
    let old = NSRect::new(NSPoint::new(120.0, 200.0), NSSize::new(800.0, 560.0));
    let resized = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 588.0));
    let anchored = anchor_frame_top(old, resized);
    assert_eq!(anchored.origin.x, 120.0);
    assert_eq!(anchored.origin.y + anchored.size.height, 200.0 + 560.0);
    assert_eq!(anchored.size, NSSize::new(800.0, 588.0));
}

#[test]
fn budgeted_raster_target_preserves_retina_5k_and_clamps_oversized_extents() {
    for (w, h) in [(800.0, 560.0), (1728.0, 1117.0), (2560.0, 1440.0)] {
        let size = Size {
            width: px(w),
            height: px(h),
        };
        let (scale, physical) =
            budgeted_raster_target(size, RasterScale(2.0), MAX_CPU_SURFACE_PIXELS)
                .expect("retina target fits within CPU budget");
        assert_eq!(scale, RasterScale(2.0));
        assert!(u64::from(physical.width) * u64::from(physical.height) <= MAX_CPU_SURFACE_PIXELS);
    }

    for (w, h) in [(3008.0, 1692.0), (8000.0, 4000.0)] {
        let size = Size {
            width: px(w),
            height: px(h),
        };
        let (scale, physical) =
            budgeted_raster_target(size, RasterScale(2.0), MAX_CPU_SURFACE_PIXELS)
                .expect("oversized window clamps within CPU budget");
        assert!(scale.0 > 0.0 && scale.0 < 2.0);
        assert!(u64::from(physical.width) * u64::from(physical.height) <= MAX_CPU_SURFACE_PIXELS);
    }
}

#[test]
fn blur_rect_to_view_frame_flips_y_and_skips_empty_rects() {
    let rect = SurfaceRect::new(px(10.0), px(20.0), px(200.0), px(80.0));
    assert_eq!(
        blur_rect_to_view_frame(rect, 560.0),
        Some(NSRect::new(
            NSPoint::new(10.0, 460.0),
            NSSize::new(200.0, 80.0)
        ))
    );
    assert_eq!(
        blur_rect_to_view_frame(SurfaceRect::new(px(0.0), px(0.0), px(0.0), px(50.0)), 560.0),
        None
    );
    assert_eq!(
        blur_rect_to_view_frame(
            SurfaceRect::new(px(0.0), px(0.0), px(50.0), px(-1.0)),
            560.0
        ),
        None
    );
}
