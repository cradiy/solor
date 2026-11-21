//! Process-wide workers for synchronous application work.

use super::AsyncContext;
use anyhow::Result;
use std::{
    collections::VecDeque,
    future::poll_fn,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, Weak},
    task::{Poll, Waker},
    thread,
    time::Duration,
};

#[cfg(feature = "network")]
static CAPACITY: event_listener::Event = event_listener::Event::new();

const MAX_WORKERS: usize = 2;
const MAX_QUEUED: usize = 64;
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

trait Job: Send + Sync {
    fn run(self: Arc<Self>);
}

struct Work<F, T> {
    function: Option<F>,
    result: Option<Result<T>>,
    waiter: Option<Waker>,
}

impl<F, T> Job for Mutex<Work<F, T>>
where
    F: FnOnce() -> Result<T> + Send,
    T: Send,
{
    fn run(self: Arc<Self>) {
        let receiver = Arc::downgrade(&self);
        let function = self
            .lock()
            .expect("blocking task lock")
            .function
            .take()
            .expect("queued once");
        drop(self);

        let result = catch_unwind(AssertUnwindSafe(function))
            .map_err(|_| anyhow::anyhow!("blocking task panicked"))
            .and_then(|value| value);
        if let Some(receiver) = receiver.upgrade() {
            let wake = {
                let mut work = receiver.lock().expect("blocking task lock");
                work.result = Some(result);
                work.waiter.take()
            };
            if let Some(waker) = wake {
                waker.wake();
            }
        }
    }
}

struct Pool {
    state: Mutex<State>,
    available: Condvar,
}

struct State {
    queue: VecDeque<Weak<dyn Job>>,
    workers: usize,
    busy: usize,
}

static POOL: Pool = Pool {
    state: Mutex::new(State {
        queue: VecDeque::new(),
        workers: 0,
        busy: 0,
    }),
    available: Condvar::new(),
};

impl Pool {
    fn try_submit(&'static self, job: Weak<dyn Job>) -> io::Result<bool> {
        let mut state = self.state.lock().expect("blocking queue lock");
        if state.queue.len() == MAX_QUEUED {
            state.queue.retain(|job| job.strong_count() != 0);
            if state.queue.len() == MAX_QUEUED {
                return Ok(false);
            }
        }
        if state.queue.len() >= state.workers - state.busy && state.workers < MAX_WORKERS {
            thread::Builder::new()
                .name("solor-worker".into())
                .spawn(|| self.worker())?;
            state.workers += 1;
        }
        state.queue.push_back(job);
        self.available.notify_one();
        Ok(true)
    }

    fn worker(&self) {
        let mut state = self.state.lock().expect("blocking queue lock");
        loop {
            if let Some(next) = state.queue.pop_front() {
                state.busy += 1;
                drop(state);
                #[cfg(feature = "network")]
                {
                    use event_listener::IntoNotification;
                    CAPACITY.notify(1.additional());
                }
                if let Some(job) = next.upgrade() {
                    job.run();
                }
                state = self.state.lock().expect("blocking queue lock");
                state.busy -= 1;
            } else {
                let (next, timeout) = self
                    .available
                    .wait_timeout_while(state, IDLE_TIMEOUT, |s| s.queue.is_empty())
                    .expect("blocking queue lock");
                state = next;
                if timeout.timed_out() && state.queue.is_empty() {
                    state.workers -= 1;
                    return;
                }
            }
        }
    }
}

impl AsyncContext {
    /// Runs synchronous work on a shared worker and resumes with its result.
    /// The first poll submits the closure. A full queue returns an I/O error
    /// of kind `WouldBlock`. Dropping the future releases unclaimed work.
    /// A closure already claimed by a worker can finish after its caller exits.
    pub async fn spawn_blocking<T: Send + 'static>(
        &self,
        function: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let work = Work::new(function);
        if !POOL.try_submit(Arc::downgrade(&work) as Weak<dyn Job>)? {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "blocking queue is full").into());
        }
        Work::receive(work).await
    }

    #[cfg(feature = "network")]
    pub(crate) async fn blocking_when_ready<T: Send + 'static>(
        &self,
        function: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let work = Work::new(function);
        let job = Arc::downgrade(&work) as Weak<dyn Job>;
        if !POOL.try_submit(job.clone())? {
            loop {
                let slot = CAPACITY.listen();
                if POOL.try_submit(job.clone())? {
                    break;
                }
                slot.await;
            }
        }
        Work::receive(work).await
    }
}

impl<F, T> Work<F, T> {
    fn new(function: F) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            function: Some(function),
            result: None,
            waiter: None,
        }))
    }

    async fn receive(work: Arc<Mutex<Self>>) -> Result<T> {
        poll_fn(|cx| {
            let mut work = work.lock().expect("blocking task lock");
            match work.result.take() {
                Some(result) => Poll::Ready(result),
                None => {
                    work.waiter = Some(cx.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }
}
