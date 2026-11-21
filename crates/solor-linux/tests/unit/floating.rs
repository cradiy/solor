use super::*;
use solor::pointer::{PointerKind, PointerSample};

fn point(id: u64, x: f32, y: f32) -> PointerSample {
    PointerSample {
        id: PointerId(id),
        kind: PointerKind::Mouse,
        position: Origin { x: px(x), y: px(y) },
        modifiers: Default::default(),
    }
}

#[test]
fn only_the_grabbing_pointer_moves_the_panel_in_output_coordinates() {
    let mut geometry = Geometry::new(
        Size {
            width: px(240.),
            height: px(80.),
        },
        Anchor::Top,
        [16, 0, 0, 0],
    );
    geometry.configure(800, 600);
    assert_eq!(geometry.position, [280, 16]);
    let press = point(1, 299.5, 27.25);
    let mut down = PointerInput::Down(press, PointerButton::Primary);
    geometry.pointer(&mut down);
    let PointerInput::Down(local, _) = down else {
        unreachable!()
    };
    geometry.begin(local.id, local.position);
    for (x, expected) in [(299.75, 280), (300., 281), (340., 321), (340., 321)] {
        geometry.pointer(&mut PointerInput::Move(point(1, x, 40.25)));
        assert_eq!(geometry.position, [expected, 29]);
    }
    geometry.pointer(&mut PointerInput::Move(point(2, 520., 40.)));
    geometry.pointer(&mut PointerInput::Up(
        point(2, 520., 40.),
        PointerButton::Primary,
    ));
    geometry.pointer(&mut PointerInput::Move(point(1, 900., 900.)));
    assert_eq!(geometry.position, [560, 520]);
    geometry.pointer(&mut PointerInput::Move(point(1, 510., 400.)));
    assert_eq!(geometry.position, [491, 389]);
    geometry.pointer(&mut PointerInput::Up(press, PointerButton::Primary));
    geometry.pointer(&mut PointerInput::Move(press));
    assert_eq!(geometry.position, [491, 389]);
    geometry.configure(460, 300);
    assert_eq!(geometry.bounds(), [220, 220, 240, 80]);
}
