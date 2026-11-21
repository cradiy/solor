use std::io::{Error, ErrorKind};

use anyhow::Result;
use solor::{Render, Ui};

use super::{Application, WindowOptions};

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

pub(super) fn open_window(
    app: &Windows,
    _: WindowOptions<'_>,
    _: Ui,
    _: impl Render,
) -> Result<()> {
    match *app {}
}

impl Host {
    pub(super) fn run(&self, _: impl FnOnce(Application) -> Result<()>) -> Result<()> {
        match *self {}
    }
}
