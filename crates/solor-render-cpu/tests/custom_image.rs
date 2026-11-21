//! Regional image rendering through the normal layout, damage and CPU paint paths.
use anyhow::Result;
use solor::{
    CustomImage, ImageAsset, ImageRegion, ImageRenderer, Styled, Ui, component, div, image,
    layout::Size,
    percent,
    platform::{WindowEvent, WindowHandler},
    px,
    raster::{DeviceSize, RasterScale},
    rgb,
    style::ObjectFit,
};
use solor_render_cpu::CpuWindow;
use std::{cell::RefCell, rc::Rc, time::Duration};

struct Pattern(Rc<RefCell<Vec<ImageRegion>>>);

impl ImageRenderer for Pattern {
    fn dimensions(&self) -> DeviceSize {
        DeviceSize {
            width: 131072,
            height: 131072,
        }
    }
    fn render(&self, region: ImageRegion) -> Result<ImageAsset> {
        self.0.borrow_mut().push(region);
        let mut bytes = Vec::new();
        for y in 0..region.output.height {
            for x in 0..region.output.width {
                let sx = (region.source_origin[0] + (f64::from(x) + 0.5) * region.source_step[0])
                    .floor() as u64;
                let sy = (region.source_origin[1] + (f64::from(y) + 0.5) * region.source_step[1])
                    .floor() as u64;
                bytes.extend_from_slice(&[sx as u8, sy as u8, 70, 255]);
            }
        }
        ImageAsset::from_rgba8(region.output.width, region.output.height, bytes)
    }
}

#[test]
fn fractional_scale_repairs_only_the_damaged_source_region() -> Result<()> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let bitmap = CustomImage::new(Pattern(calls.clone()))?;
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let overlay = cx.signal(true);
            div()
                .items_center()
                .justify_center()
                .bg(bitmap)
                .object_fit(ObjectFit::None)
                .on_frame_when(overlay, move |_, cx| cx.set(overlay, false))
                .child_when(overlay, || div().size(px(4.)).bg(rgb(0xffffff)))
        }),
    )?;
    assert!(calls.borrow().is_empty());
    let size = Size {
        width: px(16.),
        height: px(12.),
    };
    let scale = RasterScale(1.5);
    let full = window.frame(size, scale)?.rgba.to_vec();
    window.frame_submitted()?;
    let first = calls.borrow_mut().pop().unwrap();
    assert_eq!(
        first.output,
        DeviceSize {
            width: 24,
            height: 18
        }
    );
    assert_eq!(first.source_step, [2. / 3., 2. / 3.]);
    window.frame(size, scale)?;
    window.frame_submitted()?;
    assert!(calls.borrow().is_empty());

    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let frame = window.frame(size, scale)?;
    let partial = calls.borrow_mut().pop().unwrap();
    let dirty = frame.damage.bounds().unwrap();
    assert_eq!(
        partial.output,
        DeviceSize {
            width: 6,
            height: 6
        }
    );
    assert_eq!(partial.source_step, first.source_step);
    for (axis, offset) in [dirty.left, dirty.top].into_iter().enumerate() {
        assert!(
            (partial.source_origin[axis]
                - first.source_origin[axis]
                - offset as f64 * first.source_step[axis])
                .abs()
                < 1e-9
        );
    }
    for y in 0..18 {
        for x in 0..24 {
            let i = (y * 24 + x) * 4;
            if (dirty.left..dirty.right).contains(&(x as i64))
                && (dirty.top..dirty.bottom).contains(&(y as i64))
            {
                let sx = (first.source_origin[0] + (x as f64 + 0.5) * first.source_step[0]).floor()
                    as u64;
                let sy = (first.source_origin[1] + (y as f64 + 0.5) * first.source_step[1]).floor()
                    as u64;
                assert_eq!(&frame.rgba[i..i + 4], &[sx as u8, sy as u8, 70, 255]);
            } else {
                assert_eq!(frame.rgba[i..i + 4], full[i..i + 4]);
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn application_reads_only_visible_scanlines_of_a_sparse_raw_image() -> Result<()> {
    use std::{cell::Cell, fs::File, os::unix::fs::FileExt};
    const SIZE: DeviceSize = DeviceSize {
        width: 65536,
        height: 32768,
    };
    struct Raw {
        file: File,
        bytes_read: Rc<Cell<usize>>,
    }
    impl ImageRenderer for Raw {
        fn dimensions(&self) -> DeviceSize {
            SIZE
        }
        fn render(&self, r: ImageRegion) -> Result<ImageAsset> {
            assert_eq!(r.source_step, [1., 1.]);
            let [x, y] = r.source_origin.map(|v| v as u64);
            let stride = r.output.width as usize * 4;
            let mut bytes = vec![0; stride * r.output.height as usize];
            for (row, buf) in bytes.chunks_exact_mut(stride).enumerate() {
                self.file
                    .read_exact_at(buf, 4 * ((y + row as u64) * u64::from(SIZE.width) + x))?;
                self.bytes_read.set(self.bytes_read.get() + buf.len());
            }
            ImageAsset::from_rgba8(r.output.width, r.output.height, bytes)
        }
    }
    let path = std::env::temp_dir().join(format!("solor-regional-{}.rgba", std::process::id()));
    let file = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::fs::remove_file(path)?;
    let extent = u64::from(SIZE.width) * u64::from(SIZE.height) * 4;
    file.set_len(extent)?;
    file.write_all_at(&[91, 32, 77, 255], extent - 4)?;
    let bytes_read = Rc::new(Cell::new(0));
    let bitmap = CustomImage::new(Raw {
        file,
        bytes_read: bytes_read.clone(),
    })?;
    let mut win = CpuWindow::mount(
        Ui::new(),
        component(move |_| {
            image(bitmap)
                .object_fit(ObjectFit::None)
                .object_position_x(percent(100.))
                .object_position_y(percent(100.))
        }),
    )?;
    assert_eq!(bytes_read.get(), 0);
    let frame = win.frame(
        Size {
            width: px(8.),
            height: px(4.),
        },
        RasterScale::ONE,
    )?;
    assert_eq!(&frame.rgba[frame.rgba.len() - 4..], &[91, 32, 77, 255]);
    assert_eq!(bytes_read.get(), 8 * 4 * 4);
    Ok(())
}
