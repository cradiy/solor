//! Plain-text access to a window's system clipboard.
use anyhow::{Context as _, Result, ensure};
use std::{cell::RefCell, future::Future, pin::Pin, rc::Rc};

/// A pending clipboard transfer. An empty selection or a non-text offer yields `None`.
pub type ClipboardRead = Pin<Box<dyn Future<Output = Result<Option<String>>>>>;

/// Window-system access to the current clipboard selection.
pub trait PlatformClipboard {
    /// Offers UTF-8 text to other applications.
    fn write_text(&self, text: String) -> Result<()>;
    /// Starts reading the current text offer. Stop accumulating bytes if the given limit is exceeded.
    fn read_text(&self, max_bytes: Option<usize>) -> Result<ClipboardRead>;
}

/// A handle that can be captured before the UI is attached to its native window.
#[derive(Clone, Default)]
pub struct Clipboard(Rc<RefCell<ClipboardState>>);

#[derive(Clone)]
struct ClipboardState {
    platform: Option<Rc<dyn PlatformClipboard>>,
    text_limit: Option<usize>,
}

impl Default for ClipboardState {
    fn default() -> Self {
        Self {
            platform: None,
            text_limit: Some(16 * 1024 * 1024),
        }
    }
}

impl Clipboard {
    /// Sets the maximum UTF-8 byte length for new reads and writes on this UI.
    /// The default is 16 MiB. Pass `None` for unlimited text.
    pub fn set_text_limit(&self, max_bytes: Option<usize>) {
        self.0.borrow_mut().text_limit = max_bytes;
    }

    /// Offers text to the system clipboard.
    pub fn write_text(&self, text: impl Into<String>) -> Result<()> {
        let text = text.into();
        let state = self.0.borrow().clone();
        if let Some(limit) = state.text_limit {
            ensure!(text.len() <= limit, "clipboard text exceeds {limit} bytes");
        }
        state
            .platform
            .context("clipboard unavailable")?
            .write_text(text)
    }

    /// Requests the current text offer. Await the returned transfer on a foreground task.
    pub fn read_text(&self) -> impl Future<Output = Result<Option<String>>> + use<> {
        let state = self.0.borrow().clone();
        let transfer = state
            .platform
            .context("clipboard unavailable")
            .and_then(|clipboard| clipboard.read_text(state.text_limit));
        async move { transfer?.await }
    }
}

impl crate::Ui {
    /// Returns the clipboard for this UI.
    pub fn clipboard(&self) -> Clipboard {
        self.clipboard.clone()
    }

    /// Connects the clipboard handle to its window-system implementation.
    pub fn set_clipboard(&mut self, clipboard: Rc<dyn PlatformClipboard>) {
        self.clipboard.0.borrow_mut().platform = Some(clipboard);
    }
}
