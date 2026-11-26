use std::io::{Error, ErrorKind};

use anyhow::Result;
use solor::{Render, Ui};

use super::{WindowHandle, WindowOptions};

#[derive(Clone)]
pub(super) enum Host {}

pub(super) fn select() -> Result<Host> {
    Err(Error::new(
        ErrorKind::Unsupported,
        format!(
            "no Solor window implementation for {}",
            std::env::consts::OS
        ),
    )
    .into())
}

pub(super) type Windows = Host;

pub(super) fn open_window<R: Render>(
    _: super::Renderer,
    app: &Windows,
    _: WindowOptions<'_>,
    _: Ui,
    _: impl FnOnce(WindowHandle) -> R,
) -> Result<WindowHandle> {
    match *app {}
}

impl Host {
    pub(super) fn set_gpu_memory_policy(&mut self, _: super::GpuMemoryPolicy) {
        match *self {}
    }

    pub(super) fn run(&self, _: impl FnOnce(Windows) -> Result<()>) -> Result<()> {
        match *self {}
    }
}
