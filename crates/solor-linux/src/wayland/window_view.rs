use anyhow::Result;
use solor::{
    platform::{EventResponse, WindowEvent, WindowHandler},
    pointer::PointerId,
    raster::Rgba8,
    style::Cursor,
    text::{CaretBounds, SurroundingText},
};
use std::{rc::Rc, task::Waker, time::Instant};
pub(super) enum WindowView {
    Shm(Box<dyn WindowHandler<Rgba8>>),
    #[cfg(feature = "gpu")]
    Gpu(Box<dyn WindowHandler<super::gpu::Native>>, u32),
}
macro_rules! delegate {
    ($(fn $method:ident([$($receiver:tt)*] $this:ident $(,$arg:ident:$ty:ty)*) -> $ret:ty;)+) => {$(
        pub(super) fn $method($($receiver)* $this $(,$arg:$ty)*) -> $ret {
            match $this {
                Self::Shm(v)=>v.$method($($arg),*),
                #[cfg(feature="gpu")]
                Self::Gpu(v,_)=>v.$method($($arg),*),
            }
        }
    )+};
}
impl WindowView {
    pub(super) fn buffer_size(
        &self,
        scale_120: u32,
        logical: solor::layout::Size,
    ) -> std::io::Result<Option<solor::raster::DeviceSize>> {
        match self {
            Self::Shm(_) => super::budgeted_buffer_size(scale_120, logical),
            #[cfg(feature = "gpu")]
            Self::Gpu(_, max) => {
                let size =
                    solor::raster::RasterScale(f64::from(scale_120) / 120.).buffer_size(logical)?;
                Ok((size.width.max(size.height) <= *max).then_some(size))
            }
        }
    }

    delegate! {
        fn surrounding_text([&mut] self,max:usize)->Option<SurroundingText<'_>>;
        fn event([&mut] self,event:WindowEvent)->Result<EventResponse>;
        fn poll_tasks([&mut] self,waker:&Waker)->Result<()>;
        fn set_clipboard([&mut] self,clipboard:Rc<dyn solor::clipboard::PlatformClipboard>)->();
        fn set_window_resizable([&mut] self,resizable:bool)->Result<()>;
        fn frame_submitted([&mut] self)->Result<()>;
    }
    delegate! {
        fn pointer_cursor([&] self,id:PointerId)->Cursor;
        fn text_input_session([&] self)->Option<u64>;

        fn caret_bounds([&] self)->Option<CaretBounds>;
        fn next_redraw([&] self)->Option<Instant>;
    }
}
