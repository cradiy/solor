//! Fixed and signal-driven property declarations.
use std::{any::Any, fmt, rc::Rc};

use super::*;
use crate::{
    FontFamily, Result,
    state::{Runtime, SignalId},
};

type Projection<T> = Rc<dyn Fn(&dyn Any) -> T>;

#[derive(Clone)]
enum Reader<T> {
    Convert(fn(&dyn Any) -> T),
    Map(Projection<T>),
}

impl<T> Reader<T> {
    fn get(&self, value: &dyn Any) -> T {
        match self {
            Self::Convert(convert) => convert(value),
            Self::Map(map) => map(value),
        }
    }
}

/// A property supplied by value or sampled from a typed signal.
/// Style setters accept both forms directly.
#[derive(Clone)]
pub struct Property<T>(Source<T>);

#[derive(Clone)]
enum Source<T> {
    Fixed(T),
    Signal(SignalId, Reader<T>),
}

impl<T: fmt::Debug> fmt::Debug for Property<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Source::Fixed(value) => value.fmt(f),
            Source::Signal(id, _) => id.fmt(f),
        }
    }
}

impl<T: PartialEq> PartialEq for Property<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Source::Fixed(a), Source::Fixed(b)) => a == b,
            // The signal fixes the input type and T fixes the conversion.
            (Source::Signal(a, Reader::Convert(_)), Source::Signal(b, Reader::Convert(_))) => {
                a == b
            }
            (Source::Signal(a, Reader::Map(f)), Source::Signal(b, Reader::Map(g))) => {
                a == b && Rc::ptr_eq(f, g)
            }
            _ => false,
        }
    }
}

macro_rules! inputs {
    ($($ty:ty),+ $(,)?) => {$(
        impl<S: Clone + Into<$ty> + 'static> From<Signal<S>> for Property<$ty> {
            fn from(source: Signal<S>) -> Self {
                Self(Source::Signal(source.id(), Reader::Convert(|value| {
                    value.downcast_ref::<S>().expect("property signal type").clone().into()
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
    UnderlineStyle,
    Visibility,
    crate::GradientMap,
    Length,
    AbsoluteLength,
    Fill,
    crate::Ink,
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

#[cfg(feature = "shader")]
exact!([f32; 4]);

impl<T: Into<Background>> From<T> for Property<Background> {
    fn from(source: T) -> Self {
        Self(Source::Fixed(source.into()))
    }
}

impl<P: Into<Property<f32>>> From<crate::GradientColor<P>> for Property<Background> {
    fn from(color: crate::GradientColor<P>) -> Self {
        let palette = color.palette;
        color.position.into().map(move |position| {
            Background::GradientColor(crate::GradientColor {
                palette: palette.clone(),
                position,
            })
        })
    }
}

impl From<Property<Rgba>> for Background {
    fn from(color: Property<Rgba>) -> Self {
        match color.0 {
            Source::Fixed(value) => value.into(),
            Source::Signal(..) => Self::Color(color),
        }
    }
}

impl From<Property<Rgba>> for Foreground {
    fn from(color: Property<Rgba>) -> Self {
        match color.0 {
            Source::Fixed(value) => value.into(),
            Source::Signal(..) => Self::Bound(color),
        }
    }
}

impl From<Property<Rgba>> for Property<crate::Ink> {
    fn from(color: Property<Rgba>) -> Self {
        color.map(crate::Ink::from)
    }
}

impl From<Property<AbsoluteLength>> for Property<Length> {
    fn from(length: Property<AbsoluteLength>) -> Self {
        length.map(Length::Absolute)
    }
}

impl From<std::num::NonZeroUsize> for Property<Option<std::num::NonZeroUsize>> {
    fn from(lines: std::num::NonZeroUsize) -> Self {
        Self(Source::Fixed(Some(lines)))
    }
}

impl<T: 'static> Property<T> {
    pub(crate) fn source_id(&self) -> Option<SignalId> {
        match self.0 {
            Source::Fixed(_) => None,
            Source::Signal(id, _) => Some(id),
        }
    }

    pub(crate) fn is_signal(&self) -> bool {
        self.source_id().is_some()
    }

    pub(crate) fn read(&self, state: &Runtime<()>) -> Result<T>
    where
        T: Clone,
    {
        match &self.0 {
            Source::Fixed(value) => Ok(value.clone()),
            Source::Signal(id, read) => Ok(read.get(state.signal_value(*id)?)),
        }
    }

    pub(crate) fn observe_value(
        self,
        state: &mut Runtime<()>,
        node: crate::NodeId,
        mut changed: impl FnMut(T) + 'static,
    ) -> Result<()> {
        if let Source::Signal(id, reader) = self.0 {
            state.register_binding(
                id,
                node,
                true,
                Box::new(move |value, _| changed(reader.get(value))),
            )?;
        }
        Ok(())
    }

    pub(crate) fn map<U: 'static>(self, convert: impl Fn(T) -> U + 'static) -> Property<U> {
        Property(match self.0 {
            Source::Fixed(value) => Source::Fixed(convert(value)),
            Source::Signal(id, read) => Source::Signal(
                id,
                Reader::Map(Rc::new(move |value| convert(read.get(value)))),
            ),
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
                        set(style, read.get(state.signal_value(source)?));
                        Ok(())
                    }),
                };
                Rc::make_mut(style.bindings.0.get_or_insert_with(Default::default)).push(entry);
            }
        }
    }
}

impl StyleRefinement {
    pub(crate) fn bind_transition_background(&mut self, base: &Style) {
        let has_color_rule = |rules: &Rc<TransitionRules>| {
            rules.contains_key(&crate::TransitionProperty::BackgroundColor)
        };
        if !self.transitions.as_ref().is_some_and(has_color_rule)
            && !base.transitions.as_ref().is_some_and(has_color_rule)
        {
            return;
        }
        if self.background.is_none() && self.bindings.contains("background") {
            return;
        }
        let source = match self.background.as_ref().unwrap_or(&base.background) {
            Some(Background::Color(color)) => color.clone().map(Background::from),
            Some(Background::Bound(fill)) => Property::<Fill>::from(*fill).map(Background::Fixed),
            _ => return,
        };
        // Subscribe at style resolution so a new target wakes an idle transition.
        self.set_background(source);
    }

    pub(super) fn set_background(&mut self, source: Property<Background>) {
        source.declare(
            self,
            Field {
                key: "background",
                inherited: false,
                layout: false,
                specified: |s| s.background.is_some(),
            },
            |s, fill| s.background = Some(Some(fill)),
            |s| s.background = None,
        );
    }
}
