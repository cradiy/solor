//! Primary pointer events in surface-local logical coordinates.
#[derive(Clone, Copy)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy)]
pub enum PointerEvent {
    Down(u64, Point),
    Move(u64, Point),
    Up(u64, Point),
    Cancel(u64),
}
