use std::io::{Error, ErrorKind};

use anyhow::Result;
use solor::{Render, Ui};

use super::WindowOptions;

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

impl Host {
    pub(super) fn run_window(&self, _: WindowOptions<'_>, _: Ui, _: impl Render) -> Result<()> {
        match *self {}
    }
}
