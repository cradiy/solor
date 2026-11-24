use super::{Duration, FrameClock};

#[test]
fn irregular_callbacks_follow_refreshes_and_clock_changes_remain_monotonic() {
    let mut clock = FrameClock {
        refresh: Some((2_000_000, 8_000_000)),
        ..Default::default()
    };
    let ms = Duration::from_millis;
    assert_eq!(clock.sample(ms(7)), ms(10));
    assert_eq!(clock.sample(ms(9)), ms(18));
    assert_eq!(clock.sample(ms(32)), ms(34));

    clock.refresh = None;
    assert_eq!(clock.sample(ms(33)), ms(34));
    assert_eq!(clock.sample(ms(39)), ms(39));
    clock.refresh = Some((40_000_000, 16_000_000));
    assert_eq!(clock.sample(ms(45)), ms(56));
}
