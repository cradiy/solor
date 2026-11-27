//! AppKit cursor mapping and balanced cursor visibility tracking.
use objc2::{MainThreadMarker, msg_send, rc::Retained, sel};
use objc2_app_kit::{
    NSCursor, NSCursorFrameResizeDirections, NSCursorFrameResizePosition, NSHorizontalDirections,
    NSVerticalDirections,
};
use solor::{ResizeEdge, style::Cursor};

#[derive(Default)]
pub(crate) struct CursorState {
    applied: Option<Cursor>,
    hidden: bool,
}

impl CursorState {
    pub(crate) fn sync(&mut self, cursor: Cursor, mtm: MainThreadMarker) -> bool {
        if self.applied == Some(cursor) {
            return false;
        }
        if cursor == Cursor::None {
            if !self.hidden {
                NSCursor::hide();
                self.hidden = true;
            }
            self.applied = Some(cursor);
            return true;
        }
        if self.hidden {
            NSCursor::unhide();
            self.hidden = false;
        }
        let native = native_cursor(cursor, mtm);
        native.set();
        self.applied = Some(cursor);
        true
    }

    pub(crate) fn current(&self, mtm: MainThreadMarker) -> Retained<NSCursor> {
        match self.applied {
            Some(Cursor::None) | None => NSCursor::arrowCursor(),
            Some(cursor) => native_cursor(cursor, mtm),
        }
    }
}

impl Drop for CursorState {
    fn drop(&mut self) {
        if self.hidden {
            NSCursor::unhide();
            self.hidden = false;
        }
    }
}

fn native_cursor(cursor: Cursor, _mtm: MainThreadMarker) -> Retained<NSCursor> {
    match cursor {
        Cursor::Auto | Cursor::Default | Cursor::None | Cursor::Wait | Cursor::Progress => {
            NSCursor::arrowCursor()
        }
        Cursor::Pointer => NSCursor::pointingHandCursor(),
        Cursor::ContextMenu => NSCursor::contextualMenuCursor(),
        Cursor::Help => NSCursor::arrowCursor(),
        Cursor::Cell | Cursor::Crosshair => NSCursor::crosshairCursor(),
        Cursor::Text => NSCursor::IBeamCursor(),
        Cursor::VerticalText => NSCursor::IBeamCursorForVerticalLayout(),
        Cursor::Move | Cursor::AllScroll | Cursor::Grab => NSCursor::openHandCursor(),
        Cursor::Grabbing => NSCursor::closedHandCursor(),
        Cursor::Alias => NSCursor::dragLinkCursor(),
        Cursor::Copy => NSCursor::dragCopyCursor(),
        Cursor::NoDrop | Cursor::NotAllowed => NSCursor::operationNotAllowedCursor(),
        Cursor::ColResize => NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::All),
        Cursor::RowResize => NSCursor::rowResizeCursorInDirections(NSVerticalDirections::All),
        Cursor::EwResize => frame_resize(NSCursorFrameResizePosition::Right).unwrap_or_else(|| {
            NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::All)
        }),
        Cursor::NsResize => frame_resize(NSCursorFrameResizePosition::Bottom)
            .unwrap_or_else(|| NSCursor::rowResizeCursorInDirections(NSVerticalDirections::All)),
        Cursor::NeswResize => frame_resize(NSCursorFrameResizePosition::TopRight)
            .unwrap_or_else(NSCursor::arrowCursor),
        Cursor::NwseResize => {
            frame_resize(NSCursorFrameResizePosition::TopLeft).unwrap_or_else(NSCursor::arrowCursor)
        }
        Cursor::Resize(edge) => {
            let position = match edge {
                ResizeEdge::Top => NSCursorFrameResizePosition::Top,
                ResizeEdge::TopRight => NSCursorFrameResizePosition::TopRight,
                ResizeEdge::Right => NSCursorFrameResizePosition::Right,
                ResizeEdge::BottomRight => NSCursorFrameResizePosition::BottomRight,
                ResizeEdge::Bottom => NSCursorFrameResizePosition::Bottom,
                ResizeEdge::BottomLeft => NSCursorFrameResizePosition::BottomLeft,
                ResizeEdge::Left => NSCursorFrameResizePosition::Left,
                ResizeEdge::TopLeft => NSCursorFrameResizePosition::TopLeft,
            };
            frame_resize(position).unwrap_or_else(|| match edge {
                ResizeEdge::Left | ResizeEdge::Right => {
                    NSCursor::columnResizeCursorInDirections(NSHorizontalDirections::All)
                }
                ResizeEdge::Top | ResizeEdge::Bottom => {
                    NSCursor::rowResizeCursorInDirections(NSVerticalDirections::All)
                }
                _ => NSCursor::arrowCursor(),
            })
        }
        Cursor::ZoomIn => NSCursor::zoomInCursor(),
        Cursor::ZoomOut => NSCursor::zoomOutCursor(),
    }
}

fn frame_resize(position: NSCursorFrameResizePosition) -> Option<Retained<NSCursor>> {
    #[allow(unsafe_code)]
    // SAFETY: Checking `respondsToSelector:` on `NSCursor` class object on the main thread
    // ensures `frameResizeCursorFromPosition:inDirections:` exists (macOS 15+).
    unsafe {
        let cls = objc2::class!(NSCursor);
        let supported: bool = msg_send![
            cls,
            respondsToSelector: sel!(frameResizeCursorFromPosition:inDirections:)
        ];
        supported.then(|| {
            NSCursor::frameResizeCursorFromPosition_inDirections(
                position,
                NSCursorFrameResizeDirections::All,
            )
        })
    }
}
