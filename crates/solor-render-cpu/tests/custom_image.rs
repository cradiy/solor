//! Regional image rendering through the normal layout, damage and CPU paint paths.
use anyhow::Result;
use solor::{
    CustomImage, ImageAsset, ImageRegion, ImageRenderer, Styled, Ui, component, div, image,
    layout::Size,
    percent,
    platform::{WindowEvent, WindowHandler},
    px,
    raster::{Clip, DeviceSize, RasterScale},
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

#[derive(Clone)]
struct Tiles {
    changed: Vec<Clip>,
    requests: Rc<RefCell<Vec<ImageRegion>>>,
}

impl ImageRenderer for Tiles {
    fn dimensions(&self) -> DeviceSize {
        DeviceSize {
            width: 8,
            height: 8,
        }
    }
    fn render(&self, region: ImageRegion) -> Result<ImageAsset> {
        self.requests.borrow_mut().push(region);
        let mut bytes = Vec::new();
        for y in 0..region.output.height {
            for x in 0..region.output.width {
                let sx = (region.source_origin[0] + (f64::from(x) + 0.5) * region.source_step[0])
                    .floor() as i64;
                let sy = (region.source_origin[1] + (f64::from(y) + 0.5) * region.source_step[1])
                    .floor() as i64;
                let painted = self
                    .changed
                    .iter()
                    .any(|r| (r.left..r.right).contains(&sx) && (r.top..r.bottom).contains(&sy));
                bytes.extend_from_slice(&[if painted { 240 } else { 12 }, 20, 30, 255]);
            }
        }
        ImageAsset::from_rgba8(region.output.width, region.output.height, bytes)
    }
}

#[test]
fn tile_publication_repairs_the_fitted_region_and_handles_skipped_snapshots() -> Result<()> {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut tiles = Tiles {
        changed: Vec::new(),
        requests: requests.clone(),
    };
    let base = CustomImage::new(tiles.clone())?;
    let a = Clip {
        left: 2,
        top: 2,
        right: 5,
        bottom: 4,
    };
    let b = Clip {
        left: 0,
        top: 0,
        right: 1,
        bottom: 1,
    };
    let c = Clip {
        left: 7,
        top: 7,
        right: 8,
        bottom: 8,
    };
    tiles.changed.push(a);
    let first = base.updated_region(a, tiles.clone())?;
    tiles.changed.push(b);
    let skipped = first.updated_region(b, tiles.clone())?;
    tiles.changed.push(c);
    let last = skipped.updated_region(c, tiles.clone())?;
    tiles.changed = vec![a];
    let batch = last.updated_regions([b, c], tiles.clone())?;
    assert!(last.updated_regions([], tiles.clone()).is_err());
    assert!(
        last.updated_regions([b, Clip { right: 9, ..c }], tiles)
            .is_err()
    );
    let snapshots = [first.clone(), last.clone(), batch.clone()];
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let current = cx.signal(base);
            let mut pending = snapshots.into_iter();
            div()
                .items_stretch()
                .on_frame(move |_, cx| {
                    if let Some(next) = pending.next() {
                        cx.set(current, next)?;
                    }
                    Ok(())
                })
                .child(image(current).w_full().h_full())
        }),
    )?;
    let size = Size {
        width: px(12.),
        height: px(8.),
    };
    let scale = RasterScale(1.5);
    window.frame(size, scale)?;
    window.frame_submitted()?;
    let canvas = Clip::canvas(scale.buffer_size(size)?);
    let partial = Clip {
        left: 6,
        top: 3,
        right: 11,
        bottom: 6,
    };
    for (snapshot, expected, output) in [
        (
            first,
            partial,
            DeviceSize {
                width: 5,
                height: 3,
            },
        ),
        (
            last,
            canvas,
            DeviceSize {
                width: 12,
                height: 12,
            },
        ),
    ] {
        requests.borrow_mut().clear();
        window.event(WindowEvent::FrameReady(Duration::ZERO))?;
        let frame = window.frame(size, scale)?;
        assert_eq!(frame.damage.bounds(), Some(expected));
        assert_eq!(requests.borrow().len(), 1);
        assert_eq!(requests.borrow()[0].output, output);
        let mut reference = CpuWindow::mount(Ui::new(), component(move |_| image(snapshot)))?;
        assert_eq!(frame.rgba, reference.frame(size, scale)?.rgba);
        window.frame_submitted()?;
    }

    // At this zoom the corner tiles leave a wide span of unchanged output.
    let scale = RasterScale(2.5);
    window.frame(size, scale)?;
    window.frame_submitted()?;
    requests.borrow_mut().clear();
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let frame = window.frame(size, scale)?;
    let corners = [
        Clip {
            left: 5,
            top: 0,
            right: 8,
            bottom: 3,
        },
        Clip {
            left: 22,
            top: 17,
            right: 25,
            bottom: 20,
        },
    ];
    assert_eq!(frame.damage.rects().len(), 2);
    for corner in corners {
        assert!(frame.damage.rects().contains(&corner));
    }
    let tile_size = DeviceSize {
        width: 3,
        height: 3,
    };
    assert_eq!(requests.borrow().len(), 2);
    assert!(
        requests
            .borrow()
            .iter()
            .all(|request| request.output == tile_size)
    );
    let mut reference = CpuWindow::mount(Ui::new(), component(move |_| image(batch)))?;
    assert_eq!(frame.rgba, reference.frame(size, scale)?.rgba);
    Ok(())
}

#[test]
fn bound_custom_background_repairs_a_positioned_tile_inside_rounded_corners() -> Result<()> {
    fn panel(background: impl Into<solor::style::Background>) -> impl solor::IntoElement {
        div()
            .w_full()
            .h_full()
            .bg(background)
            .object_fit(ObjectFit::None)
            .object_position_x(percent(100.))
            .rounded(px(4.))
    }
    let requests = Rc::new(RefCell::new(Vec::new()));
    let mut tiles = Tiles {
        changed: Vec::new(),
        requests: requests.clone(),
    };
    let base = CustomImage::new(tiles.clone())?;
    let tile = Clip {
        left: 4,
        top: 0,
        right: 8,
        bottom: 4,
    };
    tiles.changed.push(tile);
    let next = base.updated_region(tile, tiles)?;
    let reference_image = next.clone();
    let mut window = CpuWindow::mount(
        Ui::new(),
        component(move |cx| {
            let source = cx.signal(base);
            div()
                .on_frame(move |_, cx| cx.set(source, next.clone()))
                .child(panel(source))
        }),
    )?;
    let size = Size {
        width: px(14.),
        height: px(8.),
    };
    let scale = RasterScale::ONE;
    let before = window.frame(size, scale)?.rgba.to_vec();
    window.frame_submitted()?;
    requests.borrow_mut().clear();
    window.event(WindowEvent::FrameReady(Duration::ZERO))?;
    let frame = window.frame(size, scale)?;
    assert_eq!(
        frame.damage.rects(),
        &[Clip {
            left: 10,
            right: 14,
            ..tile
        }]
    );
    let calls = requests.take();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].source_origin, [4., 0.]);
    assert_eq!(
        calls[0].output,
        DeviceSize {
            width: 4,
            height: 4
        }
    );
    let inside = 4 * (2 * 14 + 11);
    assert_eq!(&frame.rgba[inside..inside + 4], &[240, 20, 30, 255]);
    assert_ne!(frame.rgba[inside], before[inside]);
    assert_eq!(frame.rgba[4 * 13..4 * 14], before[4 * 13..4 * 14]);
    let mut reference = CpuWindow::mount(Ui::new(), component(move |_| panel(reference_image)))?;
    assert_eq!(frame.rgba, reference.frame(size, scale)?.rgba);
    Ok(())
}
