//! Fixed and signal-driven property declarations.
use std::{fmt, rc::Rc};

use super::*;
use crate::{
    FontFamily, Result,
    state::{Runtime, SignalId},
};

type Reader<T> = Rc<dyn Fn(&Runtime<()>) -> Result<T>>;

/// A property supplied by value or sampled from a typed signal.
/// Style setters accept both forms directly.
#[derive(Clone)]
pub struct Property<T>(Source<T>);

#[derive(Clone)]
enum Source<T> {
    Fixed(T),
    Signal(SignalId, Reader<T>),
}

macro_rules! inputs {
    ($($ty:ty),+ $(,)?) => {$(
        impl<S: Clone + Into<$ty> + 'static> From<Signal<S>> for Property<$ty> {
            fn from(source: Signal<S>) -> Self {
                Self(Source::Signal(source.id(), Rc::new(move |state| {
                    Ok(state.read(source)?.clone().into())
                })))
            }
        }
    )+};
}
macro_rules! converted {
    ($($ty:ty),+ $(,)?) => {
        inputs!($($ty),+);
        $(impl<V: Into<$ty>> From<V> for Property<$ty> {
            fn from(value: V) -> Self { Self(Source::Fixed(value.into())) }
        })+
    };
}
macro_rules! exact {
    ($($ty:ty),+ $(,)?) => {
        inputs!($($ty),+);
        $(impl From<$ty> for Property<$ty> {
            fn from(value: $ty) -> Self { Self(Source::Fixed(value)) }
        })+
    };
}

converted!(
    Visibility,
    crate::GradientMap,
    Length,
    AbsoluteLength,
    Fill,
    Rgba,
    FontFamily,
    Rc<[FontFamily]>,
    Rc<[FontFeature]>,
    Rc<[BoxShadow]>,
    Rc<[Transform]>,
    Rc<[GridTrack]>
);
exact!(
    Display,
    Position,
    BoxSizing,
    ObjectFit,
    Overflow,
    f32,
    i32,
    FlexDirection,
    FlexWrap,
    Align,
    Distribution,
    GridAutoFlow,
    GridLine,
    Cursor,
    PointerEvents,
    BorderStyle,
    u16,
    FontStyle,
    TextAlign,
    WhiteSpace,
    TextOverflow,
    bool,
    Option<std::num::NonZeroUsize>
);

impl From<std::num::NonZeroUsize> for Property<Option<std::num::NonZeroUsize>> {
    fn from(lines: std::num::NonZeroUsize) -> Self {
        Self(Source::Fixed(Some(lines)))
    }
}

impl<T: 'static> Property<T> {
    pub(super) fn map<U: 'static>(self, convert: fn(T) -> U) -> Property<U> {
        Property(match self.0 {
            Source::Fixed(value) => Source::Fixed(convert(value)),
            Source::Signal(id, read) => {
                Source::Signal(id, Rc::new(move |state| Ok(convert(read(state)?))))
            }
        })
    }
}

type Apply = Rc<dyn Fn(&Runtime<()>, &mut StyleRefinement) -> Result<()>>;

#[derive(Clone)]
pub(super) struct Field {
    pub key: &'static str,
    pub inherited: bool,
    pub layout: bool,
    pub specified: fn(&StyleRefinement) -> bool,
}

/// Sparse signal sources. Fixed declarations store their values in the regular style fields.
#[derive(Clone, Default)]
pub struct Bindings(Option<Rc<Vec<Binding>>>);

#[derive(Clone)]
struct Binding {
    field: Field,
    source: SignalId,
    apply: Apply,
    clear: fn(&mut StyleRefinement),
}

impl fmt::Debug for Bindings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries().iter().map(|b| (b.field.key, b.source)))
            .finish()
    }
}

impl Bindings {
    fn entries(&self) -> &[Binding] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }

    pub(crate) fn sources(&self) -> impl Iterator<Item = (SignalId, bool)> + '_ {
        self.entries().iter().map(|b| (b.source, b.field.layout))
    }

    pub(crate) fn contains(&self, key: &str) -> bool {
        self.entries().iter().any(|b| b.field.key == key)
    }

    pub(crate) fn without_fixed_overrides(&self, style: &StyleRefinement) -> Self {
        let mut active = self.clone();
        active.retain(|b| !(b.field.specified)(style));
        active
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    fn retain(&mut self, keep: impl Fn(&Binding) -> bool) {
        if let Some(list) = &mut self.0
            && list.iter().any(|b| !keep(b))
        {
            Rc::make_mut(list).retain(keep);
            if list.is_empty() {
                self.0 = None;
            }
        }
    }

    pub(crate) fn inherited(&self) -> Self {
        let mut result = self.clone();
        result.retain(|b| b.field.inherited);
        result
    }

    pub(crate) fn unshadowed(&self, later: &StyleRefinement) -> Self {
        let mut result = self.clone();
        result.retain(|b| {
            !(b.field.specified)(later)
                && !later
                    .bindings
                    .entries()
                    .iter()
                    .any(|v| v.field.key == b.field.key)
        });
        result
    }

    pub(crate) fn extend(&mut self, later: &Self) {
        if self.is_empty() {
            *self = later.clone();
        } else if let Some(entries) = &mut self.0
            && !later.is_empty()
        {
            Rc::make_mut(entries).extend_from_slice(later.entries());
        }
    }

    pub(crate) fn clear_fixed(&self, style: &mut StyleRefinement) {
        for b in self.entries() {
            (b.clear)(style);
        }
    }

    pub(crate) fn resolve(&self, state: &Runtime<()>, style: &mut StyleRefinement) -> Result<()> {
        for b in self.entries() {
            (b.apply)(state, style)?;
        }
        Ok(())
    }
}

impl<T: 'static> Property<T> {
    pub(super) fn declare(
        self,
        style: &mut StyleRefinement,
        field: Field,
        set: fn(&mut StyleRefinement, T),
        clear: fn(&mut StyleRefinement),
    ) {
        style.bindings.retain(|b| b.field.key != field.key);
        match self.0 {
            Source::Fixed(value) => set(style, value),
            Source::Signal(source, read) => {
                clear(style);
                let entry = Binding {
                    field,
                    source,
                    clear,
                    apply: Rc::new(move |state, style| {
                        set(style, read(state)?);
                        Ok(())
                    }),
                };
                Rc::make_mut(style.bindings.0.get_or_insert_with(Default::default)).push(entry);
            }
        }
    }
}
