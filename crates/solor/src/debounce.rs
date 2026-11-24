use crate::Context;
use anyhow::Result;
use std::{cell::Cell, time::Duration};
use web_time::Instant;

/// Accepts the first call in a burst. Share one gate with `Rc` to debounce several handlers together.
pub struct Debounce {
    quiet: Duration,
    last_attempt: Cell<Option<Instant>>,
}

impl Debounce {
    /// Creates a gate that opens again after the given quiet period.
    pub fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            last_attempt: Cell::new(None),
        }
    }

    /// Returns whether to handle this call. Every call restarts the quiet period.
    pub fn accept(&self) -> bool {
        self.accept_at(Instant::now())
    }

    fn accept_at(&self, now: Instant) -> bool {
        self.last_attempt
            .replace(Some(now))
            .is_none_or(|last| now.duration_since(last) >= self.quiet)
    }
}

/// Runs a UI callback on the first call in each burst.
///
/// ```
/// use solor::{component, debounce, div};
/// use std::time::Duration;
///
/// let button = component(|cx| {
///     let count = cx.signal(0);
///     div().on_click(debounce(Duration::from_millis(300), move |cx| {
///         cx.update(count, |n| *n += 1)
///     }))
/// });
/// ```
pub fn debounce(
    quiet: Duration,
    handler: impl Fn(&mut Context<'_>) -> Result<()>,
) -> impl Fn(&mut Context<'_>) -> Result<()> {
    let gate = Debounce::new(quiet);
    move |cx| {
        if gate.accept() {
            handler(cx)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/debounce.rs"]
mod tests;
