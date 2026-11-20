use solor::text::{Engine, SharedEngine};

pub const FONT: &[u8] = include_bytes!("../../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");

pub fn fixture_engine() -> SharedEngine {
    let mut fonts = solor::Fonts::empty();
    fonts.set_locale("zh-CN");
    fonts.add_font(FONT.to_vec()).expect("valid fixture font");
    Engine::with_fonts(fonts)
}
