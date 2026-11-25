use super::{Anchor, ExclusiveZone, KeyboardInteractivity};
use solor::Pixels;
use std::io;

pub(crate) fn units(pixels: Pixels) -> io::Result<i32> {
    let value = f64::from(pixels).round();
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
        return Err(invalid(
            "layer surface length is outside the Wayland coordinate range",
        ));
    }
    Ok(value as i32)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub(crate) fn extent(value: Option<Pixels>) -> io::Result<u32> {
    match value {
        None => Ok(0),
        Some(value) => {
            let size = units(value)?;
            if size <= 0 {
                return Err(invalid(
                    "layer surface extent must round to a positive logical size",
                ));
            }
            Ok(size as u32)
        }
    }
}

impl ExclusiveZone {
    pub(crate) fn units(self) -> io::Result<i32> {
        match self {
            Self::Avoid => Ok(0),
            Self::Ignore => Ok(-1),
            Self::Reserve(value) => {
                let zone = units(value)?;
                if zone <= 0 {
                    return Err(invalid("reserved distance must be positive"));
                }
                Ok(zone)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct LayerChanges {
    pub size: Option<(u32, u32)>,
    pub zone: Option<i32>,
    pub keyboard: Option<KeyboardInteractivity>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerRules {
    pub anchor: Anchor,
    pub movable: bool,
    pub version: u32,
}

impl LayerRules {
    pub fn validate(self, changes: LayerChanges) -> io::Result<()> {
        if let Some((width, height)) = changes.size {
            if self.movable && (width == 0 || height == 0) {
                return Err(invalid("a movable panel needs fixed dimensions"));
            }
            if (width == 0 && !self.anchor.contains(Anchor::Left | Anchor::Right))
                || (height == 0 && !self.anchor.contains(Anchor::Top | Anchor::Bottom))
            {
                return Err(invalid(
                    "an automatic layer surface extent requires both opposite anchors",
                ));
            }
        }
        if self.movable && changes.zone.is_some_and(|zone| zone > 0) {
            return Err(invalid("a movable panel cannot reserve desktop space"));
        }
        if changes.keyboard == Some(KeyboardInteractivity::OnDemand) && self.version < 4 {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "on-demand focus requires layer-shell version 4",
            ));
        }
        Ok(())
    }
}
