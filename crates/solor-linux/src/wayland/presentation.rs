//! Presentation timestamps for traced buffer submissions.
use super::Desktop;
use rustix::time::{ClockId, clock_gettime};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::presentation_time::client::{wp_presentation, wp_presentation_feedback};

// Client dispatch and enqueue stamps use this clock irrespective of the server clock.
pub(super) fn monotonic_ns() -> i128 {
    let time = clock_gettime(ClockId::Monotonic);
    i128::from(time.tv_sec) * 1_000_000_000 + i128::from(time.tv_nsec)
}

impl Dispatch<wp_presentation::WpPresentation, ()> for Desktop {
    fn event(
        _: &mut Self,
        _: &wp_presentation::WpPresentation,
        event: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            eprintln!("presentation_clock_id={clk_id}");
        }
    }
}

impl Dispatch<wp_presentation_feedback::WpPresentationFeedback, usize> for Desktop {
    fn event(
        _: &mut Self,
        _: &wp_presentation_feedback::WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        commit: &usize,
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
                ..
            } => {
                let seconds = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                let sequence = (u64::from(seq_hi) << 32) | u64::from(seq_lo);
                eprintln!(
                    "presented={commit} time={seconds}.{tv_nsec:09} sequence={sequence} refresh_ns={refresh}"
                );
            }
            wp_presentation_feedback::Event::Discarded => eprintln!("discarded={commit}"),
            _ => {}
        }
    }
}
