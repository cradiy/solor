//! Live URL requests shared by the mounted image elements of a UI.
use super::decode::ENCODED_LIMIT;
use crate::{Context, ImageAsset, Signal, TaskScope, Url};
use anyhow::{Result, ensure};
use std::{
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    io::Read,
    rc::{Rc, Weak},
    sync::{self, Arc, LazyLock},
    time::Duration,
};

type Images = RefCell<HashMap<Url, Weak<Request>>>;
type ImageSignal = Signal<Option<ImageAsset>>;

#[derive(Default)]
pub(crate) struct RemoteImages {
    images: OnceCell<Rc<Images>>,
}

struct Request {
    cache: Weak<Images>,
    url: Url,
    lifetime: Arc<()>,
    scope: TaskScope,
    pixels: OnceCell<ImageAsset>,
    listeners: RefCell<HashSet<ImageSignal>>,
}

impl Request {
    fn forget(&self) {
        if let Some(cache) = self.cache.upgrade() {
            let mut cache = cache.borrow_mut();
            if cache
                .get(&self.url)
                .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
            {
                cache.remove(&self.url);
            }
        }
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        self.forget();
    }
}

pub(crate) struct Subscription {
    request: Rc<Request>,
    pub(crate) signal: ImageSignal,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.request.listeners.borrow_mut().remove(&self.signal);
    }
}

impl RemoteImages {
    pub(crate) fn subscribe(&self, mut url: Url, cx: &mut Context<'_>) -> Result<Subscription> {
        ensure!(
            matches!(url.scheme(), "http" | "https"),
            "expected an HTTP or HTTPS image URL"
        );
        url.set_fragment(None);
        let images = self.images.get_or_init(Default::default);
        let existing = images.borrow().get(&url).and_then(Weak::upgrade);
        let request = existing.unwrap_or_else(|| {
            let request = Rc::new(Request {
                cache: Rc::downgrade(images),
                url: url.clone(),
                lifetime: Arc::default(),
                scope: TaskScope::default(),
                pixels: OnceCell::new(),
                listeners: RefCell::default(),
            });
            let weak = Rc::downgrade(&request);
            images.borrow_mut().insert(url.clone(), weak.clone());
            let lifetime = Arc::downgrade(&request.lifetime);
            cx.tasks.spawn_in(&request.scope, async move |cx| {
                let result = cx
                    .blocking_when_ready(move || download(url, lifetime))
                    .await;
                if let Some(request) = weak.upgrade() {
                    match result {
                        Ok(asset) => {
                            for signal in request.listeners.borrow().iter().copied() {
                                cx.set(signal, Some(asset.clone()))?;
                            }
                            request.pixels.set(asset).expect("one download per request");
                        }
                        Err(error) => {
                            request.forget();
                            log::warn!("remote image load failed: {error}");
                        }
                    }
                }
                Ok(())
            });
            request
        });
        let signal = cx.signal(request.pixels.get().cloned());
        request.listeners.borrow_mut().insert(signal);
        Ok(Subscription { request, signal })
    }
}

fn download(url: Url, lifetime: sync::Weak<()>) -> Result<ImageAsset> {
    static HTTP: LazyLock<ureq::Agent> = LazyLock::new(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into()
    });
    ensure!(lifetime.strong_count() != 0, "image was released");
    let mut response = HTTP.get(url.as_str()).call()?;
    ensure!(
        response.status().is_success(),
        "HTTP status {}",
        response.status()
    );
    ensure!(
        response
            .body()
            .content_length()
            .is_none_or(|n| n <= ENCODED_LIMIT),
        "encoded image exceeds 64 MiB"
    );
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(ENCODED_LIMIT)
        .reader();
    let mut bytes = Vec::new();
    let mut chunk = [0; 16 * 1024];
    loop {
        ensure!(lifetime.strong_count() != 0, "image was released");
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    ensure!(lifetime.strong_count() != 0, "image was released");
    ImageAsset::from_download(&bytes)
}
