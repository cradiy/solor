//! Composition, conversion at attachment, and retained element behavior.
use anyhow::Context as _;
use std::{cell::RefCell, rc::Rc};

use crate::layout::{LayoutNode, Origin, Query, Scene, Size};
use crate::state::{Runtime, Signal, SignalId};
use crate::tree::NodeId;

use super::{Attachment, Context, Repaints, Result, SharedString, ViewId};
use crate::layout::{FlexDirection, FlexLayout};
use crate::length::UnitResolver;
use crate::pointer::Handlers;
use crate::style::{Align, Background, Corners, TextStyleRefinement};
use crate::style::{PointerEvents, Visibility};
use crate::text::{Paragraph, Text, TextStyle};
use crate::{Fill, Pixels, Rgba, Style, StyleRefinement, Styled, StyledBody, px};

/// Builds an element declaration when a component is attached.
pub trait Render {
    /// Consumes the component and declares its retained element tree.
    fn render(self, cx: &mut Context<'_>) -> impl IntoElement;
}

/// Converts a component or primitive into a single-use declaration.
pub trait IntoElement {
    /// Erases the declaration's concrete type for heterogeneous composition.
    fn into_element(self) -> AnyElement;
}

impl<R: Render + 'static> IntoElement for R {
    fn into_element(self) -> AnyElement {
        AnyElement(Box::new(move |target, cx, units| {
            let item = self.render(cx).into_element();
            (item.0)(target, cx, units)
        }))
    }
}

/// Layout and local drawing behavior of a retained primitive.
pub trait Element: LayoutNode {
    /// Intrinsic defaults used before inherited typography and local refinements.
    fn base_style(&self) -> Style {
        Style::default()
    }

    /// Content-specific inputs such as font metrics. The framework handles the box and paint.
    fn prepare_content(&mut self, _style: &Style) -> Result<()> {
        Ok(())
    }

    /// Records this body's content after its background.
    fn paint(&self, _cx: &mut PaintContext) -> Result<()> {
        Ok(())
    }
}

impl Element for FlexLayout {
    fn base_style(&self) -> Style {
        Style {
            row_gap: self.gap,
            column_gap: self.gap,
            ..Style::default()
        }
    }

    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        if let Some(layout) = &style.layout {
            self.direction = layout.flex_direction.unwrap_or(self.direction);
            self.justify_content = layout.justify_content.unwrap_or(self.justify_content);
            self.align_items = layout.align_items.unwrap_or(self.align_items);
        }
        self.gap = match self.direction {
            FlexDirection::Row | FlexDirection::RowReverse => style.column_gap,
            FlexDirection::Column | FlexDirection::ColumnReverse => style.row_gap,
        };
        Ok(())
    }
}

impl Element for Text {
    fn base_style(&self) -> Style {
        let mut style = Style::default();
        style.text.size = self.style().size;
        style.text.line_height = self.style().line_height;
        style
    }
    fn prepare_content(&mut self, style: &Style) -> Result<()> {
        self.set_style(TextStyle {
            size: style.text.size,
            line_height: style.text.line_height,
        })?;
        self.set_paragraph_style(style.text.attributes.as_deref());
        Ok(())
    }
    fn paint(&self, cx: &mut PaintContext) -> Result<()> {
        cx.commands.push(Draw::Text {
            origin: cx.origin,
            size: cx.size,
            paragraph: self.resolve(Query::Exact(cx.size.width))?,
            color: cx.text_color,
        });
        Ok(())
    }
}

/// A recorded drawing operation. Origins are local to the recording target.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// Fills a rounded rectangle.
    Fill {
        /// Upper-left corner of the fill.
        origin: Origin,
        /// Extent of the rectangle.
        size: Size,
        /// Color or gradient covering the rectangle.
        fill: Fill,
        /// Circular radii in logical pixels, fitted to the fill bounds.
        corner_radii: Corners<Pixels>,
    },
    /// Draws a shaped paragraph.
    Text {
        /// Upper-left corner of the paragraph.
        origin: Origin,
        /// Assigned content box used to clip glyphs.
        size: Size,
        /// Shaped text retained for this command.
        paragraph: Rc<Paragraph>,
        /// Foreground applied to the glyphs.
        color: Rgba,
    },
}

/// Local content bounds, inherited color and state access during recording.
pub struct PaintContext<'a> {
    origin: Origin,
    /// Assigned content area after removing padding.
    pub size: Size,
    /// Resolved foreground color for this element.
    pub text_color: Rgba,
    commands: &'a mut Vec<Draw>,
    next_frame: bool,
    state: &'a Runtime<()>,
    reads: &'a RefCell<Vec<SignalId>>,
}

impl PaintContext<'_> {
    /// Samples a signal and subscribes this element to changes after successful paint.
    pub fn read<T: 'static>(&self, signal: Signal<T>) -> Result<&T> {
        let value = self.state.read(signal)?;
        let mut reads = self.reads.borrow_mut();
        if !reads.contains(&signal.id()) {
            reads.push(signal.id());
        }
        Ok(value)
    }

    /// Requests another host-paced frame after this view frame is submitted.
    /// Call on each paint while animation is active. Requests from siblings coalesce.
    pub fn request_animation_frame(&mut self) {
        self.next_frame = true;
    }

    /// Records a rectangle at the content origin.
    pub fn fill(&mut self, size: Size, fill: impl Into<Fill>) {
        self.commands.push(Draw::Fill {
            origin: self.origin,
            size,
            fill: fill.into(),
            corner_radii: Corners::default(),
        });
    }
}

pub(super) type Click = Box<dyn Fn(&mut Context<'_>) -> Result<()>>;

struct MountedBackground {
    source: Background,
    corner_radii: Corners<Pixels>,
}

/// When an element records new drawing commands.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RefreshMode {
    /// Reuse commands until a paint dependency or layout changes.
    #[default]
    OnDemand,
    /// Sample on every host frame, including frames with identical inputs.
    Continuous,
}

/// A fixed refresh mode or a runtime-selected mode.
pub enum RefreshPolicy {
    /// Use this mode throughout the element's attachment.
    Fixed(RefreshMode),
    /// Follow the current value of the signal.
    Bound(Signal<RefreshMode>),
}

impl From<RefreshMode> for RefreshPolicy {
    fn from(mode: RefreshMode) -> Self {
        Self::Fixed(mode)
    }
}

impl From<Signal<RefreshMode>> for RefreshPolicy {
    fn from(mode: Signal<RefreshMode>) -> Self {
        Self::Bound(mode)
    }
}

pub(super) struct MountedNode {
    pub(super) id: NodeId,
    pub(super) offset: usize,
    pub(super) fixed_label: bool,
    pub(super) visible: bool,
    pub(super) local_visible: bool,
    paint_mode: RefreshPolicy,
    pub(super) text_style: TextStyleRefinement,
    text_color: Rgba,
    background: Option<Box<MountedBackground>>,
    pub(super) binding_target: Option<NodeId>,
    record: fn(&Scene, NodeId, &mut PaintContext) -> Result<()>,
    pub(super) owned: Vec<SignalId>,
    pub(crate) handlers: Option<Box<Handlers>>,
    pub(super) z_index: i32,
    pub(crate) hit_testable: bool,
}

impl MountedNode {
    fn new<E: Element>(
        id: NodeId,
        handlers: Option<Box<Handlers>>,
        text_style: TextStyleRefinement,
        style: Style,
        fixed_label: bool,
        paint_mode: RefreshPolicy,
        local_visible: bool,
    ) -> Self {
        Self {
            id,
            offset: 0,
            local_visible,
            visible: local_visible,
            fixed_label: fixed_label
                && style.background.is_none()
                && matches!(paint_mode, RefreshPolicy::Fixed(RefreshMode::OnDemand)),
            paint_mode,
            binding_target: None,
            handlers,
            z_index: style
                .appearance
                .as_deref()
                .and_then(|a| a.z_index)
                .unwrap_or_default(),
            hit_testable: style.appearance.as_deref().and_then(|a| a.pointer_events)
                != Some(PointerEvents::None),
            text_style,
            text_color: style.text.color,
            background: style.background.map(|source| {
                Box::new(MountedBackground {
                    source,
                    corner_radii: style.corner_radii,
                })
            }),
            owned: Vec::new(),
            record: |scene, id, cx| {
                let body = scene.body::<StyledBody<E>>(id)?;
                cx.size = body.content_size(cx.size);
                cx.origin = body.origin();
                body.content.paint(cx)
            },
        }
    }

    pub(super) fn paint(
        &self,
        scene: &Scene,
        state: &Runtime<()>,
        reads: &RefCell<Vec<SignalId>>,
        commands: &mut Vec<Draw>,
    ) -> Result<bool> {
        if scene.is_pending() {
            anyhow::bail!("layout pending");
        }
        reads.borrow_mut().clear();
        if !self.visible {
            return Ok(false);
        }
        let geometry = scene.geometry(self.id).context("missing layout geometry")?;
        let start = commands.len();
        let size = geometry.metrics.size;
        let mut cx = PaintContext {
            origin: Origin::default(),
            size,
            text_color: self.text_color,
            commands,
            next_frame: false,
            state,
            reads,
        };
        let mode = match self.paint_mode {
            RefreshPolicy::Fixed(mode) => mode,
            RefreshPolicy::Bound(source) => *cx.read(source)?,
        };
        cx.next_frame = mode == RefreshMode::Continuous;
        if let Some(paint) = &self.background {
            let fill = match &paint.source {
                Background::Fixed(fill) => fill.clone(),
                Background::Bound(signal) => cx.read(*signal)?.clone(),
            };
            cx.commands.push(Draw::Fill {
                origin: Origin::default(),
                size,
                fill,
                corner_radii: paint.corner_radii.fit_to(size),
            });
        }
        let result = (self.record)(scene, self.id, &mut cx).map(|()| cx.next_frame);
        if result.is_err() {
            commands.truncate(start);
        }
        result
    }
}

pub(super) struct Installation {
    pub node: MountedNode,
    pub children: Vec<Child>,
    pub watches: Vec<Watch>,
}

pub(super) struct PaintSubscription {
    repaints: Repaints,
    node: NodeId,
}

impl PaintSubscription {
    pub(super) fn observe(
        state: &mut Runtime<()>,
        source: SignalId,
        target: NodeId,
        node: NodeId,
        repaints: &Repaints,
    ) -> Result<crate::state::BindingId> {
        let subscription = Self {
            repaints: repaints.clone(),
            node,
        };
        Ok(state.observe(source, target, move || subscription.invalidate())?)
    }

    fn invalidate(&self) {
        self.repaints.borrow_mut().invalidate(self.node);
    }
}

impl Drop for PaintSubscription {
    fn drop(&mut self) {
        // A retired source can no longer invalidate commands through its binding.
        self.repaints.borrow_mut().commands.remove(&self.node);
    }
}

pub(super) type Watch =
    Box<dyn FnOnce(&mut Attachment<'_>, NodeId, NodeId, &[usize], Option<&Repaints>) -> Result<()>>;
type Install = Box<dyn FnOnce(Target<'_>, &mut Context<'_>, UnitResolver) -> Result<Installation>>;

enum Target<'a> {
    Root(&'a mut Option<Scene>),
    Child {
        scene: &'a mut Scene,
        parent: NodeId,
        index: usize,
        inherited: TextStyleRefinement,
    },
}

/// Opaque, single-use declaration. Only the host can attach it.
pub struct AnyElement(Install);

impl AnyElement {
    fn new<E: Element>(
        fixed_label: bool,
        make: impl FnOnce(&mut Context<'_>) -> Result<Node<E>> + 'static,
    ) -> Self {
        Self(Box::new(move |target, cx, units| {
            let mut node = make(cx)?;
            let inherited = match &target {
                Target::Root(_) => &TextStyleRefinement::default(),
                Target::Child { inherited, .. } => inherited,
            };
            let mut style = node.body.base_style();
            let text = style.refine(&node.style, inherited, units);
            if [
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
            let local_visible = match style.appearance.as_deref().and_then(|a| a.visibility) {
                Some(Visibility::Hidden) => false,
                Some(Visibility::When(source)) => {
                    let initial = *cx.read(source)?;
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
                    initial
                }
                _ => true,
            };
            node.body.prepare_content(&style)?;
            let body = StyledBody::new(node.body, &style, units)?;
            let item_sizing = body.item_sizing(&style, units)?;
            let id = match target {
                Target::Root(slot) => slot.insert(Scene::new(body)).root(),
                Target::Child {
                    scene,
                    parent,
                    index,
                    ..
                } => {
                    let align_self = style
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.align_self)
                        .unwrap_or(Align::Auto);
                    let item = crate::layout::FlexItemStyle {
                        grow,
                        shrink,
                        align_self,
                        auto_width: style.width == crate::Length::Auto,
                        auto_height: style.height == crate::Length::Auto,
                    };
                    scene.mount_entry(parent, Some(index), body, item, item_sizing)?
                }
            };
            Ok(Installation {
                node: MountedNode::new::<E>(
                    id,
                    node.handlers,
                    text,
                    style,
                    fixed_label,
                    node.paint_mode,
                    local_visible,
                ),
                children: node.children,
                watches: node.watches,
            })
        }))
    }

    pub(super) fn root(
        self,
        cx: &mut Context<'_>,
        units: UnitResolver,
    ) -> Result<(Scene, Installation)> {
        let mut scene = None;
        let installed = (self.0)(Target::Root(&mut scene), cx, units)?;
        Ok((
            scene.expect("successful root attachment creates a scene"),
            installed,
        ))
    }

    pub(super) fn attach(
        self,
        scene: &mut Scene,
        parent: NodeId,
        index: usize,
        inherited: TextStyleRefinement,
        units: UnitResolver,
        cx: &mut Context<'_>,
    ) -> Result<Installation> {
        (self.0)(
            Target::Child {
                scene,
                parent,
                index,
                inherited,
            },
            cx,
            units,
        )
    }
}

impl IntoElement for AnyElement {
    fn into_element(self) -> Self {
        self
    }
}

pub(super) type Factory = Box<dyn Fn() -> AnyElement>;

pub(super) enum Child {
    Fixed(AnyElement),
    When(Signal<bool>, Factory),
}

/// A primitive and the children, handlers and styles to attach with it.
pub struct Node<E> {
    paint_mode: RefreshPolicy,
    style: StyleRefinement,
    watches: Vec<Watch>,
    body: E,
    children: Vec<Child>,
    pub(crate) handlers: Option<Box<Handlers>>,
}

/// Wraps a custom layout body in the common element builder.
pub fn element<E: Element>(body: E) -> Node<E> {
    Node {
        paint_mode: RefreshMode::OnDemand.into(),
        style: StyleRefinement::default(),
        watches: Vec::new(),
        body,
        children: Vec::new(),
        handlers: None,
    }
}

impl<E> Styled for Node<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<E: Element> IntoElement for Node<E> {
    fn into_element(self) -> AnyElement {
        AnyElement::new(false, move |_| Ok(self))
    }
}

impl<E: Element> Node<E> {
    /// Appends a child in declaration order.
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(Child::Fixed(child.into_element()));
        self
    }

    /// Appends fixed children in iterator order at this point in the declaration.
    pub fn children<T: IntoElement>(mut self, children: impl IntoIterator<Item = T>) -> Self {
        self.children.extend(
            children
                .into_iter()
                .map(|child| Child::Fixed(child.into_element())),
        );
        self
    }

    /// Registers the handler invoked on a direct click.
    pub fn on_click(mut self, click: impl Fn(&mut Context<'_>) -> Result<()> + 'static) -> Self {
        self.handlers.get_or_insert_with(Default::default).click = Some(Box::new(click));
        self
    }

    /// Selects a fixed or signal-driven refresh mode. The default is on demand.
    pub fn refresh_mode(mut self, mode: impl Into<RefreshPolicy>) -> Self {
        self.paint_mode = mode.into();
        self
    }

    /// Mounts the generated child while its boolean signal is true.
    pub fn child_when<T: IntoElement>(
        mut self,
        open: Signal<bool>,
        make: impl Fn() -> T + 'static,
    ) -> Self {
        self.children
            .push(Child::When(open, Box::new(move || make().into_element())));
        self
    }
}

/// A fixed label or a retained label bound to application state.
pub enum TextContent {
    /// Text supplied at attachment.
    Fixed(SharedString),
    /// Text updated from the signal at flush.
    Bound(Signal<SharedString>),
}

impl<T: Into<SharedString>> From<T> for TextContent {
    fn from(value: T) -> Self {
        Self::Fixed(value.into())
    }
}

impl From<Signal<SharedString>> for TextContent {
    fn from(value: Signal<SharedString>) -> Self {
        Self::Bound(value)
    }
}

struct Label {
    content: TextContent,
    style: StyleRefinement,
}

impl Styled for Label {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Declares a text leaf that uses the view's font engine.
pub fn text(content: impl Into<TextContent>) -> impl IntoElement + Styled {
    Label {
        content: content.into(),
        style: StyleRefinement::default(),
    }
}

impl IntoElement for Label {
    fn into_element(self) -> AnyElement {
        let fixed = matches!(self.content, TextContent::Fixed(_));
        AnyElement::new(fixed, move |cx| {
            let (initial, source) = match self.content {
                TextContent::Fixed(value) => (value, None),
                TextContent::Bound(signal) => (cx.read(signal)?.clone(), Some(signal)),
            };
            let mut node = element(Text::new(
                Rc::clone(cx.engine.context("text requires a font")?),
                initial,
                TextStyle {
                    size: px(16.0),
                    line_height: px(24.0),
                },
                true,
            )?);
            node.style = self.style;
            if let Some(signal) = source {
                node.watches
                    .push(Box::new(move |cx, target, id, _, repaints| {
                        let pending = Rc::clone(cx.labels);
                        let view = ViewId(cx.scope);
                        cx.state.bind(signal, target, move |_, _| {
                            pending.borrow_mut().insert((view, id), signal);
                        })?;
                        if let Some(repaints) = repaints {
                            repaints.borrow_mut().invalidate(id);
                        }
                        Ok(())
                    }));
            }
            Ok(node)
        })
    }
}

/// A general-purpose container with children, styles and event handlers.
pub type Div = Node<FlexLayout>;

/// Creates a container that stacks children vertically by default.
pub fn div() -> Div {
    element(FlexLayout {
        direction: FlexDirection::Column,
        justify_content: crate::style::Distribution::Start,
        align_items: Align::Start,
        gap: px(0.0),
    })
}
