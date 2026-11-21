//! Cooperative foreground work owned by the component that starts it.

mod blocking;
use crate::{
    Context, Ui,
    state::{Runtime, Signal},
};
use anyhow::{Context as _, Result};
use async_task::{Runnable, Task};
use slotmap::{DefaultKey, SlotMap};
use std::{
    cell::{OnceCell, RefCell},
    future::Future,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Waker,
};

// Only the foreground pump shares the state allocation. Synchronous contexts
// keep their exclusive borrow, including references returned by Context::read.
type SharedState = Rc<RefCell<Runtime<()>>>;

pub(super) struct UiState(SharedState);

impl Default for UiState {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(Runtime::new(()))))
    }
}

impl UiState {
    pub(super) fn get_mut(&mut self) -> &mut Runtime<()> {
        Rc::get_mut(&mut self.0)
            .expect("exclusive between task polls")
            .get_mut()
    }
}

impl std::ops::Deref for UiState {
    type Target = RefCell<Runtime<()>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

type ActiveState = RefCell<Option<SharedState>>;

struct PollGuard<'a>(&'a ActiveState);

impl Drop for PollGuard<'_> {
    fn drop(&mut self) {
        self.0.take();
    }
}

/// Signal access between await points of a foreground task.
pub struct AsyncContext {
    state: Weak<ActiveState>,
}

impl AsyncContext {
    /// Publishes a value to a live signal and queues its subscribers on change.
    pub fn set<T: PartialEq + 'static>(&self, signal: Signal<T>, value: T) -> Result<()> {
        let state = self.state.upgrade().context("UI has closed")?;
        let active = state.try_borrow()?;
        active
            .as_ref()
            .context("outside foreground task dispatch")?
            .try_borrow_mut()?
            .set(signal, value)?;
        Ok(())
    }
}

type Handles = Rc<RefCell<SlotMap<DefaultKey, Task<()>>>>;

#[derive(Default)]
pub(super) struct TaskScope(OnceCell<Handles>);

struct Ready {
    sender: mpsc::Sender<Runnable>,
    pending: AtomicBool,
    host: Mutex<Option<Waker>>,
}

impl Ready {
    fn notify(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let waker = self.host.lock().expect("task waker lock").clone();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

type LiveTasks = RefCell<SlotMap<DefaultKey, Waker>>;

// The future owns this entry from creation through completion or cancellation,
// including cancellation before its first poll.
struct Registration {
    live: Weak<LiveTasks>,
    id: DefaultKey,
    owner: Weak<RefCell<SlotMap<DefaultKey, Task<()>>>>,
    slot: DefaultKey,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(live) = self.live.upgrade() {
            live.borrow_mut().remove(self.id);
        }
        if let Some(owner) = self.owner.upgrade()
            && let Some(handle) = owner.borrow_mut().remove(self.slot)
        {
            handle.detach();
        }
    }
}

struct Driver {
    state: Rc<ActiveState>,
    live: Rc<LiveTasks>,
    ready: Arc<Ready>,
    receiver: mpsc::Receiver<Runnable>,
    error: Rc<RefCell<Option<anyhow::Error>>>,
}

impl Driver {
    fn new(host: Option<Waker>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            state: Default::default(),
            live: Default::default(),
            ready: Arc::new(Ready {
                sender,
                pending: AtomicBool::new(false),
                host: Mutex::new(host),
            }),
            receiver,
            error: Default::default(),
        }
    }

    fn spawn<F: Future<Output = Result<()>> + 'static>(
        &self,
        handles: &Handles,
        start: impl FnOnce(AsyncContext) -> F + 'static,
    ) {
        let mut live = self.live.borrow_mut();
        let id = live.insert(Waker::noop().clone());
        handles.borrow_mut().insert_with_key(|slot| {
            let registration = Registration {
                live: Rc::downgrade(&self.live),
                id,
                owner: Rc::downgrade(handles),
                slot,
            };
            let errors = self.error.clone();
            let cx = AsyncContext {
                state: Rc::downgrade(&self.state),
            };
            let ready = self.ready.clone();
            let (runnable, handle) = async_task::spawn_local(
                async move {
                    let _registration = registration;
                    if let Err(error) = start(cx).await {
                        errors.borrow_mut().get_or_insert(error);
                    }
                },
                move |runnable| {
                    ready
                        .sender
                        .send(runnable)
                        .expect("live foreground receiver");
                    ready.notify();
                },
            );
            live[id] = runnable.waker();
            runnable.schedule();
            handle
        });
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.ready.host.lock().expect("task waker lock").take();
        // A remote wake may already own the scheduled runnable and be about
        // to send it. Keep receiving until every local future has been dropped
        // on this thread. Dropping a runnable cancels it without polling.
        for waker in self.live.borrow().values() {
            waker.wake_by_ref();
        }
        while !self.live.borrow().is_empty() {
            drop(self.receiver.recv().expect("live foreground sender"));
        }
    }
}

#[derive(Default)]
pub(super) struct Tasks {
    driver: OnceCell<Box<Driver>>,
    app: TaskScope,
    host: Option<Waker>,
}

impl Context<'_> {
    /// Starts a future on the UI thread. A component or event task is cancelled
    /// when its element unmounts. Application tasks live until the UI closes.
    /// Awaiting releases the UI thread. Each poll must finish promptly.
    pub fn spawn<F: Future<Output = Result<()>> + 'static>(
        &mut self,
        start: impl FnOnce(AsyncContext) -> F + 'static,
    ) {
        let scope = self
            .owned
            .as_deref()
            .map_or(&self.tasks.app, |owner| &owner.tasks);
        self.tasks.spawn_in(scope, start);
    }
}

impl Tasks {
    pub(super) fn spawn_in<F: Future<Output = Result<()>> + 'static>(
        &self,
        scope: &TaskScope,
        start: impl FnOnce(AsyncContext) -> F + 'static,
    ) {
        let driver = self
            .driver
            .get_or_init(|| Box::new(Driver::new(self.host.clone())));
        let handles = scope.0.get_or_init(Default::default);
        driver.spawn(handles, start);
    }
}

impl Ui {
    /// Polls ready foreground tasks for one event-loop turn. The waker must
    /// interrupt the host's wait when work arrives. Task errors reach the host here.
    pub fn poll_tasks(&mut self, waker: &Waker) -> Result<()> {
        if self
            .tasks
            .host
            .as_ref()
            .is_none_or(|host| !host.will_wake(waker))
        {
            self.tasks.host = Some(waker.clone());
            if let Some(driver) = self.tasks.driver.get() {
                *driver.ready.host.lock().expect("task waker lock") = Some(waker.clone());
            }
        }
        let Some(driver) = self.tasks.driver.get() else {
            return Ok(());
        };
        if !driver.ready.pending.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        driver.state.replace(Some(self.state.0.clone()));
        let _guard = PollGuard(&driver.state);
        // A task that wakes itself goes to the next host turn, behind ready input.
        const POLLS_PER_TURN: usize = 32;
        let batch: smallvec::SmallVec<[Runnable; POLLS_PER_TURN]> =
            driver.receiver.try_iter().take(POLLS_PER_TURN).collect();
        if batch.len() == POLLS_PER_TURN {
            driver.ready.notify();
        }
        for runnable in batch {
            runnable.run();
        }
        driver.error.borrow_mut().take().map_or(Ok(()), Err)
    }
}
