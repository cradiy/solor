//! `NSApplication` lifecycle, main run-loop integration, and window orchestration.
use crate::{
    channel::{ApplicationSender, Poll},
    wake::{MacWake, RedrawTimer, TickGuard, invoke_main_tick},
    window::WindowRecord,
};
use anyhow::{Context as _, Result, ensure};
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSEvent, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem,
};
use objc2_foundation::{NSNotification, NSObject, NSObjectProtocol, NSPoint, NSString};
use solor::{
    platform::{Platform, PlatformWindow, WindowHandle, WindowHandler, WindowOptions},
    raster::Rgba8,
};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    sync::Arc,
    task::Waker,
};

type WindowErrorHandler = dyn Fn(WindowHandle, &anyhow::Error);

pub(crate) struct AppState {
    mtm: MainThreadMarker,
    app: Retained<NSApplication>,
    windows: Vec<Rc<RefCell<WindowRecord>>>,
    pending: Vec<Rc<RefCell<WindowRecord>>>,
    inboxes: Vec<crate::channel::Inbox>,
    #[cfg(feature = "gpu")]
    gpu: Rc<RefCell<crate::gpu::GpuContext>>,
    #[cfg(feature = "gpu")]
    trace_submissions: bool,
    error_handler: Option<Rc<WindowErrorHandler>>,
    first_error: Option<anyhow::Error>,
    revisit: Rc<Cell<bool>>,
    wake: Arc<MacWake>,
    redraw_timer: RedrawTimer,
    stopping: bool,
    ticking: bool,
}

struct WindowOpenEnv {
    modal_parent: Option<Rc<RefCell<WindowRecord>>>,
    revisit: Rc<Cell<bool>>,
    wake: Arc<MacWake>,
    mtm: MainThreadMarker,
}

impl AppState {
    fn prepare_window_open(
        state: &Rc<RefCell<Self>>,
        options: WindowOptions<'_>,
    ) -> Result<WindowOpenEnv> {
        ensure!(
            !options.modal || options.parent.is_some(),
            "a modal window requires a parent"
        );
        let modal_parent = options
            .parent
            .map(|handle| state.borrow().find_parent(handle))
            .transpose()?;
        let st = state.borrow();
        Ok(WindowOpenEnv {
            modal_parent,
            revisit: st.revisit.clone(),
            wake: st.wake.clone(),
            mtm: st.mtm,
        })
    }

    fn register_window_record(
        state: &Rc<RefCell<Self>>,
        record: Rc<RefCell<WindowRecord>>,
    ) -> WindowHandle {
        let handle = record.borrow().life.handle();
        state.borrow_mut().pending.push(record);
        wake_tick_if_idle(state);
        handle
    }

    fn open_window_record(
        state: &Rc<RefCell<Self>>,
        options: WindowOptions<'_>,
        build: impl FnOnce(WindowHandle) -> Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> Result<WindowHandle> {
        let env = Self::prepare_window_open(state, options)?;
        let record = WindowRecord::create(
            options,
            env.modal_parent.as_ref(),
            env.revisit,
            env.wake,
            env.mtm,
            build,
        )?;
        Ok(Self::register_window_record(state, record))
    }

    #[cfg(feature = "gpu")]
    fn open_gpu_window_record<R: solor::Render>(
        state: &Rc<RefCell<Self>>,
        options: WindowOptions<'_>,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<WindowHandle> {
        let env = Self::prepare_window_open(state, options)?;
        let (gpu, trace_submissions) = {
            let st = state.borrow();
            (st.gpu.clone(), st.trace_submissions)
        };
        let record = WindowRecord::create_gpu(
            &gpu,
            options,
            env.modal_parent.as_ref(),
            env.revisit,
            env.wake,
            env.mtm,
            trace_submissions,
            ui,
            root,
        )?;
        Ok(Self::register_window_record(state, record))
    }

    fn find_parent(&self, parent: &WindowHandle) -> Result<Rc<RefCell<WindowRecord>>> {
        ensure!(parent.is_open(), "parent window is closed");
        let found = self
            .windows
            .iter()
            .chain(self.pending.iter())
            .find(|rec| {
                let rec_ref = rec.borrow();
                let life_handle = rec_ref.life.handle();
                &life_handle == parent && rec_ref.life.is_open()
            })
            .cloned()
            .context("parent window does not belong to this application")?;
        Ok(found)
    }

    fn tick(state: &Rc<RefCell<Self>>) {
        {
            let mut st = state.borrow_mut();
            if st.ticking || st.stopping {
                return;
            }
            st.ticking = true;
            st.wake.clear_pending();
        }
        let waker = Waker::from(state.borrow().wake.clone());
        loop {
            let mut inboxes = std::mem::take(&mut state.borrow_mut().inboxes);
            let mut more_messages = false;
            if !inboxes.is_empty() {
                let app_handle = MacApplication(Rc::downgrade(state));
                let mut retained = Vec::with_capacity(inboxes.len());
                for mut inbox in inboxes.drain(..) {
                    match inbox(&app_handle) {
                        Poll::Idle => retained.push(inbox),
                        Poll::More => {
                            more_messages = true;
                            retained.push(inbox);
                        }
                        Poll::Closed => {}
                    }
                }
                let mut st = state.borrow_mut();
                let newly_added = std::mem::take(&mut st.inboxes);
                retained.extend(newly_added);
                st.inboxes = retained;
                if more_messages {
                    st.revisit.set(true);
                }
            }

            let (mut windows, mtm) = {
                let mut st = state.borrow_mut();
                st.revisit.set(false);
                let pending = std::mem::take(&mut st.pending);
                st.windows.extend(pending);
                (st.windows.clone(), st.mtm)
            };

            let mut closed = Vec::new();
            let mut earliest_redraw = None;

            for record in &windows {
                if !record.borrow().life.is_open() {
                    closed.push(Rc::as_ptr(record));
                    continue;
                }
                WindowRecord::cancel_modal_gestures(record);
                WindowRecord::poll_tasks(record, &waker);
                WindowRecord::handle_activation(record);
                let failure = record.borrow_mut().take_failure();
                if let Some(error) = failure {
                    Self::fail_window(state, record, error);
                }
                if !record.borrow().life.is_open() {
                    closed.push(Rc::as_ptr(record));
                    continue;
                }
                WindowRecord::present_if_dirty(record);
                let failure = record.borrow_mut().take_failure();
                if let Some(error) = failure {
                    Self::fail_window(state, record, error);
                }
                if !record.borrow().life.is_open() {
                    closed.push(Rc::as_ptr(record));
                    continue;
                }
                {
                    let mut rec = record.borrow_mut();
                    rec.sync_blur_views(mtm);
                    rec.sync_cursor(mtm);
                }
                let deadline = record.borrow().next_redraw();
                if let Some(deadline) = deadline {
                    earliest_redraw = Some(match earliest_redraw {
                        Some(old) if old <= deadline => old,
                        _ => deadline,
                    });
                }
            }

            if !closed.is_empty() {
                let mut removed = Vec::new();
                {
                    let mut st = state.borrow_mut();
                    st.windows.retain(|rec| {
                        if closed.contains(&Rc::as_ptr(rec)) {
                            removed.push(rec.clone());
                            false
                        } else {
                            true
                        }
                    });
                }
                for rec in removed {
                    WindowRecord::destroy(&rec);
                }
            }
            windows.clear();

            let should_continue = {
                let st = state.borrow();
                st.revisit.get() || !st.pending.is_empty()
            };
            if !should_continue {
                let mut st = state.borrow_mut();
                st.redraw_timer.update(earliest_redraw, mtm);
                let empty = st.windows.is_empty() && st.pending.is_empty() && st.inboxes.is_empty();
                st.ticking = false;
                if empty {
                    st.stopping = true;
                    let app = st.app.clone();
                    drop(st);
                    stop_application(&app, mtm);
                }
                break;
            }
        }
    }

    fn fail_window(
        state: &Rc<RefCell<Self>>,
        record: &Rc<RefCell<WindowRecord>>,
        error: anyhow::Error,
    ) {
        let handle = record.borrow().life.handle();
        record.borrow().life.close();
        log::error!("{error:#}");
        let handler = state.borrow().error_handler.clone();
        if let Some(handler) = handler {
            handler(handle, &error);
        }
        let mut st = state.borrow_mut();
        st.first_error.get_or_insert(error);
    }
}

fn wake_tick_if_idle(state: &Rc<RefCell<AppState>>) {
    let st = state.borrow();
    if !st.ticking {
        st.wake.schedule_tick();
    }
}

struct DelegateIvars {
    state: Weak<RefCell<AppState>>,
    setup: Cell<*mut ()>,
    invoke_setup: fn(*mut (), MacApplication) -> Result<()>,
    setup_error: RefCell<Option<anyhow::Error>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateIvars]
    struct SolorAppDelegate;

    unsafe impl NSObjectProtocol for SolorAppDelegate {}

    unsafe impl NSApplicationDelegate for SolorAppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            let Some(state) = self.ivars().state.upgrade() else {
                return;
            };
            let mtm = state.borrow().mtm;
            let app = state.borrow().app.clone();
            app.activate();

            let setup_ptr = self.ivars().setup.replace(std::ptr::null_mut());
            if !setup_ptr.is_null() {
                let mac_app = MacApplication(Rc::downgrade(&state));
                if let Err(error) = (self.ivars().invoke_setup)(setup_ptr, mac_app) {
                    *self.ivars().setup_error.borrow_mut() = Some(error);
                    let mut st = state.borrow_mut();
                    st.stopping = true;
                    drop(st);
                    stop_application(&app, mtm);
                    return;
                }
            }
            AppState::tick(&state);
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(
            &self,
            _sender: &NSApplication,
        ) -> NSApplicationTerminateReply {
            let Some(state) = self.ivars().state.upgrade() else {
                return NSApplicationTerminateReply::TerminateNow;
            };
            let records = state.borrow().windows.clone();
            for rec in records {
                let life = rec.borrow().life.clone();
                if !life.request_close() && life.is_open() {
                    return NSApplicationTerminateReply::TerminateCancel;
                }
            }
            invoke_main_tick();
            NSApplicationTerminateReply::TerminateCancel
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(
            &self,
            _sender: &NSApplication,
        ) -> bool {
            false
        }
    }
);

/// macOS AppKit platform host for Solor windows.
#[derive(Default)]
pub struct MacOs {
    /// Allocation preference used when the first GPU window opens.
    pub gpu_memory_policy: solor::platform::GpuMemoryPolicy,
    /// Whether to trace GPU adapter selection to stderr.
    pub trace_submissions: bool,
}

/// Cloneable UI-thread handle for opening and managing windows on a running macOS event loop.
#[derive(Clone)]
pub struct MacApplication(Weak<RefCell<AppState>>);

impl MacApplication {
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self(Weak::new())
    }

    /// Sets the UI-thread callback invoked when a window fails due to a runtime error.
    pub fn on_window_error(
        &self,
        handler: impl Fn(WindowHandle, &anyhow::Error) + 'static,
    ) -> Result<()> {
        let state = self.0.upgrade().context("application has exited")?;
        state.borrow_mut().error_handler = Some(Rc::new(handler));
        Ok(())
    }

    /// Delivers background messages on the UI thread, including while no windows are open.
    /// Dropping the last sender drains queued messages and releases the channel.
    pub fn channel<T: Send + 'static>(
        &self,
        capacity: usize,
        receive: impl FnMut(&MacApplication, T) + 'static,
    ) -> Result<ApplicationSender<T>> {
        ensure!(
            capacity > 0,
            "application channel requires a positive capacity"
        );
        let state = self.0.upgrade().context("application has exited")?;
        let wake = state.borrow().wake.clone();
        let (sender, inbox) = crate::channel::channel(capacity, &wake, receive);
        state.borrow_mut().inboxes.push(inbox);
        wake_tick_if_idle(&state);
        Ok(sender)
    }

    /// Mounts a window handler and opens a window on the running `NSApplication`.
    pub fn open_window(
        &self,
        options: WindowOptions<'_>,
        view: Box<dyn WindowHandler<Rgba8>>,
    ) -> Result<WindowHandle> {
        self.build_window(options, |_| Ok(view))
    }

    /// Builds a window handler with its newly created `WindowHandle`.
    pub fn build_window(
        &self,
        options: WindowOptions<'_>,
        build: impl FnOnce(WindowHandle) -> Result<Box<dyn WindowHandler<Rgba8>>>,
    ) -> Result<WindowHandle> {
        let state = self.0.upgrade().context("application has exited")?;
        AppState::open_window_record(&state, options, build)
    }

    /// Mounts a Solor view on a Metal GPU surface for a macOS window.
    #[cfg(feature = "gpu")]
    pub fn build_gpu_window<R: solor::Render>(
        &self,
        options: WindowOptions<'_>,
        ui: solor::Ui,
        root: impl FnOnce(WindowHandle) -> R,
    ) -> Result<WindowHandle> {
        let state = self.0.upgrade().context("application has exited")?;
        AppState::open_gpu_window_record(&state, options, ui, root)
    }
}

impl Platform for MacOs {
    type Target = Rgba8;

    fn run_window(
        &self,
        options: WindowOptions<'_>,
        handler: Box<dyn WindowHandler<Self::Target>>,
    ) -> Result<()> {
        self.run(|app| app.open_window(options, handler).map(drop))
    }
}

impl MacOs {
    /// Runs `setup` once `NSApplication` finishes launching, then dispatches events until all windows close.
    pub fn run<F>(&self, setup: F) -> Result<()>
    where
        F: FnOnce(MacApplication) -> Result<()>,
    {
        let mtm =
            MainThreadMarker::new().context("Solor macOS windows must run on the main thread")?;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        install_minimal_menu_bar(&app, mtm);

        let state = Rc::new(RefCell::new(AppState {
            mtm,
            app: app.clone(),
            windows: Vec::new(),
            pending: Vec::new(),
            inboxes: Vec::new(),
            #[cfg(feature = "gpu")]
            gpu: Rc::new(RefCell::new(crate::gpu::GpuContext::new(
                self.gpu_memory_policy,
            ))),
            #[cfg(feature = "gpu")]
            trace_submissions: self.trace_submissions,
            error_handler: None,
            first_error: None,
            revisit: Rc::new(Cell::new(false)),
            wake: MacWake::new(),
            redraw_timer: RedrawTimer::default(),
            stopping: false,
            ticking: false,
        }));

        let tick_state = Rc::downgrade(&state);
        let _tick_guard = TickGuard::install(
            Box::new(move || {
                if let Some(st) = tick_state.upgrade() {
                    AppState::tick(&st);
                }
            }),
            mtm,
        );

        let mut setup_slot = Some(setup);
        fn run_setup<F: FnOnce(MacApplication) -> Result<()>>(
            ptr: *mut (),
            app: MacApplication,
        ) -> Result<()> {
            #[allow(unsafe_code)]
            // SAFETY: `ptr` points to `setup_slot: Option<F>` on `MacOs::run`'s stack while
            // `app.run()` executes, and is cleared on first invocation or before `MacOs::run` exits.
            let slot = unsafe { &mut *ptr.cast::<Option<F>>() };
            if let Some(f) = slot.take() {
                f(app)
            } else {
                Ok(())
            }
        }
        let setup_ptr = (&raw mut setup_slot).cast::<()>();

        let delegate = mtm.alloc::<SolorAppDelegate>().set_ivars(DelegateIvars {
            state: Rc::downgrade(&state),
            setup: Cell::new(setup_ptr),
            invoke_setup: run_setup::<F>,
            setup_error: RefCell::new(None),
        });
        #[allow(unsafe_code)]
        // SAFETY: `NSObject::init` initializes a freshly allocated delegate.
        let delegate: Retained<SolorAppDelegate> = unsafe { msg_send![super(delegate), init] };
        let delegate_proto: &ProtocolObject<dyn NSApplicationDelegate> =
            ProtocolObject::from_ref(&*delegate);
        app.setDelegate(Some(delegate_proto));

        app.run();

        delegate.ivars().setup.set(std::ptr::null_mut());
        app.setDelegate(None);

        let remaining_windows = std::mem::take(&mut state.borrow_mut().windows);
        for rec in remaining_windows {
            WindowRecord::destroy(&rec);
        }
        let pending_windows = std::mem::take(&mut state.borrow_mut().pending);
        for rec in pending_windows {
            WindowRecord::destroy(&rec);
        }
        state.borrow_mut().inboxes.clear();

        let setup_error = delegate.ivars().setup_error.borrow_mut().take();
        if let Some(error) = setup_error {
            return Err(error);
        }
        let first_error = state.borrow_mut().first_error.take();
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }
}

fn install_minimal_menu_bar(app: &NSApplication, mtm: MainThreadMarker) {
    if app.mainMenu().is_some() {
        return;
    }
    let menu_bar = NSMenu::new(mtm);
    let app_menu_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);

    #[allow(unsafe_code)]
    // SAFETY: Standard AppKit menu selectors (`hide:` and `terminate:`) on `NSApplication`.
    unsafe {
        let hide_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Hide"),
            Some(sel!(hide:)),
            &NSString::from_str("h"),
        );
        app_menu.addItem(&hide_item);

        let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Quit"),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        );
        app_menu.addItem(&quit_item);
    }

    app_menu_item.setSubmenu(Some(&app_menu));
    menu_bar.addItem(&app_menu_item);

    let window_menu_item = NSMenuItem::new(mtm);
    let window_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Window"));
    #[allow(unsafe_code)]
    // SAFETY: Standard AppKit `NSWindow` action selectors (`performClose:`, `performMiniaturize:`, `performZoom:`).
    unsafe {
        let close_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Close Window"),
            Some(sel!(performClose:)),
            &NSString::from_str("w"),
        );
        window_menu.addItem(&close_item);

        let minimize_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Minimize"),
            Some(sel!(performMiniaturize:)),
            &NSString::from_str("m"),
        );
        window_menu.addItem(&minimize_item);

        let zoom_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Zoom"),
            Some(sel!(performZoom:)),
            &NSString::from_str(""),
        );
        window_menu.addItem(&zoom_item);
    }
    window_menu_item.setSubmenu(Some(&window_menu));
    menu_bar.addItem(&window_menu_item);
    app.setWindowsMenu(Some(&window_menu));

    app.setMainMenu(Some(&menu_bar));
}

fn stop_application(app: &NSApplication, _mtm: MainThreadMarker) {
    app.stop(None);
    // `NSApplication::stop:` only takes effect after the next event is processed.
    let wake_event = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
        NSEventType::ApplicationDefined,
        NSPoint::ZERO,
        NSEventModifierFlags::empty(),
        0.0,
        0,
        None,
        0,
        0,
        0,
    );
    if let Some(event) = wake_event {
        app.postEvent_atStart(&event, true);
    }
}
