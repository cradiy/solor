use solor_ui::text::{Engine, SharedEngine};

pub const FONT: &[u8] = include_bytes!("fonts/NotoSansCJKsc-fixture.otf");

pub fn fixture_engine() -> SharedEngine {
    Engine::from_font(FONT.to_vec(), "zh-CN").expect("valid fixture font")
}
