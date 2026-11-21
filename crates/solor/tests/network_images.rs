//! URL image ownership, network completion and intrinsic layout.
#![cfg(all(feature = "network", feature = "png"))]

use anyhow::Result;
use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
use solor::{
    Draw, Fill, ImageAsset, Styled, Ui, Url, ViewId, component, div, image, layout::Size, px,
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, mpsc},
    task::{Wake, Waker},
    thread,
    time::Duration,
};

// Each scenario controls when the shared workers can receive a response.
static WORKERS: Mutex<()> = Mutex::new(());
const WAIT: Duration = Duration::from_secs(5);

struct Host(mpsc::Sender<()>);
impl Wake for Host {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

fn mount(ui: &mut Ui, url: &Url) -> Result<ViewId> {
    let url = url.clone();
    ui.mount(component(move |_| div().items_start().child(image(url))))
}

fn frame(ui: &mut Ui, view: ViewId) -> Result<(Size, Option<ImageAsset>)> {
    ui.flush()?;
    ui.layout(
        view,
        Size {
            width: px(100.),
            height: px(100.),
        },
    )?;
    let child = ui.children(view, ui.root(view)?)?.next().unwrap();
    let size = ui.geometry(view, child).unwrap().metrics.size;
    let asset = ui
        .record_frame(view)?
        .draws()
        .iter()
        .find_map(|draw| match draw {
            Draw::Fill {
                fill: Fill::Image(image),
                ..
            } => Some(image.asset().clone()),
            _ => None,
        });
    Ok((size, asset))
}

fn read_request(stream: &TcpStream) -> Result<String> {
    stream.set_read_timeout(Some(WAIT))?;
    let mut head = String::new();
    let mut reader = BufReader::new(stream);
    while reader.read_line(&mut head)? != 0 && !head.ends_with("\r\n\r\n") {}
    Ok(head)
}

fn reply(mut stream: TcpStream, color: [u8; 4]) -> Result<()> {
    let request = read_request(&stream)?;
    assert!(request.starts_with("GET /icon.png"));
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(&color.repeat(2), 2, 1, ExtendedColorType::Rgba8)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        png.len()
    )?;
    stream.write_all(&png)?;
    Ok(())
}

#[test]
fn mounted_consumers_share_the_request_and_release_it_after_the_last_unmount() -> Result<()> {
    let _workers = WORKERS.lock().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = Url::parse(&format!("http://{}/icon.png", listener.local_addr()?))?;
    let (connections, incoming) = mpsc::channel();
    let server = thread::spawn(move || -> Result<()> {
        for connection in listener.incoming().take(2) {
            connections.send(connection?)?;
        }
        Ok(())
    });

    let mut ui = Ui::new();
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));
    let first = mount(&mut ui, &url)?;
    let mut alias = url.clone();
    alias.set_fragment(Some("preview"));
    let second = mount(&mut ui, &alias)?;
    assert_eq!(frame(&mut ui, second)?, (Size::default(), None));
    ui.poll_tasks(&waker)?;
    let stream = incoming.recv_timeout(WAIT)?;
    ui.unmount(first)?;
    reply(stream, [10, 20, 30, 255])?;
    wakes.recv_timeout(WAIT)?;
    ui.poll_tasks(&waker)?;
    let (size, pixels) = frame(&mut ui, second)?;
    assert_eq!(
        size,
        Size {
            width: px(2.),
            height: px(1.)
        }
    );
    let pixels = pixels.unwrap();
    assert_eq!(pixels.pixels(), &[[10, 20, 30, 255]; 2]);
    let third = mount(&mut ui, &url)?;
    assert_eq!(frame(&mut ui, third)?.1, Some(pixels));
    ui.unmount(second)?;
    ui.unmount(third)?;

    let fresh = mount(&mut ui, &url)?;
    ui.poll_tasks(&waker)?;
    let stream = incoming.recv_timeout(WAIT)?;
    wakes.try_iter().for_each(drop);
    reply(stream, [60, 70, 80, 255])?;
    wakes.recv_timeout(WAIT)?;
    ui.poll_tasks(&waker)?;
    assert_eq!(
        frame(&mut ui, fresh)?.1.unwrap().pixels(),
        &[[60, 70, 80, 255]; 2]
    );
    server.join().unwrap()?;
    Ok(())
}

#[test]
fn failed_response_releases_the_cache_key_for_a_new_attachment() -> Result<()> {
    let _workers = WORKERS.lock().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = Url::parse(&format!("http://{}/icon.png", listener.local_addr()?))?;
    let (close, closed) = mpsc::channel();
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        read_request(&stream)?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n"
        )?;
        // Keep the first body open until the client has rejected its headers.
        closed.recv_timeout(WAIT)?;
        drop(stream);
        reply(listener.accept()?.0, [21, 42, 84, 255])
    });
    let mut ui = Ui::new();
    let first = mount(&mut ui, &url)?;
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));
    ui.poll_tasks(&waker)?;
    wakes.recv_timeout(WAIT)?;
    ui.poll_tasks(&waker)?;
    assert_eq!(frame(&mut ui, first)?.1, None);
    close.send(())?;
    let second = mount(&mut ui, &url)?;
    ui.unmount(first)?;
    ui.poll_tasks(&waker)?;
    while frame(&mut ui, second)?.1.is_none() {
        wakes.recv_timeout(WAIT)?;
        ui.poll_tasks(&waker)?;
    }
    server.join().unwrap()?;
    Ok(())
}

#[test]
fn a_burst_of_distinct_images_resumes_as_worker_slots_open() -> Result<()> {
    let _workers = WORKERS.lock().unwrap();
    const COUNT: usize = 72;
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let base = Url::parse(&format!("http://{}/icon.png", listener.local_addr()?))?;
    let (release, gate) = mpsc::channel();
    let server = thread::spawn(move || -> Result<()> {
        gate.recv_timeout(WAIT)?;
        for stream in listener.incoming().take(COUNT) {
            reply(stream?, [1, 2, 3, 255])?;
        }
        Ok(())
    });
    let (notify, wakes) = mpsc::channel();
    let waker = Waker::from(Arc::new(Host(notify)));
    let mut ui = Ui::new();
    let mut views = Vec::new();
    for i in 0..COUNT {
        let mut url = base.clone();
        url.set_query(Some(&i.to_string()));
        views.push(mount(&mut ui, &url)?);
        ui.poll_tasks(&waker)?;
    }
    release.send(())?;
    while !views.is_empty() {
        wakes.recv_timeout(WAIT)?;
        ui.poll_tasks(&waker)?;
        let mut pending = Vec::new();
        for view in views {
            if frame(&mut ui, view)?.1.is_none() {
                pending.push(view);
            }
        }
        views = pending;
    }
    server.join().unwrap()?;
    Ok(())
}

#[test]
fn releasing_the_last_consumer_abandons_an_unfinished_response() -> Result<()> {
    let _workers = WORKERS.lock().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = Url::parse(&format!("http://{}/icon.png", listener.local_addr()?))?;
    let (headers, started) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let server = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        read_request(&stream)?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 65536\r\nConnection: close\r\n\r\na"
        )?;
        headers.send(())?;
        gate.recv_timeout(WAIT)?;
        // Let any read that was already blocked return to the cancellation check.
        let _ = stream.write_all(b"b");
        match stream.read(&mut [0; 1]) {
            Ok(0) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => Ok(()),
            other => anyhow::bail!("expected the abandoned HTTP connection to close: {other:?}"),
        }
    });
    let mut ui = Ui::new();
    let view = mount(&mut ui, &url)?;
    ui.poll_tasks(Waker::noop())?;
    started.recv_timeout(WAIT)?;
    ui.unmount(view)?;
    ui.poll_tasks(Waker::noop())?;
    release.send(())?;
    server.join().unwrap()?;
    Ok(())
}
