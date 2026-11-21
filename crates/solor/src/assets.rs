//! Named resources supplied by applications and reusable component libraries.
use crate::{SharedString, SvgAsset, Ui};
use anyhow::{Context as _, Result};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    path::Path,
    rc::{Rc, Weak},
};

/// A named bitmap returned by a resource pack.
pub enum ImageResource<'a> {
    /// An absolute filesystem path. Pixels are decoded on visible paint.
    Path(Cow<'a, Path>),
    /// Encoded bitmap data borrowed from the pack or produced for this lookup.
    Bytes(Cow<'a, [u8]>),
    /// Application-owned image snapshot rendered by visible region.
    Custom(crate::CustomImage),
}

/// Supplies SVG and bitmap resources by name. Return `Ok(None)` for a missing name.
/// Tuples search their members from left to right, stopping on a match or an error.
pub trait AssetSource {
    /// Looks up the encoded bytes of a self-contained SVG.
    fn svg(&self, _name: &str) -> Result<Option<Cow<'_, [u8]>>> {
        Ok(None)
    }

    /// Looks up a bitmap file or its encoded contents.
    fn image(&self, _name: &str) -> Result<Option<ImageResource<'_>>> {
        Ok(None)
    }
}

macro_rules! compose {
    ($($pack:ident $index:tt),+) => {
        impl<$($pack: AssetSource),+> AssetSource for ($($pack,)+) {
            fn svg(&self, name: &str) -> Result<Option<Cow<'_, [u8]>>> {
                $(if let Some(value) = self.$index.svg(name)? { return Ok(Some(value)); })+
                Ok(None)
            }
            fn image(&self, name: &str) -> Result<Option<ImageResource<'_>>> {
                $(if let Some(value) = self.$index.image(name)? { return Ok(Some(value)); })+
                Ok(None)
            }
        }
    };
}
compose!(A 0, B 1);
compose!(A 0, B 1, C 2);
compose!(A 0, B 1, C 2, D 3);

/// A registered resource pack and its shared, live SVG trees. Clones share both.
#[derive(Clone, Default)]
pub struct Assets(Option<Rc<Registry>>);

struct Registry {
    source: Box<dyn AssetSource>,
    trees: RefCell<HashMap<SharedString, Weak<usvg::Tree>>>,
}

impl<T: AssetSource + 'static> From<T> for Assets {
    fn from(source: T) -> Self {
        Self(Some(Rc::new(Registry {
            source: Box::new(source),
            trees: Default::default(),
        })))
    }
}

impl Assets {
    pub(crate) fn svg(&self, name: &SharedString) -> Result<SvgAsset> {
        let registry = self
            .0
            .as_ref()
            .with_context(|| format!("unknown SVG {name}"))?;
        if let Some(tree) = registry.trees.borrow().get(name).and_then(Weak::upgrade) {
            return Ok(SvgAsset(tree));
        }
        let bytes = registry
            .source
            .svg(name)
            .with_context(|| format!("loading SVG {name}"))?
            .with_context(|| format!("unknown SVG {name}"))?;
        let asset = SvgAsset::from_bytes(&bytes).with_context(|| format!("parsing SVG {name}"))?;
        let mut trees = registry.trees.borrow_mut();
        if trees.len() == trees.capacity() {
            trees.retain(|_, t| t.strong_count() > 0);
        }
        trees.insert(name.clone(), Rc::downgrade(asset.tree()));
        Ok(asset)
    }

    pub(crate) fn image(&self, name: &str) -> Result<ImageResource<'_>> {
        let registry = self
            .0
            .as_ref()
            .with_context(|| format!("unknown image {name}"))?;
        registry
            .source
            .image(name)
            .with_context(|| format!("loading image {name}"))?
            .with_context(|| format!("unknown image {name}"))
    }
}

#[derive(Default)]
pub(crate) struct Resources {
    pub(crate) named: Assets,
    #[cfg(feature = "network")]
    pub(crate) remote: crate::bitmap::remote::RemoteImages,
}

impl Ui {
    /// Registers a resource pack for this UI. Its contents are requested on first use.
    pub fn with_assets(mut self, source: impl Into<Assets>) -> Self {
        self.assets.named = source.into();
        self
    }
}

macro_rules! name_inputs {
    ($ty:ident) => {
        impl From<&str> for $ty {
            fn from(name: &str) -> Self {
                Self::Name(name.into())
            }
        }
        impl From<String> for $ty {
            fn from(name: String) -> Self {
                Self::Name(name.into())
            }
        }
        impl From<crate::SharedString> for $ty {
            fn from(name: crate::SharedString) -> Self {
                Self::Name(name)
            }
        }
    };
}
pub(crate) use name_inputs;

macro_rules! source_inputs {
    ($ty:ident, $path:expr) => {
        impl From<std::path::PathBuf> for $ty {
            fn from(path: std::path::PathBuf) -> Self {
                ($path)(path)
            }
        }
        impl From<&std::path::Path> for $ty {
            fn from(path: &std::path::Path) -> Self {
                path.to_owned().into()
            }
        }
        impl From<Vec<u8>> for $ty {
            fn from(bytes: Vec<u8>) -> Self {
                Self::Bytes(bytes.into())
            }
        }
        impl From<std::borrow::Cow<'static, [u8]>> for $ty {
            fn from(bytes: std::borrow::Cow<'static, [u8]>) -> Self {
                Self::Bytes(bytes)
            }
        }
        impl From<&[u8]> for $ty {
            fn from(bytes: &[u8]) -> Self {
                Self::Bytes(bytes.to_vec().into())
            }
        }
        impl<const N: usize> From<&'static [u8; N]> for $ty {
            fn from(bytes: &'static [u8; N]) -> Self {
                Self::Bytes(std::borrow::Cow::Borrowed(bytes))
            }
        }
    };
}
pub(crate) use source_inputs;
