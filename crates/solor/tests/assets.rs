//! Resource pack composition and lookup through mounted elements.
use anyhow::Result;
use solor::{AssetSource, Assets, Ui, component, div, svg};
use std::{borrow::Cow, cell::Cell, rc::Rc};

const MARK: &[u8] =
    br#"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><path d="M0 0h3v2Z"/></svg>"#;

struct Library(Rc<Cell<usize>>);

impl AssetSource for Library {
    fn svg(&self, _: &str) -> Result<Option<Cow<'_, [u8]>>> {
        self.0.set(self.0.get() + 1);
        Ok(Some(MARK.into()))
    }
}

struct Theme;
impl AssetSource for Theme {
    fn svg(&self, name: &str) -> Result<Option<Cow<'_, [u8]>>> {
        match name {
            "theme/mark" => Ok(Some(MARK.into())),
            "broken" => anyhow::bail!("theme unavailable"),
            _ => Ok(None),
        }
    }
}

#[test]
fn shared_windows_resolve_live_icons_once_and_respect_pack_priority() -> Result<()> {
    let calls = Rc::new(Cell::new(0));
    let assets = Assets::from((Theme, Library(calls.clone())));
    let mut first = Ui::new().with_assets(assets.clone());
    let mut second = Ui::new().with_assets(assets);
    assert_eq!(calls.get(), 0);
    let left = first.mount(component(|_| {
        div().child(svg("theme/mark")).child(svg("/mark"))
    }))?;
    let right = second.mount(component(|_| svg("/mark")))?;
    assert_eq!(calls.get(), 1);
    let failure = second.mount(component(|_| svg("broken"))).unwrap_err();
    assert!(format!("{failure:#}").contains("theme unavailable"));
    assert_eq!(calls.get(), 1);
    first.unmount(left)?;
    second.unmount(right)?;
    second.mount(component(|_| svg("/mark")))?;
    assert_eq!(calls.get(), 2);
    Ok(())
}

#[cfg(feature = "png")]
#[test]
fn named_file_and_encoded_sources_render_the_same_pixels() -> Result<()> {
    use image::{ExtendedColorType, ImageEncoder, codecs::png::PngEncoder};
    use solor::{Draw, Fill, ImageCache, ImageResource, Styled, image, layout::Size, px};
    use std::{fs, path::PathBuf};

    struct Photos {
        path: PathBuf,
        encoded: Vec<u8>,
    }
    impl AssetSource for Photos {
        fn image(&self, name: &str) -> Result<Option<ImageResource<'_>>> {
            Ok(match name {
                "cover" => Some(ImageResource::Path(self.path.as_path().into())),
                "/embedded" => Some(ImageResource::Bytes(self.encoded.as_slice().into())),
                _ => None,
            })
        }
    }
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded).write_image(
        &[90, 60, 30, 255],
        1,
        1,
        ExtendedColorType::Rgba8,
    )?;
    let path = std::env::temp_dir().join(format!("solor-assets-test-{}.png", std::process::id()));
    let mut file = fs::File::create_new(&path)?;
    std::io::Write::write_all(&mut file, &encoded)?;
    drop(file);
    let mut ui = Ui::new().with_assets(Photos {
        path: path.clone(),
        encoded: encoded.clone(),
    });
    let file_path = path.clone();
    let view = ui.mount(component(move |_| {
        div()
            .items_start()
            .child(image("cover"))
            .child(image("/embedded"))
            .child(image(file_path.as_path()))
            .child(image(encoded))
    }))?;
    ui.layout(
        view,
        Size {
            width: px(10.),
            height: px(10.),
        },
    )?;
    let frame = ui.record_frame(view)?;
    let mut cache = ImageCache::default();
    let mut files = 0;
    let mut painted = 0;
    for draw in frame.draws() {
        if let Draw::Fill { fill, .. } = draw {
            files += usize::from(matches!(fill, Fill::ImageFile(_)));
            if let Fill::Image(bitmap) = cache.resolve(fill)?.as_ref() {
                assert_eq!(bitmap.asset().pixels(), &[[90, 60, 30, 255]]);
                painted += 1;
            }
        }
    }
    fs::remove_file(path)?;
    assert_eq!((files, painted), (2, 4));
    Ok(())
}

#[cfg(feature = "png")]
#[test]
fn oversized_file_background_defers_io_until_resolution() -> Result<()> {
    use solor::{Fill, ImageCache, ImageFile};
    use std::fs;
    let path = std::env::temp_dir().join(format!("solor-sparse-image-{}.png", std::process::id()));
    let fill = Fill::from(ImageFile::new(path.clone()));
    let file = fs::File::create_new(&path)?;
    file.set_len(3_u64 << 30)?;
    let err = ImageCache::default().resolve(&fill).unwrap_err();
    fs::remove_file(path)?;
    assert!(format!("{err:#}").contains("encoded image exceeds"));
    Ok(())
}
