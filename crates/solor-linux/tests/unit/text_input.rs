use super::*;
use solor::text::TextInputState;
use std::os::unix::net::UnixStream;

use super::super::application::fixture::Host;
use solor::{
    Ui, ViewId, component, div,
    layout::{Origin, Size},
    platform::{EventResponse, WindowHandler},
    px,
    raster::{RasterFrame, RasterScale, Rgba8},
    text::CaretBounds,
};
use std::{cell::Cell, rc::Rc};

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
    input.surrounding_serial = 4;
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

struct Field {
    ui: Ui,
    view: ViewId,
    caret_x: Rc<Cell<f32>>,
}

impl WindowHandler<Rgba8> for Field {
    fn text_input_session(&self) -> Option<u64> {
        self.ui.text_input_session(self.view).unwrap()
    }
    fn caret_bounds(&self) -> Option<CaretBounds> {
        Some(CaretBounds {
            origin: Origin {
                x: px(self.caret_x.get()),
                y: px(0.),
            },
            size: Size {
                width: px(1.),
                height: px(18.),
            },
        })
    }
    fn surrounding_text(&mut self, max: usize) -> Option<SurroundingText<'_>> {
        self.ui
            .focused_text_input(self.view)
            .unwrap()?
            .surrounding_text(max)
    }
    fn event(&mut self, event: WindowEvent) -> anyhow::Result<EventResponse> {
        if let WindowEvent::TextInput(edit) = event {
            self.ui.dispatch_text_input(self.view, edit)?;
        }
        Ok(Default::default())
    }
    fn frame(&mut self, _: Size, _: RasterScale) -> anyhow::Result<RasterFrame<'_>> {
        unreachable!("text-input dispatch does not render a frame")
    }
}

struct Editor {
    host: Host,
    id: ObjectId,
    device: ZwpTextInputV3,
    caret_x: Rc<Cell<f32>>,
}

impl Editor {
    fn open() -> anyhow::Result<Self> {
        let mut host = Host::new()?;
        let mut ui = Ui::new();
        let input = ui.context().signal(TextInputState::new("cat"));
        let view = ui.mount(component(|_| div().text_input(input)))?;
        ui.focus(view, ui.root(view)?)?;
        let caret_x = Rc::new(Cell::new(4.));
        let id = host.open(Field {
            ui,
            view,
            caret_x: caret_x.clone(),
        })?;
        let window = host.desktop().windows.get_mut(&id).unwrap();
        window.mapping = Mapping::Mapped;
        let surface = window.surface.clone();
        let device = host.desktop().seats[0]
            .text_input
            .as_ref()
            .unwrap()
            .device
            .clone();
        host.deliver(&device, Event::Enter { surface });
        Ok(Self {
            host,
            id,
            device,
            caret_x,
        })
    }

    fn serial(&mut self) -> u32 {
        self.host.desktop().seats[0]
            .text_input
            .as_ref()
            .unwrap()
            .serial
    }
    fn text(&mut self) -> String {
        let window = self.host.desktop().windows.get_mut(&self.id).unwrap();
        assert!(window.failure.is_none());
        SeatInput::context(window).unwrap().text().to_owned()
    }
    fn replace(&mut self, serial: u32, value: &str) {
        self.host.deliver(
            &self.device,
            Event::DeleteSurroundingText {
                before_length: 1,
                after_length: 0,
            },
        );
        self.host.deliver(
            &self.device,
            Event::CommitString {
                text: Some(value.to_owned()),
            },
        );
        self.host.deliver(&self.device, Event::Done { serial });
    }
}

#[test]
fn late_replacement_after_caret_motion_waits_for_matching_done_to_publish() -> anyhow::Result<()> {
    let mut editor = Editor::open()?;
    let old = editor.serial();
    editor.caret_x.set(16.);
    editor.host.desktop().sync_text_inputs();
    let current = editor.serial();
    assert_eq!(current, old.wrapping_add(1));
    editor.replace(old, "r");
    assert_eq!(editor.text(), "car");
    editor.host.desktop().sync_text_inputs();
    assert_eq!(editor.serial(), current);
    editor
        .host
        .deliver(&editor.device, Event::Done { serial: current });
    assert_eq!(editor.serial(), current.wrapping_add(1));
    let published = editor.host.desktop().seats[0]
        .text_input
        .as_ref()
        .unwrap()
        .surrounding
        .as_ref()
        .unwrap();
    assert_eq!(published.text(), "car");
    Ok(())
}

#[test]
fn late_replacement_after_edit_preserves_text_until_valid_replacement() -> anyhow::Result<()> {
    let mut editor = Editor::open()?;
    let old = editor.serial();
    editor
        .host
        .desktop()
        .windows
        .get_mut(&editor.id)
        .unwrap()
        .event(WindowEvent::TextInput(TextInputEvent::Commit("s".into())));
    editor.host.desktop().sync_text_inputs();
    let current = editor.serial();
    assert_eq!(current, old.wrapping_add(1));
    editor.replace(old, "r");
    assert_eq!(editor.text(), "cats");
    editor.replace(current, "!");
    assert_eq!(editor.text(), "cat!");
    assert_eq!(editor.serial(), current.wrapping_add(1));
    Ok(())
}
