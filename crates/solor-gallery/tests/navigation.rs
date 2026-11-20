//! Pointer navigation through the same mounted window used by the native launcher.
use anyhow::Result;
use solor::{
    layout::{Origin, Size},
    platform::{WindowEvent, WindowHandler},
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
};
use solor_render_cpu::CpuWindow;

fn click(window: &mut CpuWindow, x: f32, y: f32) -> Result<()> {
    let point = PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    };
    for input in [
        PointerInput::Down(point, PointerButton::Primary),
        PointerInput::Up(point, PointerButton::Primary),
    ] {
        window.event(WindowEvent::Pointer(input))?;
    }
    Ok(())
}

#[test]
fn controls_change_the_specimen_and_choices_survive_navigation() -> Result<()> {
    let size = Size {
        width: px(800.0),
        height: px(560.0),
    };
    let specimen = |window: &mut CpuWindow| -> Result<Vec<u8>> {
        let frame = window.frame(size, RasterScale::ONE)?;
        let stride = frame.size.width as usize * 4;
        let pixels = frame.rgba[200 * stride..350 * stride].to_vec();
        window.frame_submitted()?;
        assert!(!window.event(WindowEvent::FrameReady)?);
        Ok(pixels)
    };
    let mut window = solor_gallery::window()?;
    let normal = specimen(&mut window)?;
    click(&mut window, 269.0, 154.0)?;
    let wide = specimen(&mut window)?;
    assert!(normal != wide, "letter spacing must change the specimen");
    click(&mut window, 693.0, 29.0)?;
    let linear = specimen(&mut window)?;
    assert!(linear != wide, "Paint must replace the typography specimen");
    click(&mut window, 167.0, 154.0)?;
    let radial = specimen(&mut window)?;
    assert!(radial != linear, "radial and linear gradients must differ");
    click(&mut window, 749.0, 29.0)?;
    let center = specimen(&mut window)?;
    assert!(center != radial, "Layout must replace the paint specimen");
    click(&mut window, 215.0, 154.0)?;
    assert!(
        center != specimen(&mut window)?,
        "justify must reposition the items"
    );
    click(&mut window, 614.0, 29.0)?;
    assert!(
        wide == specimen(&mut window)?,
        "returning should preserve the selected tracking"
    );
    Ok(())
}
