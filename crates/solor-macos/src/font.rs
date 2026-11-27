//! macOS system interface font discovery.
use objc2_app_kit::NSFont;
use objc2_foundation::MainThreadMarker;
use std::sync::OnceLock;

/// Returns the system UI font family name for macOS when available in font discovery.
pub fn system_ui_font() -> Option<String> {
    static FAMILY: OnceLock<Option<String>> = OnceLock::new();
    FAMILY
        .get_or_init(|| {
            if MainThreadMarker::new().is_none() {
                return Some("Helvetica Neue".to_owned());
            }
            let font = NSFont::systemFontOfSize(13.0);
            let name = font.familyName().map(|s| s.to_string())?;
            if name.starts_with('.') {
                // `.AppleSystemUIFont` is an internal AppKit alias not exposed as a fontdb family.
                Some("Helvetica Neue".to_owned())
            } else {
                Some(name)
            }
        })
        .clone()
}
