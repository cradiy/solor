use super::*;
use solor::text::TextInputState;
use std::os::unix::net::UnixStream;

#[test]
fn done_discards_old_focus_and_commits_before_new_preedit() -> anyhow::Result<()> {
    let (client, _server) = UnixStream::pair()?;
    let conn = Connection::from_socket(client)?;
    let device = ZwpTextInputV3::inert(conn.backend().downgrade());
    let mut input = SeatInput::new(device, 12);
    input.serial = 3;
    input.enabled_serial = 3;
    input.session = Some(9);
    for (serial, target) in [(1, Some(9)), (3, Some(11)), (3, None)] {
        input.batch.commit = Some("old".into());
        assert!(input.take_batch(serial, target).is_none());
        let (_, batch) = input.take_batch(3, Some(9)).unwrap();
        assert_eq!(batch.edits(false).count(), 0);
    }

    let mut repeated = TextInputState::new("abcdefgh".repeat(1000));
    repeated.select(TextSelection::caret(2048))?;
    input.surrounding = repeated
        .surrounding_text(MAX_SURROUNDING_BYTES)
        .map(SurroundingText::into_owned);
    input.serial = 4;
    assert!(input.take_batch(3, Some(9)).is_some());
    assert!(!input.accepts_deletion(3, repeated.surrounding_text(MAX_SURROUNDING_BYTES), 1, 0));
    assert!(input.accepts_deletion(4, repeated.surrounding_text(MAX_SURROUNDING_BYTES), 1, 0));
    repeated.select(TextSelection::caret(4096))?;
    assert!(!input.accepts_deletion(4, repeated.surrounding_text(MAX_SURROUNDING_BYTES), 1, 0));
    input.batch.delete = Some((1, 0));
    input.batch.preedit = Some(preedit(Some("拼\u{1f600}".into()), 0, 7));
    input.batch.commit = Some("好".into());
    let mut state = TextInputState::new("你!");
    state.set_preedit("hao", None)?;
    let (_, batch) = input.take_batch(4, Some(9)).unwrap();
    for edit in batch.edits(true) {
        match edit {
            TextInputEvent::DeleteSurrounding { before, after } => {
                state.delete_surrounding(before, after)?
            }
            TextInputEvent::Commit(value) => state.commit(&value),
            TextInputEvent::Preedit(p) => state.set_preedit(p.text, p.selection)?,
        }
    }
    assert_eq!(state.display_segments(), ["你好", "拼\u{1f600}", ""]);
    assert_eq!(
        state.preedit().unwrap().selection,
        Some(TextSelection { anchor: 7, head: 0 })
    );
    let (_, empty) = input.take_batch(3, Some(9)).unwrap();
    assert!(
        matches!(empty.edits(true).next(), Some(TextInputEvent::Preedit(p)) if p.text.is_empty())
    );
    Ok(())
}
