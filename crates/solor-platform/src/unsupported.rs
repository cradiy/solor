use std::io::{Error, ErrorKind};

use anyhow::Result;
use solor::{Render, Ui};

use super::{Application, WindowOptions};

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
    app: &mut Windows,
    _: WindowOptions<'_>,
    _: Ui,
    _: impl Render,
) -> Result<()> {
    match *app {}
}

impl Host {
    pub(super) fn run(&self, _: impl FnOnce(&mut Application<'_>) -> Result<()>) -> Result<()> {
        match *self {}
    }
}
