use super::{EventResponse, Presentation, WindowBackground, WindowEvent, WindowHandler};
use crate::{
    Render, Ui, ViewFrame, ViewId, layout::Size, pointer::PointerFrame, raster::RasterScale,
};
use anyhow::Result;

/// Converts a recorded window frame into the platform's submission format.
pub trait ViewRenderer {
    /// Output accepted by the window host.
    type Target: Presentation;
    /// Render in paint order. Reset any damage history if the preceding frame was not submitted.
    fn render(
        &mut self,
        frame: ViewFrame,
        size: Size,
        scale: RasterScale,
        reset: bool,
    ) -> Result<<Self::Target as Presentation>::Frame<'_>>;
}

/// A mounted view whose input geometry advances when its rendered frame is submitted.
pub struct UiWindow<R> {
    ui: Ui,
    view: ViewId,
    renderer: R,
    prepared: Option<PreparedFrame>,
    animate: bool,
    caret: Option<(u64, crate::text::CaretBounds)>,
}

struct PreparedFrame {
    pointer: PointerFrame,
    animate: bool,
    caret: Option<(u64, crate::text::CaretBounds)>,
}

impl<R: ViewRenderer> UiWindow<R> {
    /// Mounts a root on a renderer owned by this window.
    pub fn with_renderer(mut ui: Ui, root: impl Render, renderer: R) -> Result<Self> {
        let view = ui.mount(root)?;
        Ok(Self {
            ui,
            view,
            renderer,
            prepared: None,
            animate: false,
            caret: None,
        })
    }
}
impl<R: ViewRenderer + From<WindowBackground>> UiWindow<R> {
    /// Mounts the root with an opaque background.
    pub fn mount(ui: Ui, root: impl Render) -> Result<Self> {
        Self::mount_with_background(ui, root, WindowBackground::Opaque)
    }
    /// Mounts the root with the platform's requested background mode.
    pub fn mount_with_background(
        ui: Ui,
        root: impl Render,
        background: WindowBackground,
    ) -> Result<Self> {
        Self::with_renderer(ui, root, R::from(background))
    }
}

impl<R: ViewRenderer> WindowHandler<R::Target> for UiWindow<R> {
    fn set_clipboard(&mut self, clipboard: std::rc::Rc<dyn crate::clipboard::PlatformClipboard>) {
        self.ui.set_clipboard(clipboard);
    }

    fn text_input_session(&self) -> Option<u64> {
        self.ui
            .text_input_session(self.view)
            .expect("mounted window view")
    }
    fn poll_tasks(&mut self, waker: &std::task::Waker) -> Result<()> {
        self.ui.poll_tasks(waker)
    }

    fn caret_bounds(&self) -> Option<crate::text::CaretBounds> {
        let (session, bounds) = self.caret?;
        (self.text_input_session() == Some(session)).then_some(bounds)
    }

    fn surrounding_text(&mut self, max_bytes: usize) -> Option<crate::text::SurroundingText<'_>> {
        self.ui
            .focused_text_input(self.view)
            .expect("mounted window view")?
            .surrounding_text(max_bytes)
    }

    fn set_window_resizable(&mut self, resizable: bool) -> Result<()> {
        self.ui.set_window_resizable(self.view, resizable)
    }

    fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
        let mut response = EventResponse::default();
        match event {
            WindowEvent::TextInput(edit) => {
                self.ui.dispatch_text_input(self.view, edit)?;
            }
            WindowEvent::Pointer(input) => {
                response.window_control = self.ui.dispatch_pointer(self.view, input)?
            }
            WindowEvent::Key(event) => {
                self.ui.dispatch_key(self.view, &event)?;
            }
            WindowEvent::FrameReady(time) if std::mem::take(&mut self.animate) => {
                self.ui.dispatch_frame(self.view, time)?;
                response.redraw = true;
            }
            _ => {}
        }
        response.redraw |= self
            .next_redraw()
            .is_some_and(|deadline| deadline <= std::time::Instant::now());
        Ok(response)
    }

    fn pointer_cursor(&self, pointer: crate::pointer::PointerId) -> crate::style::Cursor {
        self.ui.pointer_cursor(self.view, pointer)
    }

    fn next_redraw(&self) -> Option<std::time::Instant> {
        self.ui.next_redraw(self.view)
    }

    fn frame(
        &mut self,
        size: Size,
        scale: RasterScale,
    ) -> Result<<R::Target as Presentation>::Frame<'_>> {
        self.ui.flush()?;
        self.ui.layout(self.view, size)?;
        let frame = self.ui.record_frame(self.view)?;
        let prepared = PreparedFrame {
            pointer: frame.pointer_frame().clone(),
            animate: frame.requests_animation_frame(),
            caret: self.text_input_session().zip(frame.caret_bounds()),
        };
        let reset = self.prepared.take().is_some();
        let output = self.renderer.render(frame, size, scale, reset)?;
        self.prepared = Some(prepared);
        Ok(output)
    }
    fn frame_submitted(&mut self) -> Result<()> {
        if let Some(PreparedFrame {
            pointer,
            animate,
            caret,
        }) = self.prepared.take()
        {
            self.ui.accept_pointer_frame(pointer)?;
            self.animate = animate;
            self.caret = caret;
        }
        Ok(())
    }
}
