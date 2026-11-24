//! Presentation feedback and per-window animation timing.
use super::Desktop;
use rustix::time::{ClockId, clock_gettime};
use std::time::Duration;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, backend::ObjectId};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};

// Client dispatch and enqueue stamps use this clock irrespective of the server clock.
pub(super) fn monotonic_ns() -> i128 {
    let time = clock_gettime(ClockId::Monotonic);
    i128::from(time.tv_sec) * 1_000_000_000 + i128::from(time.tv_nsec)
}

#[derive(Default)]
pub(super) struct FrameClock {
    refresh: Option<(i128, u32)>,
    last: Option<Duration>,
}

impl FrameClock {
    pub(super) fn clear_refresh(&mut self) {
        self.refresh = None;
    }

    pub(super) fn sample(&mut self, elapsed: Duration) -> Duration {
        let target = if let Some((stamp, period)) = self.refresh {
            let period = Duration::from_nanos(u64::from(period));
            let until_refresh =
                (stamp - elapsed.as_nanos() as i128).rem_euclid(period.as_nanos() as i128);
            let next = elapsed + Duration::from_nanos(until_refresh as u64);
            // Consecutive callbacks can straddle one vblank with both frames queued.
            if self.last.is_some_and(|last| next < last + period / 2) {
                next + period
            } else {
                next
            }
        } else {
            elapsed
        };
        let time = self.last.unwrap_or_default().max(target);
        self.last = Some(time);
        time
    }
}

impl Dispatch<wp_presentation::WpPresentation, ()> for Desktop {
    fn event(
        state: &mut Self,
        _: &wp_presentation::WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.monotonic_presentation = clk_id == ClockId::Monotonic as u32;
            if state.trace_submissions {
                eprintln!("presentation_clock_id={clk_id}");
            }
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, (ObjectId, usize)> for Desktop {
    fn event(
        state: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        (surface, commit): &(ObjectId, usize),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
                refresh,
                seq_hi,
                seq_lo,
                flags,
            } => {
                let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                if let Some(window) = state.windows.get_mut(surface)
                    && window.mapping == super::Mapping::Mapped
                {
                    let vsync = matches!(flags, WEnum::Value(kind) if kind.contains(wp_presentation_feedback::Kind::Vsync));
                    window.frame_clock.refresh =
                        (state.monotonic_presentation && vsync && refresh != 0).then(|| {
                            let stamp = i128::from(seconds) * 1_000_000_000 + i128::from(tv_nsec);
                            let offset = stamp - monotonic_ns();
                            (window.start.elapsed().as_nanos() as i128 + offset, refresh)
                        });
                }
                if !state.trace_submissions {
                    return;
                }
                let sequence = (u64::from(seq_hi) << 32) | u64::from(seq_lo);
                eprintln!(
                    "presented={commit} time={seconds}.{tv_nsec:09} sequence={sequence} refresh_ns={refresh}"
                );
            }
            wp_presentation_feedback::Event::Discarded if state.trace_submissions => {
                eprintln!("discarded={commit}")
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/frame_clock.rs"]
mod tests;
