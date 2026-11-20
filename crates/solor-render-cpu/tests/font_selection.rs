//! Registration, family inheritance and retained glyphs across font faces.
use std::path::Path;

use anyhow::Result;
use solor::{
    Draw, Fonts, GenericFontFamily, Styled, Ui, component, div,
    raster::{DeviceSize, RasterScale},
    text,
};
use solor_render_cpu::pixels::Surface;

fn fixture() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/fonts/NotoSansCJKsc-fixture.otf"
    ))
}

const SIZE: DeviceSize = DeviceSize {
    width: 160,
    height: 80,
};

#[test]
fn registered_file_provides_glyphs_after_ui_is_dropped() -> Result<()> {
    let mut fonts = Fonts::empty();
    assert!(fonts.add_font(vec![0; 32]).is_err());
    assert!(fonts.families().is_empty());
    assert!(fonts.set_default_family("unknown").is_err());
    let name = fonts.load_font(fixture())?;
    fonts.set_default_family(&name)?;
    let mut ui = Ui::with_fonts(fonts);
    let view = ui.mount(component(|_| text("A中")))?;
    ui.layout(view, SIZE.at_unit_scale())?;
    let frame = ui.record_frame(view)?;
    drop(ui);
    let [Draw::Text { paragraph, .. }] = frame.draws() else {
        panic!("expected one paragraph");
    };
    assert_eq!(paragraph.faces.len(), 1);
    assert_eq!(paragraph.faces[0].data(), std::fs::read(fixture())?);
    assert_eq!(paragraph.lines[0].glyphs.len(), 2);
    let image = Surface::draw_view(&frame, SIZE, RasterScale::ONE)?;
    assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[0] < 200));
    Ok(())
}

#[test]
#[ignore = "requires SOLOR_TEST_LATIN_FONT pointing to a Latin font without CJK glyphs"]
fn inherited_family_falls_back_and_a_child_can_override_it() -> Result<()> {
    let latin_bytes = std::fs::read(std::env::var("SOLOR_TEST_LATIN_FONT")?)?;
    let mut fonts = Fonts::empty();
    let latin = fonts.add_font(latin_bytes.clone())?;
    let cjk = fonts.load_font(fixture())?;
    fonts.set_default_family(&cjk)?;
    fonts.set_generic_font_family(GenericFontFamily::Serif, Some(cjk.clone()));
    let mut ui = Ui::with_fonts(fonts);
    let root = ui.mount(component(move |_| {
        div()
            .font_family(latin)
            .font_fallbacks([GenericFontFamily::Serif.into()])
            .child(text("A中A"))
            .child(text("A").font_family(cjk))
    }))?;
    ui.layout(root, SIZE.at_unit_scale())?;
    let frame = ui.record_frame(root)?;
    let [
        Draw::Text {
            paragraph: mixed, ..
        },
        Draw::Text {
            paragraph: child, ..
        },
    ] = frame.draws()
    else {
        panic!("expected two paragraphs")
    };
    let glyphs = &mixed.lines[0].glyphs;
    assert_eq!(glyphs.len(), 3);
    assert_eq!(mixed.faces.len(), 2);
    assert_eq!(mixed.faces[glyphs[0].face].data(), latin_bytes);
    assert_eq!(glyphs[0].face, glyphs[2].face);
    assert_ne!(glyphs[0].face, glyphs[1].face);
    let fallback = &mixed.faces[glyphs[1].face];
    assert_eq!(fallback.data(), std::fs::read(fixture())?);
    assert_eq!(fallback.cache_key(), child.faces[0].cache_key());
    drop(ui);
    let image = Surface::draw_view(&frame, SIZE, RasterScale::ONE)?;
    for glyph in glyphs {
        let start = glyph.x.ceil() as usize;
        let end = (glyph.x + glyph.width).floor() as usize;
        assert!(
            image
                .rgba
                .as_chunks::<{ 4 * SIZE.width as usize }>()
                .0
                .iter()
                .take(24)
                .any(|row| row[4 * start..4 * end]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[0] < 200))
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Fontconfig sans-serif set to SOLOR_TEST_LATIN_FONT and the fixture family selected by the desktop"]
fn system_ui_restores_the_host_face_inside_an_application_font() -> Result<()> {
    let host = std::fs::read(std::env::var("SOLOR_TEST_LATIN_FONT")?)?;
    let mut ui = Ui::new();
    let view = ui.mount(component(|_| text("A")))?;
    ui.layout(view, SIZE.at_unit_scale())?;
    let frame = ui.record_frame(view)?;
    let [Draw::Text { paragraph, .. }] = frame.draws() else {
        panic!("expected the default system face");
    };
    assert_eq!(paragraph.faces[0].data(), host);

    let mut fonts = Fonts::system();
    let desktop = fonts.load_font(fixture())?;
    assert_eq!(
        solor_linux::system_ui_font().as_deref(),
        Some(desktop.as_str())
    );
    let app: solor::SharedString = fonts
        .families()
        .into_iter()
        .find(|name| *name != desktop.as_str())
        .expect("Latin family")
        .into();
    fonts.set_default_family(&app)?;
    fonts.set_generic_font_family(GenericFontFamily::SystemUi, Some(app.clone()));
    fonts.set_generic_font_family(GenericFontFamily::SystemUi, None);
    let mut ui = Ui::with_fonts(fonts);
    ui.set_system_font_provider(solor_linux::system_ui_font);
    let view = ui.mount(component(move |_| {
        div().child(text("A")).child(
            div()
                .font_family(GenericFontFamily::SansSerif)
                .child(text("A"))
                .child(
                    div()
                        .font_family(GenericFontFamily::SystemUi)
                        .child(text("A")),
                ),
        )
    }))?;
    ui.layout(view, SIZE.at_unit_scale())?;
    let frame = ui.record_frame(view)?;
    let [
        Draw::Text {
            paragraph: default, ..
        },
        Draw::Text {
            paragraph: inherited,
            ..
        },
        Draw::Text {
            paragraph: restored,
            ..
        },
    ] = frame.draws()
    else {
        panic!("expected all three labels");
    };
    assert_eq!(default.faces[0].data(), host);
    assert_eq!(default.faces[0].cache_key(), inherited.faces[0].cache_key());
    assert_eq!(restored.faces[0].data(), std::fs::read(fixture())?);
    Ok(())
}
