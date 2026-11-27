//! AppKit `NSWindow`, `NSView`, input routing, IME, file drop, and CPU presentation.
use crate::{
    buffer::CpuLayerPresenter,
    clipboard::MacClipboard,
    cursor::CursorState,
    display_link::DisplayLink,
    events::{
        MOUSE_POINTER_ID, key_event_from_ns_event, modifier_key_transition,
        pointer_button_from_number, pointer_sample_from_event_in_view, scroll_delta_from_raw,
        view_point_to_solor,
    },
    life::WindowLife,
    wake::{MacWake, invoke_main_tick},
};
use anyhow::{Context as _, Result};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSButton, NSColor,
    NSDragOperation, NSDraggingDestination, NSDraggingInfo, NSEvent, NSEventModifierFlags,
    NSFloatingWindowLevel, NSNormalWindowLevel, NSPasteboardReadingOptionKey,
    NSPasteboardTypeFileURL, NSScreen, NSTextInputClient, NSTrackingArea, NSTrackingAreaOptions,
    NSView, NSViewLayerContentsRedrawPolicy, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowButton, NSWindowDelegate,
    NSWindowLevel, NSWindowOrderingMode, NSWindowStyleMask, NSWindowTitleVisibility,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSDictionary, NSNotFound, NSNotification,
    NSObjectProtocol, NSOperatingSystemVersion, NSPoint, NSProcessInfo, NSRange, NSRangePointer,
    NSRect, NSSize, NSString, NSURL,
};
use objc2_quartz_core::CALayer;
use solor::{
    ResizeEdge,
    keyboard::KeyState,
    layout::Size,
    platform::{
        EventResponse, PlatformWindow, SurfaceRect, WindowBackground, WindowControl, WindowEvent,
        WindowHandle, WindowHandler, WindowOptions, WindowState,
    },
    pointer::PointerInput,
    px,
    raster::{Clip, Damage, DeviceSize, RasterScale, Rgba8},
    text::{Preedit, TextInputEvent, TextSelection},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::Arc,
    task::Waker,
    time::Instant,
};

const MAX_SURROUNDING_BYTES: usize = 4000;
const MAX_CPU_SURFACE_PIXELS: u64 = 16 * 1024 * 1024;
const DEFAULT_INTEGRATED_TITLEBAR_HEIGHT: f64 = 52.0;
const DEFAULT_TRAFFIC_LIGHT_LEFT_INSET: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TrafficLightFrames {
    titlebar: NSRect,
    close: NSRect,
    minimize: NSRect,
    zoom: NSRect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TrafficLightLayout {
    titlebar_frame: NSRect,
    close_origin: NSPoint,
    minimize_origin: NSPoint,
    zoom_origin: NSPoint,
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    edge: ResizeEdge,
    start_mouse: NSPoint,
    start_frame: NSRect,
}

pub(crate) enum WindowView {
    Cpu(Box<dyn WindowHandler<Rgba8>>),
    #[cfg(feature = "gpu")]
    Gpu {
        handler: Box<dyn WindowHandler<crate::gpu::Native>>,
        max_dimension: u32,
    },
}

macro_rules! dispatch_view_mut {
    ($self:expr, $handler:ident => $expr:expr) => {
        match $self {
            WindowView::Cpu($handler) => $expr,
            #[cfg(feature = "gpu")]
            WindowView::Gpu {
                handler: $handler, ..
            } => $expr,
        }
    };
}

macro_rules! dispatch_view_ref {
    ($self:expr, $handler:ident => $expr:expr) => {
        match $self {
            WindowView::Cpu($handler) => $expr,
            #[cfg(feature = "gpu")]
            WindowView::Gpu {
                handler: $handler, ..
            } => $expr,
        }
    };
}

impl WindowView {
    fn set_clipboard(&mut self, clipboard: Rc<dyn solor::clipboard::PlatformClipboard>) {
        dispatch_view_mut!(self, h => h.set_clipboard(clipboard));
    }

    fn set_window_resizable(&mut self, resizable: bool) -> Result<()> {
        dispatch_view_mut!(self, h => h.set_window_resizable(resizable))
    }

    fn file_drop_target(&self, pos: solor::layout::Origin) -> Option<solor::NodeId> {
        dispatch_view_ref!(self, h => h.file_drop_target(pos))
    }

    fn drop_files(&mut self, target: solor::NodeId, paths: &[PathBuf]) -> Result<bool> {
        dispatch_view_mut!(self, h => h.drop_files(target, paths))
    }

    fn text_input_session(&self) -> Option<u64> {
        dispatch_view_ref!(self, h => h.text_input_session())
    }

    fn poll_tasks(&mut self, waker: &Waker) -> Result<()> {
        dispatch_view_mut!(self, h => h.poll_tasks(waker))
    }

    fn caret_bounds(&self) -> Option<solor::text::CaretBounds> {
        dispatch_view_ref!(self, h => h.caret_bounds())
    }

    fn surrounding_text(&mut self, max_bytes: usize) -> Option<solor::text::SurroundingText<'_>> {
        dispatch_view_mut!(self, h => h.surrounding_text(max_bytes))
    }

    fn event(&mut self, event: WindowEvent) -> Result<EventResponse> {
        dispatch_view_mut!(self, h => h.event(event))
    }

    fn pointer_cursor(&self, pointer: solor::pointer::PointerId) -> solor::style::Cursor {
        dispatch_view_ref!(self, h => h.pointer_cursor(pointer))
    }

    fn next_redraw(&self) -> Option<Instant> {
        dispatch_view_ref!(self, h => h.next_redraw())
    }
}

pub(crate) struct WindowRecord {
    pub(crate) life: Rc<WindowLife>,
    ns_window: Retained<SolorWindow>,
    ns_view: Retained<SolorView>,
    presenter: Option<CpuLayerPresenter>,
    display_link: Option<DisplayLink>,
    view: Option<WindowView>,
    title: String,
    background: WindowBackground,
    resizable: bool,
    min_size: Option<Size>,
    max_size: Option<Size>,
    size: Size,
    scale: RasterScale,
    dirty: bool,
    full_damage: bool,
    wants_animation: bool,
    #[cfg(feature = "gpu")]
    acquire_retry: Option<Instant>,
    failure: Option<anyhow::Error>,
    start: Instant,
    cursor: CursorState,
    cursor_inside: bool,
    last_modifiers: NSEventModifierFlags,
    last_key_equivalent: Option<solor::keyboard::KeyEvent>,
    ime_active_dispatch: bool,
    ime_consumed: bool,
    has_marked_text: bool,
    marked_utf16_len: usize,
    resize_drag: Option<ResizeDrag>,
    traffic_light_frames: Option<TrafficLightFrames>,
    blur_rects: Vec<SurfaceRect>,
    blur_dirty: bool,
    blur_views: Vec<Retained<NSVisualEffectView>>,
    file_drop_hover: Option<solor::NodeId>,
    presented_once: bool,
    revisit: Rc<Cell<bool>>,
    wake: Arc<MacWake>,
}

struct WindowIvars {
    record: RefCell<Weak<RefCell<WindowRecord>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[ivars = WindowIvars]
    struct SolorWindow;

    unsafe impl NSObjectProtocol for SolorWindow {}

    impl SolorWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            true
        }
    }

    unsafe impl NSWindowDelegate for SolorWindow {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> bool {
            let record = self.ivars().record.borrow().upgrade();
            if let Some(record) = record {
                let life = record.borrow().life.clone();
                if life.blocked() {
                    false
                } else {
                    let allowed = life.request_close();
                    if allowed {
                        invoke_main_tick();
                    }
                    allowed
                }
            } else {
                true
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let life = record.borrow().life.clone();
            life.close();
            invoke_main_tick();
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let state = {
                let mut rec = record.borrow_mut();
                rec.sync_traffic_lights();
                rec.sync_geometry_from_appkit();
                rec.sync_window_state()
            };
            if let Some(state) = state {
                WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
            }
            WindowRecord::present_if_dirty(&record);
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let state = {
                let mut rec = record.borrow_mut();
                rec.sync_traffic_lights();
                rec.sync_window_state()
            };
            if let Some(state) = state {
                WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
            }
            invoke_main_tick();
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let state = {
                let mut rec = record.borrow_mut();
                rec.resize_drag = None;
                rec.sync_traffic_lights();
                rec.sync_window_state()
            };
            if let Some(state) = state {
                WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
            }
            invoke_main_tick();
        }

        #[unsafe(method(windowWillEnterFullScreen:))]
        fn window_will_enter_full_screen(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let mut rec = record.borrow_mut();
            rec.restore_traffic_lights();
            rec.apply_titlebar_appearance(rec.life.client_decorations(), true);
        }

        #[unsafe(method(windowDidEnterFullScreen:))]
        fn window_did_enter_full_screen(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let state = {
                let mut rec = record.borrow_mut();
                rec.sync_geometry_from_appkit();
                rec.sync_window_state()
            };
            if let Some(state) = state {
                WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
            }
            invoke_main_tick();
        }

        #[unsafe(method(windowWillExitFullScreen:))]
        fn window_will_exit_full_screen(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let rec = record.borrow();
            rec.apply_titlebar_appearance(rec.life.client_decorations(), false);
        }

        #[unsafe(method(windowDidExitFullScreen:))]
        fn window_did_exit_full_screen(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let state = {
                let mut rec = record.borrow_mut();
                rec.sync_traffic_lights();
                rec.sync_geometry_from_appkit();
                rec.sync_window_state()
            };
            if let Some(state) = state {
                WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
            }
            invoke_main_tick();
        }

        #[unsafe(method(windowDidChangeBackingProperties:))]
        fn window_did_change_backing_properties(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let mut rec = record.borrow_mut();
            rec.sync_geometry_from_appkit();
            drop(rec);
            WindowRecord::present_if_dirty(&record);
        }

        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn window_did_change_occlusion_state(&self, _notification: &NSNotification) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            record.borrow_mut().sync_display_link();
        }
    }
);

struct ViewIvars {
    record: RefCell<Weak<RefCell<WindowRecord>>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ViewIvars]
    struct SolorView;

    unsafe impl NSObjectProtocol for SolorView {}

    impl SolorView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(_opaqueRectForWindowMoveWhenInTitlebar))]
        fn opaque_rect_for_window_move_when_in_titlebar(&self) -> NSRect {
            let owns_drag = self
                .ivars()
                .record
                .borrow()
                .upgrade()
                .is_some_and(|record| record.borrow().life.client_decorations());
            if owns_drag {
                self.bounds()
            } else {
                NSRect::ZERO
            }
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            #[allow(unsafe_code)]
            // SAFETY: Calling superclass `viewDidChangeEffectiveAppearance` on `NSView`.
            unsafe {
                let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance];
            }
            let record = self.ivars().record.borrow().upgrade();
            if let Some(record) = record {
                record.borrow_mut().sync_traffic_lights();
            }
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            #[allow(unsafe_code)]
            // SAFETY: Calling superclass `resetCursorRects` on `NSView`.
            unsafe {
                let _: () = msg_send![super(self), resetCursorRects];
            }
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let mtm = MainThreadMarker::from(self);
            let cursor = record.borrow().cursor.current(mtm);
            self.addCursorRect_cursor(self.bounds(), &cursor);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Down);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Up);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Down);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Up);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Down);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.handle_mouse_button(event, KeyState::Up);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.handle_mouse_move(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_move(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_move(event);
        }

        #[unsafe(method(otherMouseDragged:))]
        fn other_mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_move(event);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            {
                let mut rec = record.borrow_mut();
                rec.cursor_inside = false;
            }
            WindowRecord::dispatch_event(&record, WindowEvent::Pointer(PointerInput::Leave(MOUSE_POINTER_ID)));
            invoke_main_tick();
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let (blocked, view_height) = {
                let rec = record.borrow();
                (rec.life.blocked(), f64::from(rec.size.height))
            };
            if blocked {
                return;
            }
            let local_point = self.convertPoint_fromView(event.locationInWindow(), None);
            let sample = pointer_sample_from_event_in_view(event, local_point, view_height);
            let delta = scroll_delta_from_raw(
                event.scrollingDeltaX(),
                event.scrollingDeltaY(),
                event.hasPreciseScrollingDeltas(),
            );
            WindowRecord::dispatch_event(&record, WindowEvent::Pointer(PointerInput::Scroll(sample, delta)));
            invoke_main_tick();
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            self.handle_key(event, true)
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let _ = self.handle_key(event, false);
        }

        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            if record.borrow().life.blocked() {
                return;
            }
            if let Some(key_event) = key_event_from_ns_event(event, KeyState::Up) {
                WindowRecord::dispatch_event(&record, WindowEvent::Key(key_event));
                invoke_main_tick();
            }
        }

        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let transition = {
                let mut rec = record.borrow_mut();
                if rec.life.blocked() {
                    return;
                }
                let prev = rec.last_modifiers;
                let curr = event.modifierFlags();
                rec.last_modifiers = curr;
                modifier_key_transition(prev, curr)
            };
            if let Some(key_event) = transition {
                WindowRecord::dispatch_event(&record, WindowEvent::Key(key_event));
                invoke_main_tick();
            }
        }
    }

    unsafe impl NSTextInputClient for SolorView {
        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            self.ivars()
                .record
                .borrow()
                .upgrade()
                .is_some_and(|record| record.borrow().has_marked_text)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return NSRange::new(NSNotFound as usize, 0);
            };
            let mut rec = record.borrow_mut();
            if !rec.has_marked_text {
                return NSRange::new(NSNotFound as usize, 0);
            }
            let base = rec
                .surrounding_utf16_selection()
                .map_or(0, |(start, _)| start);
            NSRange::new(base, rec.marked_utf16_len)
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return NSRange::new(NSNotFound as usize, 0);
            };
            let mut rec = record.borrow_mut();
            match rec.surrounding_utf16_selection() {
                Some((start, len)) => NSRange::new(start, len),
                None => NSRange::new(NSNotFound as usize, 0),
            }
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        unsafe fn attributed_substring_for_proposed_range(
            &self,
            range: NSRange,
            actual_range: NSRangePointer,
        ) -> Option<Retained<NSAttributedString>> {
            #[allow(unsafe_code)]
            // SAFETY: Caller passed a valid `actual_range` pointer or null per `NSTextInputClient`.
            unsafe {
                self.attributed_substring(range, actual_range)
            }
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        unsafe fn first_rect_for_character_range(
            &self,
            _range: NSRange,
            _actual_range: NSRangePointer,
        ) -> NSRect {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return NSRect::ZERO;
            };
            let rec = record.borrow();
            let Some(caret) = rec.view.as_ref().and_then(WindowView::caret_bounds) else {
                return NSRect::ZERO;
            };
            let view_h = f64::from(rec.size.height);
            let local_y = view_h - f64::from(caret.origin.y) - f64::from(caret.size.height);
            let view_rect = NSRect::new(
                NSPoint::new(f64::from(caret.origin.x), local_y),
                NSSize::new(f64::from(caret.size.width).max(1.0), f64::from(caret.size.height).max(1.0)),
            );
            let win_rect = self.convertRect_toView(view_rect, None);
            rec.ns_window.convertRectToScreen(win_rect)
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> usize {
            NSNotFound as usize
        }

        #[unsafe(method(insertText:replacementRange:))]
        unsafe fn insert_text_replacement_range(
            &self,
            string: &AnyObject,
            _replacement_range: NSRange,
        ) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let text = extract_ns_text(string);
            {
                let mut rec = record.borrow_mut();
                if rec.ime_active_dispatch {
                    rec.ime_consumed = true;
                }
                rec.has_marked_text = false;
                rec.marked_utf16_len = 0;
            }
            WindowRecord::dispatch_event(&record, WindowEvent::TextInput(TextInputEvent::Commit(text)));
            invoke_main_tick();
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        unsafe fn set_marked_text_selected_range_replacement_range(
            &self,
            string: &AnyObject,
            selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let text = extract_ns_text(string);
            let utf16_len = text.encode_utf16().count();
            {
                let mut rec = record.borrow_mut();
                if rec.ime_active_dispatch {
                    rec.ime_consumed = true;
                }
                rec.has_marked_text = !text.is_empty();
                rec.marked_utf16_len = utf16_len;
            }
            let selection = utf16_range_to_bytes(&text, selected_range).map(|(anchor, head)| {
                TextSelection { anchor, head }
            });
            WindowRecord::dispatch_event(
                &record,
                WindowEvent::TextInput(TextInputEvent::Preedit(Preedit { text, selection })),
            );
            invoke_main_tick();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let was_marked = {
                let mut rec = record.borrow_mut();
                rec.marked_utf16_len = 0;
                std::mem::replace(&mut rec.has_marked_text, false)
            };
            if was_marked {
                WindowRecord::dispatch_event(
                    &record,
                    WindowEvent::TextInput(TextInputEvent::Preedit(Preedit {
                        text: String::new(),
                        selection: None,
                    })),
                );
                invoke_main_tick();
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        unsafe fn do_command_by_selector(&self, _selector: Sel) {
            // Returning without setting `ime_consumed` lets `handle_key` route the keystroke
            // as a regular `WindowEvent::Key` without triggering an AppKit beep.
        }
    }

    unsafe impl NSDraggingDestination for SolorView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.update_file_drop_hover(sender)
        }

        #[unsafe(method(draggingUpdated:))]
        fn dragging_updated(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            self.update_file_drop_hover(sender)
        }

        #[unsafe(method(draggingExited:))]
        fn dragging_exited(&self, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
            let Some(record) = self.ivars().record.borrow().upgrade() else {
                return;
            };
            let prev = record.borrow_mut().file_drop_hover.take();
            if let Some(target) = prev {
                WindowRecord::dispatch_event(
                    &record,
                    WindowEvent::FileDropHover {
                        target,
                        hovered: false,
                    },
                );
                invoke_main_tick();
            }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            self.perform_file_drop(sender)
        }
    }
);

impl SolorView {
    #[allow(unsafe_code)]
    unsafe fn attributed_substring(
        &self,
        range: NSRange,
        actual_range: NSRangePointer,
    ) -> Option<Retained<NSAttributedString>> {
        let record = self.ivars().record.borrow().upgrade()?;
        let mut rec = record.borrow_mut();
        let handler = rec.view.as_mut()?;
        let surrounding = handler.surrounding_text(MAX_SURROUNDING_BYTES)?;
        let text = surrounding.text();
        let (utf16, byte_start, byte_end) = slice_utf16_range(text, range)?;
        if !actual_range.is_null() {
            let actual_loc = text[..byte_start].encode_utf16().count();
            let actual_len = text[byte_start..byte_end].encode_utf16().count();
            // SAFETY: Caller passed a non-null `NSRangePointer` per `NSTextInputClient`.
            unsafe {
                actual_range.write(NSRange::new(actual_loc, actual_len));
            }
        }
        Some(NSAttributedString::initWithString(
            NSAttributedString::alloc(),
            &NSString::from_str(&utf16),
        ))
    }

    fn handle_mouse_button(&self, event: &NSEvent, state: KeyState) {
        let Some(record) = self.ivars().record.borrow().upgrade() else {
            return;
        };
        if state == KeyState::Up {
            let mut rec = record.borrow_mut();
            if rec.resize_drag.take().is_some() {
                return;
            }
        }
        let (blocked, view_height) = {
            let rec = record.borrow();
            (rec.life.blocked(), f64::from(rec.size.height))
        };
        if blocked {
            return;
        }
        let Some(button) = pointer_button_from_number(event.buttonNumber()) else {
            return;
        };
        let local_point = self.convertPoint_fromView(event.locationInWindow(), None);
        let sample = pointer_sample_from_event_in_view(event, local_point, view_height);
        let input = match state {
            KeyState::Down => PointerInput::Down(sample, button),
            KeyState::Up => PointerInput::Up(sample, button),
        };
        WindowRecord::dispatch_event(&record, WindowEvent::Pointer(input));
        invoke_main_tick();
    }

    fn handle_mouse_move(&self, event: &NSEvent) {
        let Some(record) = self.ivars().record.borrow().upgrade() else {
            return;
        };
        let resize_drag = record.borrow().resize_drag;
        if let Some(drag) = resize_drag {
            apply_resize_drag(&record, drag);
            return;
        }
        let (blocked, view_height) = {
            let mut rec = record.borrow_mut();
            rec.cursor_inside = true;
            (rec.life.blocked(), f64::from(rec.size.height))
        };
        if blocked {
            return;
        }
        let local_point = self.convertPoint_fromView(event.locationInWindow(), None);
        let sample = pointer_sample_from_event_in_view(event, local_point, view_height);
        WindowRecord::dispatch_event(&record, WindowEvent::Pointer(PointerInput::Move(sample)));
        invoke_main_tick();
    }

    fn handle_key(&self, event: &NSEvent, key_equivalent: bool) -> bool {
        let Some(record) = self.ivars().record.borrow().upgrade() else {
            return false;
        };
        if record.borrow().life.blocked() {
            return false;
        }
        let Some(key_event) = key_event_from_ns_event(event, KeyState::Down) else {
            return false;
        };
        {
            let mut rec = record.borrow_mut();
            if key_equivalent {
                rec.last_key_equivalent = Some(key_event.clone());
            } else if rec.last_key_equivalent.take().as_ref() == Some(&key_event) {
                return false;
            }
        }

        // Let standard macOS application shortcuts (Cmd+Q / Cmd+H / Cmd+M / Cmd+W) pass to the
        // menu bar unless a handler explicitly binds them.
        let is_system_app_shortcut = key_event.modifiers.meta
            && !key_event.modifiers.control
            && !key_event.modifiers.alt
            && matches!(
                key_event.key,
                solor::keyboard::Key::Character('q' | 'h' | 'm' | 'w')
            );
        if key_equivalent && is_system_app_shortcut {
            return false;
        }

        let (has_text_session, has_marked) = {
            let rec = record.borrow();
            (
                rec.view
                    .as_ref()
                    .and_then(WindowView::text_input_session)
                    .is_some(),
                rec.has_marked_text,
            )
        };
        let printable_for_ime = !key_event.modifiers.control
            && !key_event.modifiers.meta
            && matches!(
                key_event.key,
                solor::keyboard::Key::Character(_) | solor::keyboard::Key::Text(_)
            );

        if has_text_session
            && (has_marked || printable_for_ime)
            && !key_equivalent
            && let Some(input_context) = self.inputContext()
        {
            {
                let mut rec = record.borrow_mut();
                rec.ime_active_dispatch = true;
                rec.ime_consumed = false;
            }
            let handled = input_context.handleEvent(event);
            let ime_consumed = {
                let mut rec = record.borrow_mut();
                rec.ime_active_dispatch = false;
                std::mem::replace(&mut rec.ime_consumed, false)
            };
            if handled && ime_consumed {
                return true;
            }
        }

        WindowRecord::dispatch_event(&record, WindowEvent::Key(key_event));
        invoke_main_tick();
        true
    }

    fn update_file_drop_hover(
        &self,
        sender: &ProtocolObject<dyn NSDraggingInfo>,
    ) -> NSDragOperation {
        let Some(record) = self.ivars().record.borrow().upgrade() else {
            return NSDragOperation::None;
        };
        let (blocked, view_height, prev) = {
            let rec = record.borrow();
            (
                rec.life.blocked(),
                f64::from(rec.size.height),
                rec.file_drop_hover,
            )
        };
        if blocked || dropped_file_paths(sender).is_empty() {
            return NSDragOperation::None;
        }
        let local_point = self.convertPoint_fromView(sender.draggingLocation(), None);
        let pos = view_point_to_solor(local_point, view_height);
        let next = {
            let rec = record.borrow();
            rec.view.as_ref().and_then(|v| v.file_drop_target(pos))
        };
        if prev != next {
            if let Some(old) = prev {
                WindowRecord::dispatch_event(
                    &record,
                    WindowEvent::FileDropHover {
                        target: old,
                        hovered: false,
                    },
                );
            }
            if let Some(new) = next {
                WindowRecord::dispatch_event(
                    &record,
                    WindowEvent::FileDropHover {
                        target: new,
                        hovered: true,
                    },
                );
            }
            record.borrow_mut().file_drop_hover = next;
            invoke_main_tick();
        }
        if next.is_some() {
            NSDragOperation::Copy
        } else {
            NSDragOperation::None
        }
    }

    fn perform_file_drop(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
        let Some(record) = self.ivars().record.borrow().upgrade() else {
            return false;
        };
        let paths = dropped_file_paths(sender);
        if paths.is_empty() {
            return false;
        }
        let pos = {
            let rec = record.borrow();
            if rec.life.blocked() {
                return false;
            }
            let local_point = self.convertPoint_fromView(sender.draggingLocation(), None);
            view_point_to_solor(local_point, f64::from(rec.size.height))
        };
        let target = {
            let rec = record.borrow();
            rec.view.as_ref().and_then(|v| v.file_drop_target(pos))
        };
        let prev = record.borrow_mut().file_drop_hover.take();
        if let Some(prev_target) = prev {
            WindowRecord::dispatch_event(
                &record,
                WindowEvent::FileDropHover {
                    target: prev_target,
                    hovered: false,
                },
            );
        }
        let Some(target) = target else {
            invoke_main_tick();
            return false;
        };
        let handled =
            WindowRecord::with_view(&record, |v| v.drop_files(target, &paths)).unwrap_or(false);
        if handled {
            record.borrow_mut().dirty = true;
        }
        invoke_main_tick();
        handled
    }
}

impl WindowRecord {
    pub(crate) fn create(
        options: WindowOptions<'_>,
        modal_parent: Option<&Rc<RefCell<Self>>>,
        revisit: Rc<Cell<bool>>,
        wake: Arc<MacWake>,
        mtm: MainThreadMarker,
        build: impl FnOnce(WindowHandle) -> Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> Result<Rc<RefCell<Self>>> {
        let wake_clone = wake.clone();
        let life = WindowLife::new(
            revisit.clone(),
            modal_parent
                .filter(|_| options.modal)
                .map(|p| Rc::downgrade(&p.borrow().life))
                .unwrap_or_default(),
            !options.native_decorations,
            Rc::new(move || wake_clone.schedule_tick()),
        );
        let handle = life.handle();
        let mut view = WindowView::Cpu(build(handle)?);
        view.set_clipboard(Rc::new(MacClipboard));
        view.set_window_resizable(options.resizable)?;

        let backing_layer = CALayer::layer();
        let (ns_window, ns_view) = init_native_window_and_view(options, &backing_layer, mtm)?;
        let presenter = CpuLayerPresenter::new()?;
        Ok(Self::finalize_created(
            life,
            options,
            modal_parent,
            revisit,
            wake,
            mtm,
            ns_window,
            ns_view,
            Some(presenter),
            view,
        ))
    }

    #[cfg(feature = "gpu")]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create_gpu<R: solor::Render>(
        gpu: &Rc<RefCell<crate::gpu::GpuContext>>,
        options: WindowOptions<'_>,
        modal_parent: Option<&Rc<RefCell<Self>>>,
        revisit: Rc<Cell<bool>>,
        wake: Arc<MacWake>,
        mtm: MainThreadMarker,
        trace_submissions: bool,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<Rc<RefCell<Self>>> {
        let wake_clone = wake.clone();
        let life = WindowLife::new(
            revisit.clone(),
            modal_parent
                .filter(|_| options.modal)
                .map(|p| Rc::downgrade(&p.borrow().life))
                .unwrap_or_default(),
            !options.native_decorations,
            Rc::new(move || wake_clone.schedule_tick()),
        );
        let handle = life.handle();
        let metal_layer = objc2_quartz_core::CAMetalLayer::layer();
        let (ns_window, ns_view) = init_native_window_and_view(options, &metal_layer, mtm)?;
        metal_layer.setContentsScale(window_scale(&ns_window).0);
        let presenter = crate::gpu::Presenter::new(
            &mut gpu.borrow_mut(),
            metal_layer,
            options.background,
            trace_submissions,
        )?;
        let max_dimension = presenter.max_dimension();
        let ui_window = solor::platform::UiWindow::with_renderer(ui, root(handle), presenter)?;
        let mut view = WindowView::Gpu {
            handler: Box::new(ui_window),
            max_dimension,
        };
        view.set_clipboard(Rc::new(MacClipboard));
        view.set_window_resizable(options.resizable)?;

        Ok(Self::finalize_created(
            life,
            options,
            modal_parent,
            revisit,
            wake,
            mtm,
            ns_window,
            ns_view,
            None,
            view,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn finalize_created(
        life: Rc<WindowLife>,
        options: WindowOptions<'_>,
        modal_parent: Option<&Rc<RefCell<Self>>>,
        revisit: Rc<Cell<bool>>,
        wake: Arc<MacWake>,
        mtm: MainThreadMarker,
        ns_window: Retained<SolorWindow>,
        ns_view: Retained<SolorView>,
        presenter: Option<CpuLayerPresenter>,
        view: WindowView,
    ) -> Rc<RefCell<Self>> {
        let initial_scale = window_scale(&ns_window);
        let initial_size = view_size(&ns_view);

        let record = Rc::new(RefCell::new(Self {
            life: life.clone(),
            ns_window: ns_window.clone(),
            ns_view: ns_view.clone(),
            presenter,
            display_link: None,
            view: Some(view),
            title: options.title.to_owned(),
            background: options.background,
            resizable: options.resizable,
            min_size: options.min_size,
            max_size: options.max_size,
            size: initial_size,
            scale: initial_scale,
            dirty: true,
            full_damage: true,
            wants_animation: false,
            #[cfg(feature = "gpu")]
            acquire_retry: None,
            failure: None,
            start: Instant::now(),
            cursor: CursorState::default(),
            cursor_inside: false,
            last_modifiers: NSEventModifierFlags::empty(),
            last_key_equivalent: None,
            ime_active_dispatch: false,
            ime_consumed: false,
            has_marked_text: false,
            marked_utf16_len: 0,
            resize_drag: None,
            traffic_light_frames: None,
            blur_rects: Vec::new(),
            blur_dirty: false,
            blur_views: Vec::new(),
            file_drop_hover: None,
            presented_once: false,
            revisit,
            wake,
        }));

        *ns_window.ivars().record.borrow_mut() = Rc::downgrade(&record);
        *ns_view.ivars().record.borrow_mut() = Rc::downgrade(&record);
        let delegate: &ProtocolObject<dyn NSWindowDelegate> = ProtocolObject::from_ref(&*ns_window);
        ns_window.setDelegate(Some(delegate));

        let weak_for_link = Rc::downgrade(&record);
        let display_link = DisplayLink::for_view(
            &ns_view,
            Rc::new(move || {
                if let Some(rec) = weak_for_link.upgrade() {
                    let elapsed = rec.borrow().start.elapsed();
                    WindowRecord::dispatch_event(&rec, WindowEvent::FrameReady(elapsed));
                    invoke_main_tick();
                }
            }),
            mtm,
        );
        record.borrow_mut().display_link = Some(display_link);

        if let Some(parent) = modal_parent {
            let parent_win = parent.borrow().ns_window.clone();
            if options.modal {
                life.attach_modal(&parent.borrow().life);
            }
            #[allow(unsafe_code)]
            // SAFETY: Both `parent_win` and `ns_window` are live `NSWindow` instances on main thread.
            unsafe {
                parent_win.addChildWindow_ordered(&ns_window, NSWindowOrderingMode::Above);
            }
        }

        // Present the first frame before showing the window to avoid an unrendered flash.
        WindowRecord::present_if_dirty(&record);
        ns_window.makeKeyAndOrderFront(None);
        let initial_state = {
            let mut rec = record.borrow_mut();
            rec.sync_traffic_lights();
            rec.sync_geometry_from_appkit();
            rec.sync_display_link();
            rec.sync_window_state()
        };
        if let Some(state) = initial_state {
            WindowRecord::dispatch_event(&record, WindowEvent::StateChanged(state));
        }
        WindowRecord::present_if_dirty(&record);

        record
    }

    pub(crate) fn poll_tasks(record: &Rc<RefCell<Self>>, waker: &Waker) {
        let mut view = {
            let mut rec = record.borrow_mut();
            if rec.failure.is_some() || !rec.life.is_open() {
                return;
            }
            rec.view.take()
        };
        let Some(ref mut v) = view else {
            return;
        };
        let result = v.poll_tasks(waker);
        let mut rec = record.borrow_mut();
        rec.view = view;
        if let Err(error) = result {
            rec.record_failure(error.context("polling foreground tasks"));
        }
    }

    pub(crate) fn cancel_modal_gestures(record: &Rc<RefCell<Self>>) {
        let cancel = record.borrow().life.take_pointer_cancel();
        if cancel {
            record.borrow_mut().resize_drag = None;
            Self::dispatch_event(
                record,
                WindowEvent::Pointer(PointerInput::Cancel(MOUSE_POINTER_ID)),
            );
        }
    }

    pub(crate) fn handle_activation(record: &Rc<RefCell<Self>>) {
        let ns_window = {
            let rec = record.borrow();
            (rec.life.take_activate_request() && rec.life.is_open()).then(|| rec.ns_window.clone())
        };
        if let Some(ns_window) = ns_window {
            ns_window.makeKeyAndOrderFront(None);
        }
    }

    pub(crate) fn next_redraw(&self) -> Option<Instant> {
        if self.failure.is_some() || !self.life.is_open() {
            return None;
        }
        #[cfg(feature = "gpu")]
        if let Some(retry) = self.acquire_retry {
            return Some(retry);
        }
        if self.dirty {
            return None;
        }
        self.view.as_ref().and_then(WindowView::next_redraw)
    }

    pub(crate) fn present_if_dirty(record: &Rc<RefCell<Self>>) {
        enum PresentOutcome {
            Presented {
                animate: bool,
                uploaded: bool,
            },
            #[cfg(feature = "gpu")]
            Retry,
        }

        let (should_present, size, scale, background, full_damage) = {
            let mut rec = record.borrow_mut();
            if rec.failure.is_some() || !rec.life.is_open() {
                return;
            }
            #[cfg(feature = "gpu")]
            if let Some(retry) = rec.acquire_retry {
                if Instant::now() < retry {
                    return;
                }
                rec.acquire_retry = None;
            }
            if !rec.dirty
                && rec
                    .view
                    .as_ref()
                    .and_then(WindowView::next_redraw)
                    .is_some_and(|deadline| deadline <= Instant::now())
            {
                rec.dirty = true;
            }
            if !rec.dirty {
                return;
            }
            (true, rec.size, rec.scale, rec.background, rec.full_damage)
        };
        if !should_present {
            return;
        }

        let mut view = record.borrow_mut().view.take();
        let Some(ref mut v) = view else {
            return;
        };

        let present_result: Result<PresentOutcome> = (|| match v {
            WindowView::Cpu(h) => {
                let (effective_scale, physical) =
                    budgeted_raster_target(size, scale, MAX_CPU_SURFACE_PIXELS)?;
                let mut frame = h.frame(size, effective_scale)?;
                let force_upload = full_damage || !record.borrow().presented_once;
                if force_upload {
                    frame.damage = Damage::from_rect(Clip::canvas(physical));
                }
                let rec = record.borrow();
                let layer = rec
                    .ns_view
                    .layer()
                    .context("SolorView backing CALayer missing")?;
                let presenter = rec
                    .presenter
                    .as_ref()
                    .context("CPU presenter missing on CPU window")?;
                presenter.present(
                    &layer,
                    &frame,
                    physical,
                    effective_scale,
                    background,
                    force_upload,
                )?;
                let animate = frame.request_callback;
                let uploaded = force_upload || !frame.damage.is_empty();
                drop(rec);
                h.frame_submitted()?;
                Ok(PresentOutcome::Presented { animate, uploaded })
            }
            #[cfg(feature = "gpu")]
            WindowView::Gpu {
                handler,
                max_dimension,
            } => {
                let (effective_scale, _physical) =
                    crate::gpu::budgeted_gpu_target(size, scale, *max_dimension)?;
                let Some(crate::gpu::GpuFrame {
                    queue,
                    texture,
                    request_callback,
                }) = handler.frame(size, effective_scale)?
                else {
                    return Ok(PresentOutcome::Retry);
                };
                queue.present(texture);
                handler.frame_submitted()?;
                Ok(PresentOutcome::Presented {
                    animate: request_callback,
                    uploaded: true,
                })
            }
        })();

        let mut rec = record.borrow_mut();
        rec.view = view;
        match present_result {
            Ok(PresentOutcome::Presented { animate, uploaded }) => {
                rec.dirty = false;
                rec.full_damage = false;
                rec.wants_animation = animate;
                #[cfg(feature = "gpu")]
                {
                    rec.acquire_retry = None;
                }
                if uploaded {
                    rec.presented_once = true;
                }
                rec.sync_display_link();
            }
            #[cfg(feature = "gpu")]
            Ok(PresentOutcome::Retry) => {
                rec.dirty = true;
                rec.acquire_retry = Some(Instant::now() + std::time::Duration::from_millis(16));
            }
            Err(error) => {
                rec.record_failure(error.context(format!(
                    "presenting frame ({}x{} logical @ scale {:.2})",
                    f64::from(size.width),
                    f64::from(size.height),
                    scale.0,
                )));
            }
        }
    }

    pub(crate) fn sync_blur_views(&mut self, mtm: MainThreadMarker) {
        if let Some(rects) = self.life.take_blur_region() {
            self.blur_rects = rects;
            self.blur_dirty = true;
        }
        if !self.blur_dirty || !self.life.is_open() {
            return;
        }
        self.blur_dirty = false;
        for effect_view in self.blur_views.drain(..) {
            effect_view.removeFromSuperview();
        }
        if self.blur_rects.is_empty() {
            return;
        }
        let Some(content_view) = self.ns_window.contentView() else {
            return;
        };
        let view_height = f64::from(self.size.height);
        for rect in &self.blur_rects {
            let Some(frame) = blur_rect_to_view_frame(*rect, view_height) else {
                continue;
            };
            let effect_view =
                NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
            effect_view.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
            effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            effect_view.setState(NSVisualEffectState::FollowsWindowActiveState);
            content_view.addSubview_positioned_relativeTo(
                &effect_view,
                NSWindowOrderingMode::Below,
                Some(&self.ns_view),
            );
            self.blur_views.push(effect_view);
        }
    }

    pub(crate) fn sync_cursor(&mut self, mtm: MainThreadMarker) {
        if !self.cursor_inside || !self.life.is_open() {
            return;
        }
        let cursor = if self.life.blocked() {
            solor::style::Cursor::Default
        } else {
            self.view
                .as_ref()
                .map_or(solor::style::Cursor::Default, |v| {
                    v.pointer_cursor(MOUSE_POINTER_ID)
                })
        };
        if self.cursor.sync(cursor, mtm) {
            self.ns_window.invalidateCursorRectsForView(&self.ns_view);
        }
    }

    pub(crate) fn take_failure(&mut self) -> Option<anyhow::Error> {
        self.failure.take()
    }

    pub(crate) fn destroy(record: &Rc<RefCell<Self>>) {
        let (ns_window, ns_view, view, display_link, blur_views) = {
            let mut rec = record.borrow_mut();
            rec.life.close();
            (
                rec.ns_window.clone(),
                rec.ns_view.clone(),
                rec.view.take(),
                rec.display_link.take(),
                std::mem::take(&mut rec.blur_views),
            )
        };
        for effect_view in blur_views {
            effect_view.removeFromSuperview();
        }
        drop(display_link);
        drop(view);
        ns_window.setDelegate(None);
        *ns_window.ivars().record.borrow_mut() = Weak::new();
        *ns_view.ivars().record.borrow_mut() = Weak::new();
        if let Some(parent) = ns_window.parentWindow() {
            parent.removeChildWindow(&ns_window);
        }
        ns_window.orderOut(None);
        ns_window.close();
    }

    fn dispatch_event(record: &Rc<RefCell<Self>>, event: WindowEvent) {
        let is_ready = {
            let rec = record.borrow();
            rec.failure.is_none() && rec.life.is_open()
        };
        if !is_ready {
            return;
        }
        let mut view = record.borrow_mut().view.take();
        let result = view.as_mut().map(|v| v.event(event));
        record.borrow_mut().view = view;

        let Some(result) = result else {
            return;
        };
        match result {
            Ok(EventResponse {
                redraw,
                window_control,
            }) => {
                {
                    let mut rec = record.borrow_mut();
                    rec.dirty |= redraw;
                }
                if let Some(control) = window_control
                    && !record.borrow().life.blocked()
                {
                    Self::apply_window_control(record, control);
                }
            }
            Err(error) => {
                record.borrow_mut().record_failure(error);
            }
        }
    }

    fn with_view<R>(
        record: &Rc<RefCell<Self>>,
        f: impl FnOnce(&mut WindowView) -> Result<R>,
    ) -> Option<R> {
        let mut view = record.borrow_mut().view.take()?;
        let result = f(&mut view);
        record.borrow_mut().view = Some(view);
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                record.borrow_mut().record_failure(error);
                None
            }
        }
    }

    fn apply_window_control(record: &Rc<RefCell<Self>>, control: WindowControl) {
        let (ns_window, resizable, life) = {
            let rec = record.borrow();
            (rec.ns_window.clone(), rec.resizable, rec.life.clone())
        };
        match control {
            WindowControl::Resize(_) | WindowControl::ToggleMaximize if !resizable => {}
            WindowControl::Drag => {
                let mtm = MainThreadMarker::from(&*ns_window);
                let app = NSApplication::sharedApplication(mtm);
                if let Some(event) = app.currentEvent() {
                    if event.clickCount() == 2 && resizable {
                        ns_window.zoom(None);
                    } else {
                        ns_window.performWindowDragWithEvent(&event);
                    }
                    Self::dispatch_event(
                        record,
                        WindowEvent::Pointer(PointerInput::Cancel(MOUSE_POINTER_ID)),
                    );
                }
            }
            WindowControl::Resize(edge) => {
                let start_mouse = NSEvent::mouseLocation();
                let start_frame = ns_window.frame();
                record.borrow_mut().resize_drag = Some(ResizeDrag {
                    edge,
                    start_mouse,
                    start_frame,
                });
            }
            WindowControl::Minimize => {
                ns_window.miniaturize(None);
            }
            WindowControl::ToggleMaximize => {
                ns_window.zoom(None);
            }
            WindowControl::ToggleDecorations => {
                if ns_window
                    .styleMask()
                    .contains(NSWindowStyleMask::FullScreen)
                {
                    return;
                }
                let currently_csd = life.client_decorations();
                let next_native = currently_csd;
                if next_native {
                    record.borrow_mut().restore_traffic_lights();
                }
                let old_frame = ns_window.frame();
                let current_content = ns_window.contentRectForFrameRect(old_frame);
                let new_mask = style_mask_for(next_native, resizable);
                let new_frame = anchor_frame_top(
                    old_frame,
                    NSWindow::frameRectForContentRect_styleMask(
                        current_content,
                        new_mask,
                        MainThreadMarker::from(&*ns_window),
                    ),
                );
                ns_window.setStyleMask(new_mask);
                apply_window_titlebar_appearance(&ns_window, !next_native, false);
                ns_window.setFrame_display(new_frame, true);
                let (title, ns_view) = {
                    let rec = record.borrow();
                    (rec.title.clone(), rec.ns_view.clone())
                };
                ns_window.setTitle(&NSString::from_str(&title));
                ns_window.makeFirstResponder(Some(&ns_view));
                life.set_client_decorations(!next_native);
                let state = {
                    let mut rec = record.borrow_mut();
                    rec.sync_traffic_lights();
                    rec.sync_geometry_from_appkit();
                    rec.dirty = true;
                    rec.full_damage = true;
                    rec.sync_window_state()
                };
                if let Some(state) = state {
                    Self::dispatch_event(record, WindowEvent::StateChanged(state));
                }
            }
            WindowControl::Close => {
                life.request_close();
            }
        }
    }

    fn is_fullscreen(&self) -> bool {
        self.ns_window
            .styleMask()
            .contains(NSWindowStyleMask::FullScreen)
    }

    fn apply_titlebar_appearance(&self, integrated: bool, fullscreen: bool) {
        apply_window_titlebar_appearance(&self.ns_window, integrated, fullscreen);
    }

    fn traffic_light_buttons(
        &self,
    ) -> Option<(Retained<NSButton>, Retained<NSButton>, Retained<NSButton>)> {
        Some((
            self.ns_window
                .standardWindowButton(NSWindowButton::CloseButton)?,
            self.ns_window
                .standardWindowButton(NSWindowButton::MiniaturizeButton)?,
            self.ns_window
                .standardWindowButton(NSWindowButton::ZoomButton)?,
        ))
    }

    fn titlebar_container(close: &NSButton) -> Option<Retained<NSView>> {
        #[allow(unsafe_code)]
        // SAFETY: `close` is a live `standardWindowButton` managed by AppKit on the main thread;
        // walking its two parent containers accesses AppKit's titlebar view hierarchy.
        unsafe {
            let button_container = close.superview()?;
            button_container.superview()
        }
    }

    fn sync_traffic_lights(&mut self) {
        if !self.life.client_decorations() || self.is_fullscreen() {
            self.restore_traffic_lights();
            return;
        }
        let Some((close, minimize, zoom)) = self.traffic_light_buttons() else {
            return;
        };
        let Some(titlebar_container) = Self::titlebar_container(&close) else {
            return;
        };
        if self.traffic_light_frames.is_none() {
            self.traffic_light_frames = Some(TrafficLightFrames {
                titlebar: titlebar_container.frame(),
                close: close.frame(),
                minimize: minimize.frame(),
                zoom: zoom.frame(),
            });
        }
        let window_height = self.ns_window.frame().size.height;
        let Some(layout) = compute_traffic_light_layout(
            window_height,
            close.frame(),
            minimize.frame(),
            titlebar_container.frame(),
            DEFAULT_INTEGRATED_TITLEBAR_HEIGHT,
            DEFAULT_TRAFFIC_LIGHT_LEFT_INSET,
        ) else {
            return;
        };

        titlebar_container.setFrame(layout.titlebar_frame);
        close.setFrameOrigin(layout.close_origin);
        minimize.setFrameOrigin(layout.minimize_origin);
        zoom.setFrameOrigin(layout.zoom_origin);

        titlebar_container.updateTrackingAreas();
        close.updateTrackingAreas();
        minimize.updateTrackingAreas();
        zoom.updateTrackingAreas();
    }

    fn restore_traffic_lights(&mut self) {
        let Some(frames) = self.traffic_light_frames.take() else {
            return;
        };
        let Some((close, minimize, zoom)) = self.traffic_light_buttons() else {
            return;
        };
        let Some(titlebar_container) = Self::titlebar_container(&close) else {
            return;
        };
        close.setFrame(frames.close);
        minimize.setFrame(frames.minimize);
        zoom.setFrame(frames.zoom);
        titlebar_container.setFrame(frames.titlebar);

        titlebar_container.updateTrackingAreas();
        close.updateTrackingAreas();
        minimize.updateTrackingAreas();
        zoom.updateTrackingAreas();
    }

    fn sync_geometry_from_appkit(&mut self) {
        let new_size = view_size(&self.ns_view);
        let new_scale = window_scale(&self.ns_window);
        if new_size != self.size || new_scale != self.scale {
            if new_size != self.size && !self.blur_rects.is_empty() {
                self.blur_dirty = true;
            }
            self.size = new_size;
            self.scale = new_scale;
            self.dirty = true;
            self.full_damage = true;
            #[cfg(feature = "gpu")]
            {
                self.acquire_retry = None;
            }
        }
    }

    fn sync_window_state(&self) -> Option<WindowState> {
        let style_mask = self.ns_window.styleMask();
        let state = WindowState {
            active: self.ns_window.isKeyWindow(),
            maximized: self.ns_window.isZoomed(),
            fullscreen: style_mask.contains(NSWindowStyleMask::FullScreen),
        };
        self.life.confirm_state(state).then_some(state)
    }

    fn sync_display_link(&mut self) {
        let visible = self
            .ns_window
            .occlusionState()
            .contains(objc2_app_kit::NSWindowOcclusionState::Visible);
        if let Some(link) = self.display_link.as_mut() {
            link.set_active(visible && self.wants_animation);
        }
    }

    fn surrounding_utf16_selection(&mut self) -> Option<(usize, usize)> {
        let handler = self.view.as_mut()?;
        let surrounding = handler.surrounding_text(MAX_SURROUNDING_BYTES)?;
        let text = surrounding.text();
        let range = surrounding.selection().range();
        let start = text[..range.start].encode_utf16().count();
        let len = text[range].encode_utf16().count();
        Some((start, len))
    }

    fn record_failure(&mut self, error: anyhow::Error) {
        self.failure.get_or_insert(error);
        self.revisit.set(true);
        self.wake.schedule_tick();
    }
}

fn init_native_window_and_view(
    options: WindowOptions<'_>,
    backing_layer: &CALayer,
    mtm: MainThreadMarker,
) -> Result<(Retained<SolorWindow>, Retained<SolorView>)> {
    let style_mask = style_mask_for(options.native_decorations, options.resizable);
    let content_rect = centered_content_rect(options.size, mtm);

    let ns_window = mtm.alloc::<SolorWindow>().set_ivars(WindowIvars {
        record: RefCell::new(Weak::new()),
    });
    #[allow(unsafe_code)]
    // SAFETY: Valid `NSWindow` initializer with `Buffered` backing store on main thread.
    let ns_window: Retained<SolorWindow> = unsafe {
        msg_send![
            super(ns_window),
            initWithContentRect: content_rect,
            styleMask: style_mask,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    #[allow(unsafe_code)]
    // SAFETY: `SolorWindow` retains ownership via `Retained`; prevent AppKit double-release on close.
    unsafe {
        ns_window.setReleasedWhenClosed(false);
    }
    ns_window.setTitle(&NSString::from_str(options.title));
    apply_window_titlebar_appearance(&ns_window, !options.native_decorations, false);
    ns_window.setAcceptsMouseMovedEvents(true);
    if let Some(min) = options.min_size {
        ns_window.setContentMinSize(NSSize::new(f64::from(min.width), f64::from(min.height)));
    }
    if let Some(max) = options.max_size {
        ns_window.setContentMaxSize(NSSize::new(f64::from(max.width), f64::from(max.height)));
    }
    if options.background == WindowBackground::Transparent {
        ns_window.setOpaque(false);
        ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    } else {
        ns_window.setOpaque(true);
        ns_window.setBackgroundColor(Some(&NSColor::whiteColor()));
    }
    let level: NSWindowLevel = if options.always_on_top {
        NSFloatingWindowLevel
    } else {
        NSNormalWindowLevel
    };
    ns_window.setLevel(level);

    let content_view = ns_window
        .contentView()
        .context("NSWindow created without a contentView")?;
    let bounds = content_view.bounds();
    let ns_view = mtm.alloc::<SolorView>().set_ivars(ViewIvars {
        record: RefCell::new(Weak::new()),
    });
    #[allow(unsafe_code)]
    // SAFETY: Calling `NSView::initWithFrame:` on a freshly allocated `SolorView`.
    let ns_view: Retained<SolorView> = unsafe { msg_send![super(ns_view), initWithFrame: bounds] };
    ns_view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    ns_view.setLayer(Some(backing_layer));
    ns_view.setWantsLayer(true);
    ns_view.setLayerContentsRedrawPolicy(NSViewLayerContentsRedrawPolicy::DuringViewResize);

    let tracking_options = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveAlways
        | NSTrackingAreaOptions::InVisibleRect;
    #[allow(unsafe_code)]
    // SAFETY: `ns_view` owns the tracking area and outlives it.
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            tracking_options,
            Some(&ns_view),
            None,
        )
    };
    ns_view.addTrackingArea(&tracking);

    #[allow(unsafe_code)]
    // SAFETY: `NSPasteboardTypeFileURL` is a static AppKit constant.
    let file_url_type = unsafe { NSPasteboardTypeFileURL };
    let dragged_types = NSArray::from_slice(&[file_url_type]);
    ns_view.registerForDraggedTypes(&dragged_types);

    content_view.addSubview(&ns_view);
    ns_window.makeFirstResponder(Some(&ns_view));

    Ok((ns_window, ns_view))
}

fn blur_rect_to_view_frame(rect: SurfaceRect, view_height: f64) -> Option<NSRect> {
    let width = f64::from(rect.size.width);
    let height = f64::from(rect.size.height);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let x = f64::from(rect.origin.x);
    let y = view_height - f64::from(rect.origin.y) - height;
    Some(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)))
}

fn style_mask_for(native_decorations: bool, resizable: bool) -> NSWindowStyleMask {
    let mut mask =
        NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable;
    if !native_decorations {
        mask |= NSWindowStyleMask::FullSizeContentView;
    }
    if resizable {
        mask |= NSWindowStyleMask::Resizable;
    }
    mask
}

fn is_macos_version_at_least(major: isize, minor: isize, patch: isize) -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: major,
        minorVersion: minor,
        patchVersion: patch,
    })
}

fn apply_window_titlebar_appearance(window: &NSWindow, integrated: bool, fullscreen: bool) {
    if integrated {
        // On macOS 15.3+, keeping `titlebarAppearsTransparent` enabled inside fullscreen
        // leaves an artifact strip at the top; restore transparency when exiting fullscreen.
        let transparent = !(fullscreen && is_macos_version_at_least(15, 3, 0));
        window.setTitlebarAppearsTransparent(transparent);
        window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    } else {
        window.setTitlebarAppearsTransparent(false);
        window.setTitleVisibility(NSWindowTitleVisibility::Visible);
    }
}

fn compute_traffic_light_layout(
    window_height: f64,
    close_frame: NSRect,
    minimize_frame: NSRect,
    titlebar_frame: NSRect,
    target_titlebar_height: f64,
    left_inset: f64,
) -> Option<TrafficLightLayout> {
    if close_frame.size.width <= 0.0
        || close_frame.size.height <= 0.0
        || minimize_frame.origin.x <= close_frame.origin.x
        || window_height <= 0.0
    {
        return None;
    }
    let button_width = close_frame.size.width;
    let button_height = close_frame.size.height;
    let button_gap = (minimize_frame.origin.x - close_frame.origin.x - button_width).max(0.0);
    let pos_y = ((target_titlebar_height - button_height) * 0.5).max(0.0);
    let container_height = button_height + pos_y * 2.0;

    Some(TrafficLightLayout {
        titlebar_frame: NSRect::new(
            NSPoint::new(titlebar_frame.origin.x, window_height - container_height),
            NSSize::new(titlebar_frame.size.width, container_height),
        ),
        close_origin: NSPoint::new(left_inset, pos_y),
        minimize_origin: NSPoint::new(left_inset + button_width + button_gap, pos_y),
        zoom_origin: NSPoint::new(left_inset + (button_width + button_gap) * 2.0, pos_y),
    })
}

fn anchor_frame_top(old_frame: NSRect, mut new_frame: NSRect) -> NSRect {
    let old_top = old_frame.origin.y + old_frame.size.height;
    new_frame.origin.x = old_frame.origin.x;
    new_frame.origin.y = old_top - new_frame.size.height;
    new_frame
}

fn centered_content_rect(size: Size, mtm: MainThreadMarker) -> NSRect {
    let width = f64::from(size.width).max(1.0);
    let height = f64::from(size.height).max(1.0);
    if let Some(screen) = NSScreen::mainScreen(mtm) {
        let visible = screen.visibleFrame();
        let x = visible.origin.x + (visible.size.width - width).max(0.0) * 0.5;
        let y = visible.origin.y + (visible.size.height - height).max(0.0) * 0.5;
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    } else {
        NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(width, height))
    }
}

fn view_size(view: &NSView) -> Size {
    let bounds = view.bounds().size;
    Size {
        width: px((bounds.width as f32).max(1.0)),
        height: px((bounds.height as f32).max(1.0)),
    }
}

fn window_scale(window: &NSWindow) -> RasterScale {
    let factor = window.backingScaleFactor();
    if factor.is_finite() && factor > 0.0 {
        RasterScale(factor)
    } else {
        RasterScale(2.0)
    }
}

fn budgeted_raster_target(
    size: Size,
    preferred: RasterScale,
    max_pixels: u64,
) -> Result<(RasterScale, DeviceSize)> {
    let physical = preferred.buffer_size(size)?;
    if u64::from(physical.width) * u64::from(physical.height) <= max_pixels {
        return Ok((preferred, physical));
    }
    let logical_area = f64::from(size.width).max(1.0) * f64::from(size.height).max(1.0);
    let mut factor = ((max_pixels as f64) / logical_area)
        .sqrt()
        .min(preferred.0)
        .max(f64::EPSILON);
    for _ in 0..16 {
        let candidate = RasterScale(factor);
        let candidate_size = candidate.buffer_size(size)?;
        if u64::from(candidate_size.width) * u64::from(candidate_size.height) <= max_pixels {
            return Ok((candidate, candidate_size));
        }
        factor *= 0.98;
    }
    let fallback = RasterScale(factor);
    Ok((fallback, fallback.buffer_size(size)?))
}

fn apply_resize_drag(record: &Rc<RefCell<WindowRecord>>, drag: ResizeDrag) {
    let mouse = NSEvent::mouseLocation();
    let dx = mouse.x - drag.start_mouse.x;
    let dy = mouse.y - drag.start_mouse.y;

    let (ns_window, min_size, max_size) = {
        let rec = record.borrow();
        (rec.ns_window.clone(), rec.min_size, rec.max_size)
    };
    let min_w = min_size.map_or(120.0, |s| f64::from(s.width).max(1.0));
    let min_h = min_size.map_or(80.0, |s| f64::from(s.height).max(1.0));
    let max_w = max_size.map_or(f64::INFINITY, |s| f64::from(s.width).max(min_w));
    let max_h = max_size.map_or(f64::INFINITY, |s| f64::from(s.height).max(min_h));

    let mut frame = drag.start_frame;
    let right = frame.origin.x + frame.size.width;
    let top = frame.origin.y + frame.size.height;

    match drag.edge {
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
            let w = (frame.size.width - dx).clamp(min_w, max_w);
            frame.origin.x = right - w;
            frame.size.width = w;
        }
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
            frame.size.width = (frame.size.width + dx).clamp(min_w, max_w);
        }
        ResizeEdge::Top | ResizeEdge::Bottom => {}
    }

    match drag.edge {
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
            frame.size.height = (frame.size.height + dy).clamp(min_h, max_h);
        }
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight => {
            let h = (frame.size.height - dy).clamp(min_h, max_h);
            frame.origin.y = top - h;
            frame.size.height = h;
        }
        ResizeEdge::Left | ResizeEdge::Right => {}
    }

    ns_window.setFrame_display(frame, true);
}

fn extract_ns_text(object: &AnyObject) -> String {
    if let Some(attributed) = object.downcast_ref::<NSAttributedString>() {
        attributed.string().to_string()
    } else if let Some(string) = object.downcast_ref::<NSString>() {
        string.to_string()
    } else {
        String::new()
    }
}

fn slice_utf16_range(text: &str, range: NSRange) -> Option<(String, usize, usize)> {
    if range.location == NSNotFound as usize {
        return None;
    }
    let start_u16 = range.location;
    let end_u16 = start_u16.saturating_add(range.length);
    let mut u16_pos = 0usize;
    let mut byte_start = None;
    let mut byte_end = None;
    for (idx, ch) in text.char_indices() {
        if u16_pos >= start_u16 && byte_start.is_none() {
            byte_start = Some(idx);
        }
        u16_pos += ch.len_utf16();
        if u16_pos >= end_u16 {
            byte_end = Some(idx + ch.len_utf8());
            break;
        }
    }
    let start = byte_start.unwrap_or(text.len());
    let end = byte_end.unwrap_or(text.len()).max(start);
    Some((text[start..end].to_owned(), start, end))
}

fn utf16_range_to_bytes(text: &str, range: NSRange) -> Option<(usize, usize)> {
    if range.location == NSNotFound as usize {
        return None;
    }
    let start_u16 = range.location;
    let end_u16 = start_u16.saturating_add(range.length);
    let mut u16_pos = 0usize;
    let mut start_byte = None;
    let mut end_byte = None;
    for (idx, ch) in text.char_indices() {
        if u16_pos >= start_u16 && start_byte.is_none() {
            start_byte = Some(idx);
        }
        if u16_pos >= end_u16 && end_byte.is_none() {
            end_byte = Some(idx);
            break;
        }
        u16_pos += ch.len_utf16();
    }
    let start = start_byte.unwrap_or(text.len());
    let end = end_byte.unwrap_or(text.len()).max(start);
    Some((start, end))
}

fn dropped_file_paths(dragging_info: &ProtocolObject<dyn NSDraggingInfo>) -> Vec<PathBuf> {
    let pasteboard = dragging_info.draggingPasteboard();
    let classes = NSArray::from_slice(&[objc2::class!(NSURL)]);
    let options: Retained<NSDictionary<NSPasteboardReadingOptionKey, AnyObject>> =
        NSDictionary::new();
    #[allow(unsafe_code)]
    // SAFETY: `classes` contains `NSURL` class object matching `NSPasteboard::readObjectsForClasses_options`.
    let objects = unsafe { pasteboard.readObjectsForClasses_options(&classes, Some(&options)) };
    let Some(objects) = objects else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    for i in 0..objects.count() {
        let obj = objects.objectAtIndex(i);
        if let Some(url) = obj.downcast_ref::<NSURL>()
            && url.isFileURL()
            && let Some(path) = url.path()
        {
            paths.push(PathBuf::from(path.to_string()));
        }
    }
    paths
}

#[cfg(test)]
#[path = "../tests/unit/titlebar.rs"]
mod titlebar_tests;
