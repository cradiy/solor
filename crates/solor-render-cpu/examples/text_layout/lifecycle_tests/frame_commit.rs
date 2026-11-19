//! Single-window acceptance with an in-memory RGBA upload target.
use solor::px;
use std::{io, rc::Rc};

use super::{Fixture, Removal, hit_node};
use crate::{
    damage::RetainedPixels,
    frame::{Frame, Mode, Presentation},
    glyph_masks::GlyphMasks,
    input::Point,
    layout::{Result, Size},
    pixels::{Clip, Surface},
};
use solor::{state::Runtime, tree::NodeId};

struct Accepted {
    frame: Rc<Frame>,
    buffer: Surface,
}

struct Host {
    size: Size,
    pending: bool,
    ready: bool,
    candidate: Option<Rc<Frame>>,
    accepted: Option<Accepted>,
    raster: Option<RetainedPixels>,
    masks: GlyphMasks,
    // A raster update can finish before an upload fails.
    damage_since_acceptance: bool,
    staging: Vec<u8>,
    fail_upload: bool,
    uploaded_bytes: usize,
}

impl Host {
    fn new(size: Size) -> Self {
        Self {
            size,
            pending: true,
            ready: true,
            candidate: None,
            accepted: None,
            raster: None,
            masks: GlyphMasks::new(64, 32 * 1024),
            damage_since_acceptance: false,
            staging: Vec::new(),
            fail_upload: false,
            uploaded_bytes: 0,
        }
    }

    fn request(&mut self, size: Size) {
        self.size = size;
        self.pending = true;
        self.candidate = None;
    }

    fn present(&mut self, prepare: impl FnOnce(Size) -> Result<Rc<Frame>>) -> io::Result<bool> {
        if !self.pending || !self.ready {
            return Ok(false);
        }
        if self.candidate.is_none() {
            self.candidate = Some(prepare(self.size).map_err(io::Error::other)?);
        }
        let frame = self.candidate.as_ref().unwrap();
        let target = crate::pixels::RasterScale::ONE.buffer_size(self.size)?;
        let clip = Clip::canvas(target);
        if let Some(raster) = &mut self.raster {
            self.damage_since_acceptance |= raster
                .update(frame.clone(), target, clip, &mut self.masks)?
                .is_some();
        } else {
            self.raster = Some(RetainedPixels::new(
                frame.clone(),
                target,
                clip,
                &mut self.masks,
            )?);
            self.damage_since_acceptance = true;
        }
        let surface = self.raster.as_ref().unwrap().surface();
        let upload = self.damage_since_acceptance
            && self.accepted.as_ref().is_none_or(|old| {
                old.buffer.size != surface.size || old.buffer.rgba != surface.rgba
            });
        if upload {
            self.staging.clear();
            if std::mem::take(&mut self.fail_upload) {
                let partial = &surface.rgba[..surface.rgba.len() / 2];
                self.staging.extend_from_slice(partial);
                self.uploaded_bytes += partial.len();
                self.ready = false;
                return Err(io::Error::other("injected partial upload"));
            }
            self.staging.extend_from_slice(&surface.rgba);
            self.uploaded_bytes += surface.rgba.len();
            let buffer = Surface {
                size: target,
                rgba: std::mem::take(&mut self.staging),
            };
            self.accepted = Some(Accepted {
                frame: frame.clone(),
                buffer,
            });
        } else {
            self.accepted
                .as_mut()
                .expect("first frame requires upload")
                .frame = frame.clone();
        }
        self.damage_since_acceptance = false;
        self.candidate = None;
        self.pending = false;
        Ok(true)
    }

    fn hit(&self, point: Point, live: impl FnOnce(NodeId) -> bool) -> Option<NodeId> {
        let accepted = self.accepted.as_ref()?;
        hit_node(&accepted.frame, accepted.buffer.size.at_unit_scale(), point)
            .filter(|&id| live(id))
    }

    fn check_pixels(&self) {
        let accepted = self.accepted.as_ref().unwrap();
        let size = accepted.buffer.size;
        let oracle = Surface::draw(&accepted.frame, size, Clip::canvas(size)).unwrap();
        assert_eq!(accepted.buffer.rgba, oracle.rgba);
    }
}

const LEFT: Point = Point { x: 4.0, y: 8.0 };
const RIGHT: Point = Point { x: 20.0, y: 8.0 };

#[test]
fn movement_waits_for_upload_and_resize_supersedes_a_waiting_candidate() {
    for resize in [false, true] {
        let mut host = Host::new(Size {
            width: px(35.0),
            height: px(40.0),
        });
        let mut view = Presentation::new(1, "A", true);
        let mut state = Runtime::new(0_u32);
        let root = state.tree().root();
        let shift = state.create_signal(0_u32);
        state.bind(shift, root, |x, value| *value = *x).unwrap();
        let mut prepare = |state: &mut Runtime<u32>, size, expected_bindings| {
            let count = state.flush();
            assert_eq!(count, expected_bindings);
            if count > 0 {
                view.set_shift(0, *state.tree().get(root).unwrap())?;
            }
            Ok(view.update(size, Mode::Normal)?.frame)
        };
        assert!(host.hit(LEFT, |_| true).is_none());
        assert!(host.present(|size| prepare(&mut state, size, 1)).unwrap());
        let initial = host.accepted.as_ref().unwrap();
        let target = initial.frame.draws[0].node;
        let old_pixels = initial.buffer.rgba.clone();
        assert_eq!(host.hit(LEFT, |_| true), Some(target));
        assert_eq!(host.hit(RIGHT, |_| true), None);

        state.set(shift, 16).unwrap();
        host.request(host.size);
        host.fail_upload = true;
        assert_eq!(
            host.present(|size| prepare(&mut state, size, 1))
                .unwrap_err()
                .to_string(),
            "injected partial upload"
        );
        let failed = host.candidate.as_ref().unwrap().clone();
        assert!(host.pending);
        assert_eq!(host.staging.len(), old_pixels.len() / 2);
        assert_eq!(host.accepted.as_ref().unwrap().buffer.rgba, old_pixels);
        assert_ne!(host.raster.as_ref().unwrap().surface().rgba, old_pixels);
        assert_eq!(host.hit(LEFT, |_| true), Some(target));
        assert_eq!(host.hit(RIGHT, |_| true), None);

        if resize {
            host.request(Size {
                width: px(52.0),
                height: px(6.0),
            });
        }
        assert!(!host.present(|_| panic!("host still blocked")).unwrap());
        assert_eq!(host.hit(LEFT, |_| true), Some(target));
        host.ready = true;
        if resize {
            assert!(host.present(|size| prepare(&mut state, size, 0)).unwrap());
        } else {
            assert!(
                host.present(|_| panic!("retry must reuse prepared frame"))
                    .unwrap()
            );
        }
        let accepted = host.accepted.as_ref().unwrap();
        assert_eq!(Rc::ptr_eq(&accepted.frame, &failed), !resize);
        assert_eq!(accepted.buffer.size.at_unit_scale(), host.size);
        assert_eq!(accepted.frame.geometry[0].metrics.size, host.size);
        assert_eq!(host.hit(LEFT, |_| true), None);
        assert_eq!(host.hit(RIGHT, |_| true), (!resize).then_some(target));
        assert_eq!(
            host.hit(Point { x: RIGHT.x, y: 4.0 }, |_| true),
            Some(target)
        );
        host.check_pixels();
        assert!(!host.pending);
        assert!(!host.present(|_| panic!("idle wakeup")).unwrap());
    }
}

#[test]
fn identical_pixels_accept_the_replacement_hit_identity_without_upload() {
    let mut window = Fixture::new();
    let owner = window.scope(None);
    let old = window.mount(owner, "Label");
    let mut host = Host::new(super::SIZE);
    host.present(|_| window.prepare(Removal::None)).unwrap();
    assert_eq!(host.hit(LEFT, |_| true), Some(old));
    let bytes = host.uploaded_bytes;

    let new = window.mount(owner, "Label");
    let frame = window.prepare(Removal::Leaf(old)).unwrap();
    let live = |id| window.members.iter().any(|m| m.visual == id);
    assert_ne!(old, new);
    assert_eq!(host.hit(LEFT, live), None);
    host.request(host.size);
    assert!(host.present(|_| Ok(frame)).unwrap());
    assert_eq!(host.hit(LEFT, live), Some(new));
    assert_eq!(host.uploaded_bytes, bytes);
    host.check_pixels();
}
