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

    /// A lock must cover the configured output even if its preferred pixel density exceeds the budget.
    pub(super) fn lock_buffer_size(
        &self,
        preferred: u32,
        logical: solor::layout::Size,
    ) -> std::io::Result<(u32, Option<solor::raster::DeviceSize>)> {
        if let Some(size) = self.buffer_size(preferred, logical)? {
            return Ok((preferred, Some(size)));
        }
        let (mut low, mut high) = (0, preferred);
        let mut best = None;
        while high - low > 1 {
            let scale = low + (high - low) / 2;
            if let Some(size) = self.buffer_size(scale, logical)? {
                low = scale;
                best = Some(size);
            } else {
                high = scale;
            }
        }
        Ok((low, best))
    }

    delegate! {
        fn surrounding_text([&mut] self,max:usize)->Option<SurroundingText<'_>>;
        fn drop_files([&mut] self,target:solor::NodeId,paths:&[std::path::PathBuf])->Result<bool>;
        fn event([&mut] self,event:WindowEvent)->Result<EventResponse>;
        fn poll_tasks([&mut] self,waker:&Waker)->Result<()>;
        fn set_clipboard([&mut] self,clipboard:Rc<dyn solor::clipboard::PlatformClipboard>)->();
        fn set_window_resizable([&mut] self,resizable:bool)->Result<()>;
        fn frame_submitted([&mut] self)->Result<()>;
    }
    delegate! {
        fn file_drop_target([&] self,position:solor::layout::Origin)->Option<solor::NodeId>;
        fn pointer_cursor([&] self,id:PointerId)->Cursor;
        fn text_input_session([&] self)->Option<u64>;

        fn caret_bounds([&] self)->Option<CaretBounds>;
        fn next_redraw([&] self)->Option<Instant>;
    }
}
