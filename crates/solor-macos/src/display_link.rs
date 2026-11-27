//! Display-synchronized frame pacing via `CADisplayLink` with `NSTimer` fallback.
use block2::RcBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained, sel,
};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes, NSTimer};
use objc2_quartz_core::CADisplayLink;
use std::{cell::Cell, ptr::NonNull, rc::Rc};

struct TargetIvars {
    on_frame: Rc<dyn Fn()>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TargetIvars]
    struct DisplayLinkTarget;

    unsafe impl NSObjectProtocol for DisplayLinkTarget {}

    impl DisplayLinkTarget {
        #[unsafe(method(onFrame:))]
        fn on_frame(&self, _sender: &CADisplayLink) {
            (self.ivars().on_frame)();
        }
    }
);

impl DisplayLinkTarget {
    fn new(on_frame: Rc<dyn Fn()>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(TargetIvars { on_frame });
        #[allow(unsafe_code)]
        // SAFETY: `NSObject::init` initializes a fresh `DisplayLinkTarget` instance.
        unsafe {
            msg_send![super(this), init]
        }
    }
}

enum LinkKind {
    CoreAnimation {
        link: Retained<CADisplayLink>,
        _target: Retained<DisplayLinkTarget>,
    },
    TimerFallback {
        on_frame: Rc<dyn Fn()>,
        timer: Option<Retained<NSTimer>>,
    },
}

/// Per-window display-refresh callback source, paused when no animation frames are requested.
pub(crate) struct DisplayLink {
    kind: LinkKind,
    active: Cell<bool>,
}

impl DisplayLink {
    pub(crate) fn for_view(view: &NSView, on_frame: Rc<dyn Fn()>, mtm: MainThreadMarker) -> Self {
        #[allow(unsafe_code)]
        // SAFETY: Checking `respondsToSelector:` on main thread for macOS 14+ `NSView.displayLinkWithTarget:selector:`.
        let supports_ca_link: bool =
            unsafe { msg_send![view, respondsToSelector: sel!(displayLinkWithTarget:selector:)] };

        let kind = if supports_ca_link {
            let target = DisplayLinkTarget::new(on_frame, mtm);
            #[allow(unsafe_code)]
            // SAFETY: `target` implements `onFrame:` and is retained alongside `link`; `link` is
            // invalidated in `Drop` before `_target` is released.
            let link = unsafe {
                let link = view.displayLinkWithTarget_selector(&target, sel!(onFrame:));
                link.setPaused(true);
                link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes);
                link
            };
            LinkKind::CoreAnimation {
                link,
                _target: target,
            }
        } else {
            LinkKind::TimerFallback {
                on_frame,
                timer: None,
            }
        };

        Self {
            kind,
            active: Cell::new(false),
        }
    }

    pub(crate) fn set_active(&mut self, active: bool) {
        if self.active.get() == active {
            return;
        }
        self.active.set(active);
        match &mut self.kind {
            LinkKind::CoreAnimation { link, .. } => {
                link.setPaused(!active);
            }
            LinkKind::TimerFallback { on_frame, timer } => {
                if let Some(old) = timer.take() {
                    old.invalidate();
                }
                if active {
                    let cb = on_frame.clone();
                    let block = RcBlock::new(move |_t: NonNull<NSTimer>| {
                        cb();
                    });
                    #[allow(unsafe_code)]
                    // SAFETY: Timer runs on main run loop where `Rc<dyn Fn()>` lives.
                    let t = unsafe {
                        NSTimer::timerWithTimeInterval_repeats_block(1.0 / 60.0, true, &block)
                    };
                    #[allow(unsafe_code)]
                    // SAFETY: Scheduled on `NSRunLoop::mainRunLoop()` on the main thread.
                    unsafe {
                        NSRunLoop::mainRunLoop().addTimer_forMode(&t, NSRunLoopCommonModes);
                    }
                    *timer = Some(t);
                }
            }
        }
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        match &mut self.kind {
            LinkKind::CoreAnimation { link, .. } => {
                link.invalidate();
            }
            LinkKind::TimerFallback { timer, .. } => {
                if let Some(old) = timer.take() {
                    old.invalidate();
                }
            }
        }
    }
}
