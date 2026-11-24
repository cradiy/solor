//! Playback-synchronized text with a soft reveal and cumulative phrase lift.
use anyhow::{Context as _, Result};
use solor::{
    Div, EffectShader, PaintContext, Signal, Styled, Ui, component, div,
    layout::{Origin, Query, Size},
    px, rgb, rgba,
    style::{TextAttributes, Typography, WhiteSpace},
    text,
    text::{Line, Paragraph},
};
use solor_platform::{WindowOptions, current_platform};
use std::rc::Rc;

const PERIOD_MS: f64 = 6000.;
type Piece = (&'static str, u64, u64);

const CHINESE: &[Piece] = &[
    ("长音符", 0, 1500),
    ("短", 1500, 1850),
    ("长旋律", 1850, 3450),
    ("快", 3450, 3750),
    ("长回声", 3750, 5750),
    ("啪", 5750, 6000),
];
const ENGLISH: &[Piece] = &[
    ("sustain", 0, 1500),
    ("short", 1500, 1850),
    ("melody", 1850, 3450),
    ("tap", 3450, 3750),
    ("long", 3750, 4550),
    ("echo", 4550, 5750),
    ("pop", 5750, 6000),
];
const MIXED: &[Piece] = &[
    ("长音符", 0, 1500),
    ("short", 1500, 1850),
    ("长旋律", 1850, 3450),
    ("tap", 3450, 3750),
    ("long", 3750, 4550),
    ("echo", 4550, 5750),
    ("快", 5750, 6000),
];

struct Phrase {
    paragraph: Rc<Paragraph>,
    start_ms: u64,
    end_ms: u64,
    start_x: f32,
    end_x: f32,
    origin_x: f32,
}

impl Phrase {
    fn progress(&self, ms: f32) -> f32 {
        ((ms - self.start_ms as f32) / (self.end_ms - self.start_ms) as f32).clamp(0., 1.)
    }
}

fn phrases(
    context: &solor::Context<'_>,
    line: &'static str,
    pieces: &[Piece],
) -> Result<Vec<Phrase>> {
    let typography = Typography {
        size: px(28.),
        line_height: px(44.),
        attributes: Some(Rc::new(TextAttributes {
            font_weight: Some(600),
            white_space: Some(WhiteSpace::Pre),
            ..Default::default()
        })),
        ..Default::default()
    };
    let shaped = context.shape_text(line, &typography, Query::Natural)?;
    let [layout] = shaped.lines.as_slice() else {
        anyhow::bail!("expected a single lyric line")
    };
    let em = typography.size.get();
    let mut cursor = 0;
    pieces
        .iter()
        .map(|&(piece, start_ms, end_ms)| {
            let start = cursor + line[cursor..].find(piece).context("missing timed phrase")?;
            cursor = start + piece.len();
            let mut glyphs: Vec<_> = layout
                .glyphs
                .iter()
                .filter(|g| (start..cursor).contains(&g.start))
                .cloned()
                .collect();
            anyhow::ensure!(!glyphs.is_empty(), "unshaped timed phrase {piece}");
            let start_x = glyphs.iter().map(|g| g.x).fold(f32::INFINITY, f32::min);
            let end_x = glyphs
                .iter()
                .map(|g| g.x + g.width)
                .fold(f32::NEG_INFINITY, f32::max);
            // Keep the original advances and allow ink to extend beyond the timed range.
            let origin_x = start_x - em;
            for glyph in &mut glyphs {
                glyph.x -= origin_x;
            }
            let paragraph = Paragraph {
                lines: vec![Line {
                    glyphs,
                    width: end_x - start_x,
                    ..layout.clone()
                }],
                content: Size {
                    width: px(end_x - origin_x + em),
                    height: shaped.content.height,
                },
                ..shaped.as_ref().clone()
            };
            Ok(Phrase {
                paragraph: Rc::new(paragraph),
                start_ms,
                end_ms,
                start_x,
                end_x,
                origin_x,
            })
        })
        .collect()
}

fn draw_line(
    cx: &mut PaintContext<'_>,
    phrases: &[Phrase],
    ms: f32,
    shader: &EffectShader,
) -> Result<()> {
    let mut reveal = 0.;
    for phrase in phrases {
        if ms < phrase.start_ms as f32 {
            break;
        }
        reveal = phrase.start_x + (phrase.end_x - phrase.start_x) * phrase.progress(ms);
    }
    let completed = (reveal - 12.).max(0.);
    for phrase in phrases {
        let t = phrase.progress(ms);
        let lift = 3. * t * t * t * (t * (t * 6. - 15.) + 10.);
        let origin = Origin {
            x: px(phrase.origin_x),
            y: px(3. - lift),
        };
        cx.with_offset(origin, |cx| {
            let paragraph = &phrase.paragraph;
            cx.draw_paragraph(paragraph.clone(), paragraph.content, rgba(0xe7edf840));
            if reveal > 0. && completed >= phrase.origin_x + paragraph.content.width.get() {
                cx.draw_paragraph(paragraph.clone(), paragraph.content, rgb(0x67c7e9));
            } else if reveal > 0. && reveal + 7. > phrase.origin_x {
                let ink = shader.with_params([
                    reveal - phrase.origin_x,
                    completed - phrase.origin_x,
                    0.,
                    0.,
                ]);
                cx.draw_paragraph(paragraph.clone(), paragraph.content, ink);
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn row(
    label: &'static str,
    phrases: Vec<Phrase>,
    position: Signal<f32>,
    shader: EffectShader,
) -> Div {
    div()
        .w_full()
        .flex_row()
        .items_center()
        .gap(px(16.))
        .child(
            div()
                .w(px(54.))
                .child(text(label).text_size(px(14.)).text_color(rgba(0xe7edf870))),
        )
        .child(
            div()
                .flex_1()
                .h(px(50.))
                .overflow_hidden()
                .paint(move |cx| {
                    let ms = *cx.read(position)?;
                    draw_line(cx, &phrases, ms, &shader)
                }),
        )
}

fn main() -> Result<()> {
    let shader = EffectShader::wgsl(include_str!("reveal.wgsl"))?;
    let mut ui = Ui::new();
    let lines = [
        (
            "中文",
            phrases(&ui.context(), "长音符短长旋律快长回声啪", CHINESE)?,
        ),
        (
            "EN",
            phrases(
                &ui.context(),
                "sustain  short  melody  tap  long echo  pop",
                ENGLISH,
            )?,
        ),
        (
            "混合",
            phrases(&ui.context(), "长音符short长旋律tap long echo快", MIXED)?,
        ),
    ];
    let view = component(move |cx| {
        let position = cx.signal(0.);
        let mut start = None;
        div()
            .w_full()
            .h_full()
            .items_center()
            .justify_center()
            .bg(rgb(0x090d18))
            .p(px(20.))
            .child(
                div()
                    .w_full()
                    .max_w(px(900.))
                    .items_stretch()
                    .px(px(42.))
                    .py(px(48.))
                    .gap(px(20.))
                    .rounded(px(28.))
                    .bg(rgb(0x11182a))
                    .child(
                        div()
                            .gap(px(4.))
                            .child(
                                text("PROGRESSIVE WORD LIFT")
                                    .text_color(rgb(0x8be9fd))
                                    .text_size(px(14.)),
                            )
                            .child(
                                text("RISES DURING PLAYBACK · HOLDS AT WORD END")
                                    .text_color(rgba(0xe7edf880))
                                    .text_size(px(14.)),
                            )
                            .child(
                                text("SEGMENTATION  ·  EXPLICIT UTF-8 RANGES")
                                    .text_color(rgba(0xe7edf860))
                                    .text_size(px(14.)),
                            ),
                    )
                    .children(
                        lines.map(|(label, phrases)| row(label, phrases, position, shader.clone())),
                    ),
            )
            .on_frame(move |now, cx| {
                let elapsed = now.saturating_sub(*start.get_or_insert(now));
                cx.set(
                    position,
                    ((elapsed.as_secs_f64() * 1000.) % PERIOD_MS) as f32,
                )
            })
    });
    current_platform()?.run_window(
        WindowOptions::new("Timed text").size(px(1280.), px(620.)),
        ui,
        view,
    )
}
