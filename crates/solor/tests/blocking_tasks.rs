//! Worker results, bounded admission, and cancellation with occupied workers.
use solor::{Ui, component, div};
use std::{
    cell::Cell,
    io::ErrorKind,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Wake, Waker},
    thread,
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(5);
// Both tests exercise the same process-wide pool.
static POOL_TEST: Mutex<()> = Mutex::new(());

struct Host(mpsc::Sender<()>);

impl Wake for Host {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

#[test]
fn blocking_result_wakes_the_foreground_and_updates_its_signal() -> anyhow::Result<()> {
    let _serial = POOL_TEST.lock().unwrap();
    let mut ui = Ui::new();
    let values = ui.context().signal(Vec::<i32>::new());
    let owner = thread::current().id();
    let (release, gate) = mpsc::channel();
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));

    ui.context().spawn(async move |cx| {
        let answer = cx
            .spawn_blocking(move || {
                assert_ne!(thread::current().id(), owner);
                gate.recv_timeout(WAIT)?;
                Ok(42)
            })
            .await?;
        assert_eq!(thread::current().id(), owner);
        cx.update_in_place(values, |items| items.push(answer))
    });
    ui.poll_tasks(&waker)?;
    ui.context()
        .update_in_place(values, |items| items.push(1))?;
    release.send(())?;
    wakes.recv_timeout(WAIT)?;
    ui.poll_tasks(&waker)?;
    assert_eq!(ui.context().read(values)?, &[1, 42]);
    Ok(())
}

#[test]
fn windows_share_capacity_and_unmount_releases_waiting_work() -> anyhow::Result<()> {
    let _serial = POOL_TEST.lock().unwrap();
    let (started, workers) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let mut gates = Vec::new();
    let mut windows = Vec::new();
    for _ in 0..2 {
        let mut ui = Ui::new();
        let resumed = Rc::new(Cell::new(false));
        let seen = resumed.clone();
        let (release, gate) = mpsc::channel();
        let started = started.clone();
        let done = done.clone();
        let view = ui.mount(component(move |cx| {
            cx.spawn(async move |cx| {
                cx.spawn_blocking(move || {
                    started.send(thread::current().id())?;
                    gate.recv_timeout(WAIT)?;
                    Ok(done.send(())?)
                })
                .await?;
                seen.set(true);
                Ok(())
            });
            div()
        }))?;
        ui.poll_tasks(Waker::noop())?;
        gates.push(release);
        windows.push((ui, view, resumed));
    }
    assert_ne!(workers.recv_timeout(WAIT)?, workers.recv_timeout(WAIT)?);

    let mut ui = Ui::new();
    let captures = Arc::new(AtomicBool::new(false));
    let queued = captures.clone();
    let view = ui.mount(component(move |cx| {
        for _ in 0..64 {
            let capture = queued.clone();
            cx.spawn(async move |cx| {
                cx.spawn_blocking(move || {
                    capture.store(true, Ordering::Relaxed);
                    Ok(())
                })
                .await
            });
        }
        div()
    }))?;
    let full = Rc::new(Cell::new(false));
    let seen = full.clone();
    ui.context().spawn(async move |cx| {
        let err = cx.spawn_blocking(|| Ok(())).await.unwrap_err();
        seen.set(
            err.downcast_ref::<std::io::Error>().map(|e| e.kind()) == Some(ErrorKind::WouldBlock),
        );
        Ok(())
    });
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));
    ui.poll_tasks(&waker)?;
    while !full.get() {
        wakes.recv_timeout(WAIT)?;
        ui.poll_tasks(&waker)?;
    }

    ui.unmount(view)?;
    while Arc::strong_count(&captures) > 1 {
        wakes.recv_timeout(WAIT)?;
        ui.poll_tasks(&waker)?;
    }

    let completed = ui.context().signal(false);
    ui.context().spawn(async move |cx| {
        cx.spawn_blocking(|| Ok(())).await?;
        cx.set(completed, true)
    });
    ui.poll_tasks(&waker)?;
    for (window, view, _) in &mut windows {
        window.unmount(*view)?;
        window.poll_tasks(Waker::noop())?;
    }
    for release in gates {
        release.send(())?;
    }
    while !*ui.context().read(completed)? {
        wakes.recv_timeout(WAIT)?;
        ui.poll_tasks(&waker)?;
    }
    assert!(!captures.load(Ordering::Relaxed));
    for (mut window, _, resumed) in windows {
        finished.recv_timeout(WAIT)?;
        window.poll_tasks(Waker::noop())?;
        assert!(!resumed.get());
    }
    Ok(())
}
