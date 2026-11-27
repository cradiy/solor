//! Plain-text system clipboard integration via `NSPasteboard`.
use anyhow::{Result, ensure};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;
use solor::clipboard::{ClipboardRead, PlatformClipboard};

pub(crate) struct MacClipboard;

impl PlatformClipboard for MacClipboard {
    fn write_text(&self, text: String) -> Result<()> {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        let ns_text = NSString::from_str(&text);
        #[allow(unsafe_code)]
        // SAFETY: `NSPasteboardTypeString` is a static AppKit constant.
        let string_type = unsafe { NSPasteboardTypeString };
        ensure!(
            pasteboard.setString_forType(&ns_text, string_type),
            "NSPasteboard failed to write UTF-8 text"
        );
        Ok(())
    }

    fn read_text(&self, max_bytes: Option<usize>) -> Result<ClipboardRead> {
        let pasteboard = NSPasteboard::generalPasteboard();
        #[allow(unsafe_code)]
        // SAFETY: `NSPasteboardTypeString` is a static AppKit constant.
        let string_type = unsafe { NSPasteboardTypeString };
        let value = pasteboard
            .stringForType(string_type)
            .map(|ns| ns.to_string());
        if let (Some(limit), Some(text)) = (max_bytes, value.as_ref()) {
            ensure!(text.len() <= limit, "clipboard text exceeds {limit} bytes");
        }
        Ok(Box::pin(std::future::ready(Ok(value))))
    }
}
