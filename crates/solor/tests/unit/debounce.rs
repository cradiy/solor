use super::*;

#[test]
fn repeated_attempts_postpone_the_next_accepted_call() {
    let gate = Debounce::new(Duration::from_millis(300));
    let start = Instant::now();
    let attempt = |ms| gate.accept_at(start + Duration::from_millis(ms));

    assert!(attempt(0));
    assert!(!attempt(250));
    assert!(!attempt(500));
    assert!(attempt(800));
}
