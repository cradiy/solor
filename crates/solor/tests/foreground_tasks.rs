//! Foreground task wake, cancellation, and dispatch fairness.
use solor::{Ui, component, div};
use std::{
    cell::Cell,
    future::poll_fn,
    rc::Rc,
    sync::{Arc, mpsc},
    task::{Poll, Wake, Waker},
    thread,
    time::Duration,
};

struct Host(mpsc::Sender<()>);

impl Wake for Host {
    fn wake(self: Arc<Self>) {
        self.0.send(()).unwrap();
    }
}

#[test]
fn foreign_wake_resumes_local_state_on_the_owner_thread() -> anyhow::Result<()> {
    let mut ui = Ui::new();
    let value = ui.context().signal(0);
    let owner = thread::current().id();
    let local = Rc::new(41);
    let (send, receive) = mpsc::channel::<Waker>();
    let mut first = true;
    ui.context().spawn(async move |cx| {
        poll_fn(|poll| {
            if std::mem::take(&mut first) {
                send.send(poll.waker().clone()).unwrap();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        assert_eq!(thread::current().id(), owner);
        cx.set(value, *local + 1)
    });
    let (host, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(host)));
    ui.poll_tasks(&waker)?;
    assert_eq!(*ui.context().read(value)?, 0);
    wakes.try_iter().for_each(drop);
    thread::spawn(move || receive.recv().unwrap().wake())
        .join()
        .unwrap();
    wakes.recv_timeout(Duration::from_secs(2))?;
    ui.poll_tasks(&waker)?;
    assert_eq!(*ui.context().read(value)?, 42);
    Ok(())
}

#[test]
fn unmount_cancels_even_a_task_already_in_the_ready_queue() -> anyhow::Result<()> {
    let mut ui = Ui::new();
    let (send, receive) = mpsc::channel::<Waker>();
    let held = Rc::new(());
    let weak = Rc::downgrade(&held);
    let polls = Rc::new(Cell::new(0));
    let seen = polls.clone();
    let view = ui.mount(component(move |cx| {
        cx.spawn(async move |_| {
            poll_fn(|poll| {
                seen.set(seen.get() + 1);
                send.send(poll.waker().clone()).unwrap();
                Poll::<()>::Pending
            })
            .await;
            drop(held);
            Ok(())
        });
        div()
    }))?;
    ui.poll_tasks(Waker::noop())?;
    let late = receive.recv()?;
    late.wake_by_ref();
    ui.unmount(view)?;
    ui.poll_tasks(Waker::noop())?;
    assert_eq!(polls.get(), 1);
    assert!(weak.upgrade().is_none());
    drop(ui);
    thread::spawn(move || late.wake()).join().unwrap();
    Ok(())
}

#[test]
fn a_self_waking_task_yields_to_the_host() -> anyhow::Result<()> {
    let mut ui = Ui::new();
    let polls = Rc::new(Cell::new(0));
    let done = Rc::new(Cell::new(false));
    let (seen, stop) = (polls.clone(), done.clone());
    ui.context().spawn(async move |_| {
        poll_fn(|cx| {
            seen.set(seen.get() + 1);
            if stop.get() {
                Poll::Ready(())
            } else {
                assert!(seen.get() < 1000, "foreground dispatch did not yield");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
        Ok(())
    });
    let (host, wakes) = mpsc::channel();
    let host = Waker::from(Arc::new(Host(host)));
    ui.poll_tasks(&host)?;
    let first = polls.get();
    assert_eq!(first, 1);
    wakes.recv_timeout(Duration::from_secs(2))?;
    done.set(true);
    ui.poll_tasks(&host)?;
    assert_eq!(polls.get(), first + 1);
    Ok(())
}

#[test]
fn closing_the_ui_drops_local_futures_while_another_thread_can_wake_them() -> anyhow::Result<()> {
    let mut ui = Ui::new();
    let (send, receive) = mpsc::channel();
    let held = Rc::new(());
    let weak = Rc::downgrade(&held);
    ui.mount(component(move |cx| {
        cx.spawn(async move |_| {
            poll_fn(|cx| {
                send.send(cx.waker().clone()).unwrap();
                Poll::<()>::Pending
            })
            .await;
            drop(held);
            Ok(())
        });
        div()
    }))?;
    ui.poll_tasks(Waker::noop())?;
    let wake = receive.recv()?;
    let racing = wake.clone();
    thread::scope(|scope| {
        scope.spawn(move || {
            for _ in 0..128 {
                racing.wake_by_ref();
            }
        });
        drop(ui);
    });
    assert!(weak.upgrade().is_none());
    thread::spawn(move || wake.wake()).join().unwrap();
    Ok(())
}

#[test]
fn spawning_after_idle_wakes_the_host_until_the_backlog_is_drained() -> anyhow::Result<()> {
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));
    let mut ui = Ui::new();
    ui.poll_tasks(&waker)?;
    let finished = Rc::new(Cell::new(0));
    for _ in 0..97 {
        let finished = finished.clone();
        ui.context().spawn(async move |_| {
            finished.set(finished.get() + 1);
            Ok(())
        });
    }

    wakes.recv_timeout(Duration::from_secs(2))?;
    ui.poll_tasks(&waker)?;
    assert!((1..97).contains(&finished.get()));
    while finished.get() < 97 {
        wakes.recv_timeout(Duration::from_secs(2))?;
        ui.poll_tasks(&waker)?;
    }
    Ok(())
}
