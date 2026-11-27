//! Main-thread wakeup coalescing and deadline timer scheduling on `NSRunLoop`.
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes, NSTimer};
use std::{
    cell::RefCell,
    ffi::c_void,
    ptr::NonNull,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::Wake,
    time::Instant,
};

thread_local! {
    static ACTIVE_TICK: RefCell<Option<Box<dyn Fn()>>> = RefCell::new(None);
}

pub(crate) struct TickGuard;

impl TickGuard {
    pub(crate) fn install(tick: Box<dyn Fn()>, _mtm: MainThreadMarker) -> Self {
        ACTIVE_TICK.with(|slot| {
            *slot.borrow_mut() = Some(tick);
        });
        Self
    }
}

impl Drop for TickGuard {
    fn drop(&mut self) {
        ACTIVE_TICK.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

pub(crate) fn invoke_main_tick() {
    let callback = ACTIVE_TICK.with(|slot| {
        let borrow = slot.borrow();
        borrow.as_ref().map(|cb| &**cb as *const dyn Fn())
    });
    if let Some(cb) = callback {
        #[allow(unsafe_code)]
        // SAFETY: `ACTIVE_TICK` is only mutated on the main thread when installing or dropping
        // `TickGuard`; `cb` cannot be dropped re-entrantly while `invoke_main_tick` runs.
        unsafe {
            (*cb)();
        }
    }
}

/// Coalesces cross-thread task/window wakeups onto the AppKit main dispatch queue.
#[derive(Default)]
pub(crate) struct MacWake {
    pending: AtomicBool,
}

impl MacWake {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn clear_pending(&self) {
        self.pending.store(false, Ordering::Release);
    }

    pub(crate) fn schedule_tick(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            #[allow(unsafe_code)]
            // SAFETY: `main_tick_trampoline` runs on the main queue and clears `pending` via
            // `ACTIVE_TICK` if still active.
            unsafe {
                DispatchQueue::main().exec_async_f(std::ptr::null_mut(), main_tick_trampoline);
            }
        }
    }
}

extern "C" fn main_tick_trampoline(_: *mut c_void) {
    invoke_main_tick();
}

impl Wake for MacWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.schedule_tick();
    }
}

/// Coalesced one-shot timer on the main run loop for `next_redraw()` deadlines.
#[derive(Default)]
pub(crate) struct RedrawTimer {
    scheduled: Option<(Instant, Retained<NSTimer>)>,
}

impl RedrawTimer {
    pub(crate) fn update(&mut self, deadline: Option<Instant>, _mtm: MainThreadMarker) {
        if let Some((current, _)) = &self.scheduled
            && deadline == Some(*current)
        {
            return;
        }
        if let Some((_, timer)) = self.scheduled.take() {
            timer.invalidate();
        }
        let Some(target) = deadline else {
            return;
        };
        let delay = target
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .max(0.0001);
        let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
            invoke_main_tick();
        });
        #[allow(unsafe_code)]
        // SAFETY: `block` only invokes the main-thread `invoke_main_tick`, and the timer is
        // scheduled on `NSRunLoop::mainRunLoop()`.
        let timer = unsafe { NSTimer::timerWithTimeInterval_repeats_block(delay, false, &block) };
        #[allow(unsafe_code)]
        // SAFETY: `NSRunLoopCommonModes` is a static Foundation constant; called on main thread.
        unsafe {
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        self.scheduled = Some((target, timer));
    }
}

impl Drop for RedrawTimer {
    fn drop(&mut self) {
        if let Some((_, timer)) = self.scheduled.take() {
            timer.invalidate();
        }
    }
}
