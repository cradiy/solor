#![cfg(target_os = "macos")]
//! Integration checks for the macOS integrated titlebar adaptation in `solor-gallery`.
use anyhow::Result;
use solor::{
    WindowControl,
    layout::{Origin, Size},
    platform::{
        PlatformWindow, WindowEvent, WindowHandle, WindowHandler, WindowState, WindowSubscription,
    },
    pointer::{PointerButton, PointerId, PointerInput, PointerKind, PointerSample},
    px,
    raster::RasterScale,
};
use solor_render_cpu::CpuWindow;
use std::{cell::RefCell, rc::Rc, task::Waker};

type BoolListener = Box<dyn FnMut(bool)>;
type StateListener = Box<dyn FnMut(WindowState)>;

struct MockMacWindow {
    client_decorations: bool,
    state: WindowState,
    decoration_listeners: Vec<Option<BoolListener>>,
    state_listeners: Vec<Option<StateListener>>,
}

impl MockMacWindow {
    fn new(client_decorations: bool) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            client_decorations,
            state: WindowState {
                active: true,
                maximized: false,
                fullscreen: false,
            },
            decoration_listeners: Vec::new(),
            state_listeners: Vec::new(),
        }))
    }

    fn set_client_decorations(window: &Rc<RefCell<Self>>, enabled: bool) {
        let mut listeners = {
            let mut inner = window.borrow_mut();
            inner.client_decorations = enabled;
            inner
                .decoration_listeners
                .iter_mut()
                .map(Option::take)
                .collect::<Vec<_>>()
        };
        for slot in &mut listeners {
            if let Some(listener) = slot.as_mut() {
                listener(enabled);
            }
        }
        let mut inner = window.borrow_mut();
        for (idx, slot) in listeners.into_iter().enumerate() {
            if idx < inner.decoration_listeners.len() {
                inner.decoration_listeners[idx] = slot;
            }
        }
    }

    fn set_fullscreen(window: &Rc<RefCell<Self>>, fullscreen: bool) {
        let (state, mut listeners) = {
            let mut inner = window.borrow_mut();
            inner.state.fullscreen = fullscreen;
            let listeners = inner
                .state_listeners
                .iter_mut()
                .map(Option::take)
                .collect::<Vec<_>>();
            (inner.state, listeners)
        };
        for slot in &mut listeners {
            if let Some(listener) = slot.as_mut() {
                listener(state);
            }
        }
        let mut inner = window.borrow_mut();
        for (idx, slot) in listeners.into_iter().enumerate() {
            if idx < inner.state_listeners.len() {
                inner.state_listeners[idx] = slot;
            }
        }
    }
}

struct MockPlatformWindow(Rc<RefCell<MockMacWindow>>);

impl PlatformWindow for MockPlatformWindow {
    fn is_open(&self) -> bool {
        true
    }

    fn client_decorations(&self) -> bool {
        self.0.borrow().client_decorations
    }

    fn observe_client_decorations(&self, listener: BoolListener) -> WindowSubscription {
        let mut inner = self.0.borrow_mut();
        let idx = inner.decoration_listeners.len();
        inner.decoration_listeners.push(Some(listener));
        let weak = Rc::downgrade(&self.0);
        WindowSubscription::new(move || {
            if let Some(owner) = weak.upgrade()
                && let Some(slot) = owner.borrow_mut().decoration_listeners.get_mut(idx)
            {
                slot.take();
            }
        })
    }

    fn state(&self) -> Option<WindowState> {
        Some(self.0.borrow().state)
    }

    fn observe_state(&self, listener: StateListener) -> WindowSubscription {
        let mut inner = self.0.borrow_mut();
        let idx = inner.state_listeners.len();
        inner.state_listeners.push(Some(listener));
        let weak = Rc::downgrade(&self.0);
        WindowSubscription::new(move || {
            if let Some(owner) = weak.upgrade()
                && let Some(slot) = owner.borrow_mut().state_listeners.get_mut(idx)
            {
                slot.take();
            }
        })
    }

    fn set_close_guard(&self, _handler: Rc<dyn Fn() -> bool>) {}

    fn close(&self) {}
}

fn mouse_at(x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(1),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

fn press_at(window: &mut CpuWindow, x: f32, y: f32) -> Result<Option<WindowControl>> {
    let sample = mouse_at(x, y);
    let down = window.event(WindowEvent::Pointer(PointerInput::Down(
        sample,
        PointerButton::Primary,
    )))?;
    let up = window.event(WindowEvent::Pointer(PointerInput::Up(
        sample,
        PointerButton::Primary,
    )))?;
    Ok(down.window_control.or(up.window_control))
}

fn render_window(window: &mut CpuWindow) -> Result<()> {
    let size = Size {
        width: px(800.0),
        height: px(560.0),
    };
    window.frame(size, RasterScale::ONE)?;
    window.frame_submitted()?;
    Ok(())
}

#[test]
fn macos_titlebar_adapts_header_and_collapses_traffic_light_spacer() -> Result<()> {
    let font = include_bytes!("../../../fixtures/fonts/NotoSansCJKsc-fixture.otf");
    let mock = MockMacWindow::new(true);
    let platform_window: Rc<dyn PlatformWindow> = Rc::new(MockPlatformWindow(mock.clone()));
    let handle = WindowHandle::new(Rc::downgrade(&platform_window));

    let (ui, prepared) = solor_gallery::Gallery::prepare(solor::Ui::with_font(font.to_vec())?)?;
    let mut window = CpuWindow::mount(ui, prepared.into_root(Some(handle)))?;
    render_window(&mut window)?;

    // No 5px manual top resize strip on macOS: top pixels belong to the draggable header.
    assert_eq!(
        press_at(&mut window, 200.0, 2.0)?,
        Some(WindowControl::Drag)
    );
    // Traffic-light spacer on the left acts as drag region rather than hosting the brand/button.
    assert_eq!(
        press_at(&mut window, 40.0, 26.0)?,
        Some(WindowControl::Drag)
    );
    // No redundant custom Close button on macOS.
    assert_ne!(
        press_at(&mut window, 755.0, 26.0)?,
        Some(WindowControl::Close)
    );

    // With integrated titlebar active (70px spacer), "Toggle frame" starts past x = 160.
    // Specifically, clicking at x = 140 lands on the brand/drag region, while x = 215 hits "Toggle frame".
    assert_eq!(
        press_at(&mut window, 140.0, 26.0)?,
        Some(WindowControl::Drag)
    );
    assert_eq!(
        press_at(&mut window, 215.0, 26.0)?,
        Some(WindowControl::ToggleDecorations)
    );

    // Switching to native decorations collapses the 70px traffic-light spacer.
    MockMacWindow::set_client_decorations(&mock, false);
    window.poll_tasks(Waker::noop())?;
    render_window(&mut window)?;
    assert_eq!(
        press_at(&mut window, 140.0, 26.0)?,
        Some(WindowControl::ToggleDecorations)
    );

    // Returning to integrated titlebar restores the spacer, and fullscreen collapses it again.
    MockMacWindow::set_client_decorations(&mock, true);
    window.poll_tasks(Waker::noop())?;
    render_window(&mut window)?;
    assert_eq!(
        press_at(&mut window, 140.0, 26.0)?,
        Some(WindowControl::Drag)
    );

    MockMacWindow::set_fullscreen(&mock, true);
    window.poll_tasks(Waker::noop())?;
    render_window(&mut window)?;
    assert_eq!(
        press_at(&mut window, 140.0, 26.0)?,
        Some(WindowControl::ToggleDecorations)
    );

    MockMacWindow::set_fullscreen(&mock, false);
    window.poll_tasks(Waker::noop())?;
    render_window(&mut window)?;
    assert_eq!(
        press_at(&mut window, 215.0, 26.0)?,
        Some(WindowControl::ToggleDecorations)
    );

    Ok(())
}
