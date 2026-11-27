use super::{Poll, channel};
use crate::{app::MacApplication, wake::MacWake};
use std::sync::mpsc;

#[test]
fn channel_delivers_bounded_batches_and_closes_when_senders_drop() {
    let wake = MacWake::new();
    let app = MacApplication::empty_for_test();
    let received = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let received_ref = received.clone();

    let (sender, mut inbox) = channel(70, &wake, move |_, msg: usize| {
        received_ref.borrow_mut().push(msg);
    });
    assert!(matches!(inbox(&app), Poll::Idle));

    for i in 0..70 {
        sender.try_send(i).expect("within capacity");
    }
    assert_eq!(sender.try_send(999), Err(mpsc::TrySendError::Full(999)));

    assert!(matches!(inbox(&app), Poll::More));
    assert_eq!(received.borrow().len(), 64);

    assert!(matches!(inbox(&app), Poll::Idle));
    assert_eq!(received.borrow().len(), 70);
    assert_eq!(*received.borrow().first().unwrap(), 0);
    assert_eq!(*received.borrow().last().unwrap(), 69);

    let cloned = sender.clone();
    cloned.try_send(70).unwrap();
    drop(sender);
    assert!(matches!(inbox(&app), Poll::Idle));
    assert_eq!(*received.borrow().last().unwrap(), 70);

    drop(cloned);
    assert!(matches!(inbox(&app), Poll::Closed));
}
