use crate::{disclosure::Disclosure, window_content::Content, window_renderer::Renderer};
use solor::{
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::{Clip, RasterScale},
};

#[test]
fn primary_release_updates_pixels_and_an_idle_frame_has_no_damage() -> anyhow::Result<()> {
    let row = Disclosure::new(&[], &[])?;
    let mut window = Renderer::new(Content::disclosure(row), true);
    let size = Size {
        width: px(240.0),
        height: px(100.0),
    };
    let initial = window.frame(size, RasterScale::ONE)?;
    assert_eq!(initial.damage, Some(Clip::canvas(initial.size)));
    let closed = initial.rgba.to_vec();
    window.frame_submitted();
    let sample = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin {
            x: px(6.5),
            y: px(10.0),
        },
        modifiers: Default::default(),
    };
    for button in [PointerButton::Secondary, PointerButton::Primary] {
        assert!(!window.event(WindowEvent::Pointer(PointerInput::Down(sample, button)))?);
        let dirty = window.event(WindowEvent::Pointer(PointerInput::Up(sample, button)))?;
        assert_eq!(dirty, button == PointerButton::Primary);
    }
    let open = window.frame(size, RasterScale::ONE)?;
    assert!(open.damage.is_some());
    assert_ne!(open.rgba, closed);
    window.frame_submitted();
    let idle = window.frame(size, RasterScale::ONE)?;
    assert!(idle.damage.is_none());
    assert!(!idle.request_callback);
    Ok(())
}
