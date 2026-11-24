use std::{
    fs::File,
    io::{self, Read, Write},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    sync::Arc,
};

use anyhow::{Result, ensure};
use futures_channel::oneshot;
use rustix::{
    event::{PollFd, PollFlags},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};

const MAX_PENDING: usize = 32;
const CHUNK: usize = 64 * 1024;

#[derive(Default)]
pub(in crate::wayland) struct Transfers(Vec<Transfer>);

enum Transfer {
    Read {
        file: File,
        bytes: Vec<u8>,
        max_bytes: Option<usize>,
        reply: oneshot::Sender<Result<String>>,
        focus: std::rc::Weak<()>,
    },
    Write {
        file: File,
        text: Arc<str>,
        offset: usize,
    },
}

impl Transfers {
    pub(super) fn read(
        &mut self,
        focus: std::rc::Weak<()>,
        max_bytes: Option<usize>,
    ) -> Result<(OwnedFd, oneshot::Receiver<Result<String>>)> {
        ensure!(
            self.0.len() < MAX_PENDING,
            "too many pending clipboard transfers"
        );
        let (read, write) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)?;
        nonblocking(read.as_fd())?;
        let (reply, result) = oneshot::channel();
        self.0.push(Transfer::Read {
            file: read.into(),
            bytes: Vec::new(),
            max_bytes,
            reply,
            focus,
        });
        Ok((write, result))
    }

    pub(super) fn write(&mut self, fd: OwnedFd, text: Arc<str>) -> Result<()> {
        ensure!(
            self.0.len() < MAX_PENDING,
            "too many pending clipboard transfers"
        );
        nonblocking(fd.as_fd())?;
        self.0.push(Transfer::Write {
            file: fd.into(),
            text,
            offset: 0,
        });
        Ok(())
    }

    pub(in crate::wayland) fn poll_fds(&self) -> impl Iterator<Item = PollFd<'_>> {
        self.0.iter().map(|t| match t {
            Transfer::Read { file, .. } => PollFd::new(file, PollFlags::IN),
            Transfer::Write { file, .. } => PollFd::new(file, PollFlags::OUT),
        })
    }

    pub(in crate::wayland) fn discard_cancelled(&mut self) {
        self.0.retain(|transfer| match transfer {
            Transfer::Read { reply, focus, .. } => {
                !reply.is_canceled() && focus.strong_count() != 0
            }
            Transfer::Write { .. } => true,
        });
    }

    pub(in crate::wayland) fn progress(&mut self, ready: &[usize]) {
        for &index in ready.iter().rev() {
            let finished = match &mut self.0[index] {
                Transfer::Read {
                    file,
                    bytes,
                    max_bytes,
                    ..
                } => {
                    let mut chunk = [0; CHUNK];
                    match file.read(&mut chunk) {
                        Ok(0) => Some(String::from_utf8(std::mem::take(bytes)).map_err(Into::into)),
                        Ok(n) => {
                            if let Some(limit) = *max_bytes
                                && n > limit - bytes.len()
                            {
                                Some(Err(anyhow::anyhow!("clipboard text exceeds {limit} bytes")))
                            } else if let Err(e) = bytes.try_reserve(n) {
                                Some(Err(e.into()))
                            } else {
                                bytes.extend_from_slice(&chunk[..n]);
                                None
                            }
                        }
                        Err(e) if retry(&e) => None,
                        Err(e) => Some(Err(e.into())),
                    }
                }
                Transfer::Write { file, text, offset } => {
                    let end = (*offset + CHUNK).min(text.len());
                    match file.write(&text.as_bytes()[*offset..end]) {
                        Ok(n) => {
                            *offset += n;
                            if n == 0 || *offset == text.len() {
                                self.0.swap_remove(index);
                                continue;
                            }
                        }
                        Err(e) if retry(&e) => {}
                        Err(e) => {
                            if e.kind() != io::ErrorKind::BrokenPipe {
                                log::warn!("clipboard write failed: {e}");
                            }
                            self.0.swap_remove(index);
                            continue;
                        }
                    }
                    None
                }
            };
            if let Some(result) = finished
                && let Transfer::Read { reply, .. } = self.0.swap_remove(index)
            {
                let _ = reply.send(result);
            }
        }
    }
}
fn retry(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}

fn nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    Ok(fcntl_setfl(fd, fcntl_getfl(fd)? | OFlags::NONBLOCK)?)
}

#[cfg(test)]
#[path = "../../../tests/unit/clipboard.rs"]
mod tests;
