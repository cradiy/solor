//! Resolves local and inherited style inputs for retained element bodies.
use super::*;

pub(super) struct PreparedStyle {
    pub background: Option<Box<MountedBackground>>,
    pub style: Style,
    pub text: InheritedText,
    pub fixed: bool,
    pub visible: bool,
}

pub(super) fn prepare<E: Element>(
    node: &mut Node<E>,
    inherited: &InheritedText,
    cx: &mut Context<'_>,
    units: UnitResolver,
) -> Result<PreparedStyle> {
    let base = node.body.base_style();
    node.style.bindings = node.style.bindings.without_fixed_overrides(&node.style);
    let mut inherited_bindings = inherited.bindings.unshadowed(&node.style);
    let mut child_bindings = inherited_bindings.clone();
    child_bindings.extend(&node.style.bindings.inherited());

    // An implicit line height also depends on the parent size, even with a local size.
    if inherited.fixed.line_height.is_none()
        && node.style.text.line_height.is_none()
        && !inherited.bindings.contains("text.line_height")
        && !node.style.bindings.contains("text.line_height")
    {
        inherited_bindings.include_parent_size(&inherited.bindings);
    }
    let mut sources: Vec<_> = inherited_bindings
        .sources()
        .chain(node.style.bindings.sources())
        .collect();
    let fixed = sources.is_empty();
    let bound_visibility = node.style.bindings.contains("appearance_mut.visibility");
    let (style, fixed_text) = if fixed {
        let mut style = base;
        let fixed = style.refine(&node.style, &inherited.fixed, units);
        (style, fixed)
    } else {
        let fixed = inherited.fixed.overlay(&node.style.text);
        let recipe = StyleRecipe {
            base,
            local: std::mem::take(&mut node.style),
            inherited: InheritedText {
                fixed: inherited.fixed.clone(),
                bindings: inherited_bindings,
            },
        };
        let style = recipe.resolve(cx.state, units)?;
        let initial = style.appearance.as_deref().and_then(|a| a.visibility);
        node.watches.push(Box::new(move |cx, target, id, path, _| {
            let key = (ViewId(cx.scope), id);
            let visibility = bound_visibility.then(|| VisibilityObserver {
                key,
                target,
                path: path.into(),
                updates: cx.visibility.clone(),
                nested: Default::default(),
            });
            if let Some(observer) = &visibility {
                observer.set(cx.state, initial)?;
            }
            let apply: crate::Restyle = Rc::new(move |instance, id, state, layout| {
                let style = recipe.resolve(state, instance.units)?;
                let value = style.appearance.as_deref().and_then(|a| a.visibility);
                instance.update_style::<E>(id, style, layout)?;
                if let Some(observer) = &visibility {
                    observer.set(state, value)?;
                }
                Ok(())
            });
            while let Some((signal, mut layout)) = sources.pop() {
                sources.retain(|&(id, needs_layout)| {
                    if id == signal {
                        layout |= needs_layout;
                    }
                    id != signal
                });
                let updates = cx.styles.clone();
                let apply = apply.clone();
                cx.state.observe(signal, target, move || {
                    updates
                        .borrow_mut()
                        .entry(key)
                        .and_modify(|(_, any_layout)| *any_layout |= layout)
                        .or_insert_with(|| (apply.clone(), layout));
                })?;
            }
            Ok(())
        }));
        (style, fixed)
    };
    let text = InheritedText {
        fixed: fixed_text,
        bindings: child_bindings,
    };

    let local_visible = match style.appearance.as_deref().and_then(|a| a.visibility) {
        Some(Visibility::Hidden) => false,
        Some(Visibility::When(source)) => {
            let initial = *cx.read(source)?;
            if !bound_visibility {
                node.watches.push(Box::new(move |cx, target, id, path, _| {
                    let updates = Rc::clone(cx.visibility);
                    let view = ViewId(cx.scope);
                    let path: Rc<[usize]> = path.into();
                    cx.state.bind(source, target, move |visible, _| {
                        updates
                            .borrow_mut()
                            .entry(view)
                            .or_default()
                            .insert(Rc::clone(&path), (id, *visible));
                    })?;
                    Ok(())
                }));
            }
            initial
        }
        _ => true,
    };
    Ok(PreparedStyle {
        background: MountedBackground::resolve(&style, units)?,
        style,
        text,
        fixed,
        visible: local_visible,
    })
}

struct VisibilityObserver {
    key: (ViewId, NodeId),
    target: NodeId,
    path: Rc<[usize]>,
    updates: crate::VisibilityUpdates,
    nested: std::cell::Cell<Option<(Signal<bool>, crate::state::BindingId)>>,
}

impl VisibilityObserver {
    fn set(&self, state: &mut Runtime<()>, value: Option<Visibility>) -> Result<()> {
        let source = match value {
            Some(Visibility::When(source)) => Some(source),
            _ => None,
        };
        let visible = match source {
            Some(signal) => *state.read(signal)?,
            None => value != Some(Visibility::Hidden),
        };
        let (view, id) = self.key;
        if self.nested.get().map(|(signal, _)| signal) != source {
            let next = if let Some(signal) = source {
                let path = self.path.clone();
                let updates = self.updates.clone();
                let binding = state.bind(signal, self.target, move |visible, _| {
                    updates
                        .borrow_mut()
                        .entry(view)
                        .or_default()
                        .insert(path.clone(), (id, *visible));
                })?;
                Some((signal, binding))
            } else {
                None
            };
            if let Some((_, binding)) = self.nested.replace(next) {
                let _ = state.unbind(binding);
            }
        }
        self.updates
            .borrow_mut()
            .entry(view)
            .or_default()
            .insert(self.path.clone(), (id, visible));
        Ok(())
    }
}

struct StyleRecipe {
    base: Style,
    local: StyleRefinement,
    inherited: InheritedText,
}

impl StyleRecipe {
    fn resolve(&self, state: &Runtime<()>, units: UnitResolver) -> Result<Style> {
        let mut inherited = StyleRefinement {
            text: self.inherited.fixed.clone(),
            ..Default::default()
        };
        self.inherited.bindings.resolve(state, &mut inherited)?;
        let mut local = self.local.clone();
        self.local.bindings.resolve(state, &mut local)?;
        let mut style = self.base.clone();
        style.refine(&local, &inherited.text, units);
        Ok(style)
    }
}

pub(super) fn item_style(style: &Style) -> Result<crate::layout::FlexItemStyle> {
    if !style.text.letter_spacing.get().is_finite()
        || [
            style.corner_radii.top_left,
            style.corner_radii.top_right,
            style.corner_radii.bottom_left,
            style.corner_radii.bottom_right,
            style.column_gap,
            style.row_gap,
            style.text.size,
            style.text.line_height,
        ]
        .iter()
        .any(|v| !v.is_extent())
    {
        return Err(crate::layout::LayoutError::InvalidSize.into());
    }
    let grow = style
        .layout
        .as_ref()
        .and_then(|layout| layout.flex_grow)
        .unwrap_or_default();
    let shrink = style
        .layout
        .as_ref()
        .and_then(|layout| layout.flex_shrink)
        .unwrap_or_default();
    if [grow, shrink].iter().any(|f| !f.is_finite() || *f < 0.0) {
        return Err(crate::layout::LayoutError::InvalidSize.into());
    }
    let align_self = style
        .layout
        .as_ref()
        .and_then(|layout| layout.align_self)
        .unwrap_or(Align::Auto);
    Ok(crate::layout::FlexItemStyle {
        overflow_clip: OverflowClip::from_layout(style.layout.as_deref()),
        grow,
        shrink,
        align_self,
        auto_width: style.width == crate::Length::Auto,
        auto_height: style.height == crate::Length::Auto,
    })
}

impl crate::Instance {
    fn update_style<E: Element>(&mut self, id: NodeId, style: Style, layout: bool) -> Result<()> {
        let item = item_style(&style)?;
        let background = MountedBackground::resolve(&style, self.units)?;
        // Custom content can use any declared property to determine its metrics.
        let builtin = std::any::TypeId::of::<E>() == std::any::TypeId::of::<FlexLayout>()
            || std::any::TypeId::of::<E>() == std::any::TypeId::of::<Text>()
            || std::any::TypeId::of::<E>() == std::any::TypeId::of::<super::image::ImageBody>();
        let node = self.bodies.get_mut(&id).expect("mounted style target");
        let clip = item.overflow_clip;
        if layout || !builtin || clip != node.overflow_clip {
            let bounds = crate::styled_body::BoxModel::resolve(&style, self.units)?;
            let sizing = bounds.item_sizing(&style, self.units)?;
            let padding = bounds.content_insets();
            let mut prepared = Ok(());
            self.scene.edit_layout::<StyledBody<E>>(id, |body| {
                prepared = body.content.prepare_content(&style);
                if prepared.is_ok() {
                    body.bounds = bounds;
                }
            })?;
            prepared?;
            self.scene.set_item_style(id, item, sizing)?;
            if let Some(handlers) = node.handlers.as_mut()
                && handlers.padding != padding
            {
                handlers.padding = padding;
                self.pointer.invalidate_geometry();
            }
        } else {
            self.scene
                .body_mut::<StyledBody<E>>(id)?
                .content
                .prepare_content(&style)?;
        }
        let appearance = style.appearance.as_deref();
        let z_index = appearance.and_then(|a| a.z_index).unwrap_or_default();
        let cursor = appearance.and_then(|a| a.cursor);
        let hit_testable = appearance.and_then(|a| a.pointer_events) != Some(PointerEvents::None);
        let scroll_axes = crate::pointer::ScrollAxes::from_layout(style.layout.as_deref());
        let routing_changed = (z_index, &clip, scroll_axes, cursor, hit_testable)
            != (
                node.z_index,
                &node.overflow_clip,
                node.scroll_axes,
                node.cursor,
                node.hit_testable,
            );
        if scroll_axes != node.scroll_axes
            && let Some(offset) = self.pointer.scroll_offsets.get_mut(&id)
            && let Some(geometry) = self.scene.geometry(id)
            && !scroll_axes.clamp_offset(offset, geometry)
        {
            self.pointer.scroll_offsets.remove(&id);
        }
        node.scroll_axes = scroll_axes;
        node.z_index = z_index;
        node.overflow_clip = clip;
        node.cursor = cursor;
        node.hit_testable = hit_testable;
        node.text_color = style.text.color;
        node.background = background;
        if routing_changed {
            self.invalidate_geometry();
        }
        self.repaints.borrow_mut().invalidate(id);
        Ok(())
    }
}
