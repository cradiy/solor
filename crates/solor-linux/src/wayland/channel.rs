//! Bounded application messages delivered on the Wayland thread.
use super::{WaylandApplication, wake::EventWake};
use std::{
    sync::{Arc, mpsc},
    task::Wake,
};

/// Sends messages to a UI-thread callback. Live senders keep a windowless application running.
pub struct ApplicationSender<T>(Arc<Sender<T>>);

struct Sender<T> {
    tx: mpsc::SyncSender<T>,
    // The receiver must see disconnection before the final wake.
    on_disconnect: WakeOnDrop,
}

struct WakeOnDrop(Arc<EventWake>);

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        self.0.wake_by_ref();
    }
}

impl<T> Clone for ApplicationSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> ApplicationSender<T> {
    /// Queues a message without blocking and wakes the UI thread on success.
    pub fn try_send(&self, message: T) -> Result<(), mpsc::TrySendError<T>> {
        self.0.tx.try_send(message)?;
        self.0.on_disconnect.0.wake_by_ref();
        Ok(())
    }
}

pub(super) type Inbox = Box<dyn FnMut(&WaylandApplication) -> Poll>;

pub(super) enum Poll {
    Idle,
    More,
    Closed,
}

pub(super) fn channel<T: Send + 'static>(
    capacity: usize,
    wake: &Arc<EventWake>,
    mut receive: impl FnMut(&WaylandApplication, T) + 'static,
) -> (ApplicationSender<T>, Inbox) {
    let (tx, rx) = mpsc::sync_channel(capacity);
    let sender = ApplicationSender(Arc::new(Sender {
        tx,
        on_disconnect: WakeOnDrop(wake.clone()),
    }));
    let inbox = Box::new(move |app: &WaylandApplication| {
        for _ in 0..64 {
            match rx.try_recv() {
                Ok(message) => receive(app, message),
                Err(mpsc::TryRecvError::Empty) => return Poll::Idle,
                Err(mpsc::TryRecvError::Disconnected) => return Poll::Closed,
            }
        }
        Poll::More
    });
    (sender, inbox)
}
