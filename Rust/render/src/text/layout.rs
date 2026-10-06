//! Shape styled paragraphs, wrap at Unicode boundaries, and position ruby.
use super::*;
use cosmic_text::{
    Attrs, AttrsList, Buffer, Hinting, LayoutGlyph, LayoutLine, Metrics, ShapeLine, ShapeWord,
    Shaping, Style, Wrap,
};
use onscripter_core::text::{MAX_PAGE_BYTES, Ruby, TextSpan, is_line_break};
use std::ops::Range;

pub(super) struct Prepared {
    pub key: CacheKey,
    pub x: i32,
    pub y: i32,
    pub end: usize,
    pub style: TextStyle,
    pub scale: f32,
    pub underline: Option<[f32; 4]>,
    pub blank: bool,
}

fn attrs<'a>(style: TextStyle, families: &'a HashMap<u8, String>, metadata: usize) -> Attrs<'a> {
    Attrs::new()
        .family(fontdb::Family::Name(&families[&style.font]))
        .weight(if style.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        })
        .style(if style.italic {
            Style::Italic
        } else {
            Style::Normal
        })
        .metrics(Metrics::new(
            style.size as f32,
            style
                .line_height
                .map_or(style.size as f32 * 1.25, |height| height as f32),
        ))
        .letter_spacing(style.spacing as f32 / style.size as f32)
        .metadata(metadata)
}

fn spans(state: &TextState, range: Range<usize>) -> impl Iterator<Item = (usize, &TextSpan)> {
    let start = state
        .spans
        .partition_point(|span| span.range.end <= range.start);
    state
        .spans
        .iter()
        .enumerate()
        .skip(start)
        .take_while(move |(_, span)| span.range.start < range.end)
}

fn shape(
    state: &TextState,
    range: Range<usize>,
    fonts: &mut FontSystem,
    families: &HashMap<u8, String>,
) -> Result<LayoutLine> {
    let text = &state.content[range.clone()];
    let default = attrs(state.base_style, families, 0);
    let mut attributes = AttrsList::new(&default);
    for (index, span) in spans(state, range.clone()) {
        attributes.add_span(
            span.range.start.max(range.start) - range.start
                ..span.range.end.min(range.end) - range.start,
            &attrs(span.style, families, index),
        );
    }
    let mut shaped = ShapeLine::new(fonts, text, &attributes, Shaping::Advanced, 4);
    for bidi in &mut shaped.spans {
        for word in &mut bidi.words {
            let Some(start) = word.glyphs.iter().map(|glyph| glyph.start).min() else {
                continue;
            };
            let end = word
                .glyphs
                .iter()
                .map(|glyph| glyph.end)
                .max()
                .unwrap_or(start);
            let pieces: Vec<Range<usize>> = spans(state, range.start + start..range.start + end)
                .map(|(_, span)| {
                    span.range.start.max(range.start + start) - range.start
                        ..span.range.end.min(range.start + end) - range.start
                })
                .collect();
            if pieces.len() <= 1 {
                continue;
            }
            // Keep the paragraph's bidi order, but shape each formatting or
            // control segment separately. Ligatures must not swallow a pause
            // boundary or carry one span's color into its neighbor.
            word.glyphs.clear();
            for piece in pieces {
                word.glyphs.extend(
                    ShapeWord::new(
                        fonts,
                        text,
                        &attributes,
                        piece,
                        bidi.level,
                        word.blank,
                        Shaping::Advanced,
                    )
                    .glyphs,
                );
            }
        }
    }
    shaped
        .layout(
            state.window.font_size as f32,
            None,
            Wrap::None,
            None,
            None,
            Hinting::Disabled,
        )
        .into_iter()
        .next()
        .ok_or_else(|| Error::invalid("text layout produced no line"))
}

pub(super) fn prepare(
    state: &TextState,
    fonts: &mut FontSystem,
    families: &HashMap<u8, String>,
) -> Result<Vec<Prepared>> {
    let mut result = Vec::new();
    let area_width = (i64::from(state.window.bounds.0) + i64::from(state.window.bounds.2)
        - i64::from(state.window.origin.0))
    .max(1) as f32;
    let bottom = state.window.bounds.1 as f32 + state.window.bounds.3 as f32;
    let mut y = state.window.origin.1 as f32;
    let mut offset = 0;
    for paragraph in state.content.split(is_line_break) {
        if y >= bottom {
            break;
        }
        let range = offset..offset + paragraph.len();
        let width = spans(state, range.clone())
            .filter_map(|(_, span)| span.style.wrap_width)
            .last()
            .map_or(area_width, |width| width as f32);
        let mut run = shape(state, range.clone(), fonts, families)?;
        let fitted = spans(state, range.clone()).any(|(_, span)| span.style.fitted);
        let lines = if fitted || run.w <= width {
            vec![range.clone()]
        } else {
            wrap(state, range.clone(), &run.glyphs, width)?
        };
        for (index, line) in lines.iter().enumerate() {
            if y >= bottom {
                break;
            }
            // An unwrapped paragraph reuses the first shaping pass. Wrapped
            // lines are shaped again so joining and bidi remain correct at breaks.
            if lines.len() != 1 || index != 0 {
                run = shape(state, line.clone(), fonts, families)?;
            }
            let (ascent, descent) = (run.max_ascent, run.max_descent);
            let line_height = run
                .line_height_opt
                .unwrap_or(state.window.font_size as f32 * 1.25);
            let scale = if fitted && run.w > width {
                width / run.w
            } else {
                1.0
            };
            let centered = spans(state, line.clone()).any(|(_, span)| span.style.centered);
            let x = state.window.origin.0 as f32
                + if centered {
                    (width - run.w * scale) / 2.0
                } else {
                    0.0
                };
            let mut annotations = Vec::new();
            let mut extra = 0.0f32;
            for ruby in state
                .ruby
                .iter()
                .filter(|ruby| ruby.range.start >= line.start && ruby.range.end <= line.end)
            {
                let shaped = shape_ruby(ruby, fonts, families)?;
                extra = extra.max(shaped.ascent + shaped.descent + ascent * 0.2);
                annotations.push((ruby, shaped));
            }
            let baseline = y + extra + (line_height - ascent - descent) / 2.0 + ascent;
            for glyph in &run.glyphs {
                let style = state.spans[glyph.metadata].style;
                let physical = glyph.physical((0.0, baseline), 1.0);
                let glyph_x = x + physical.x as f32 * scale;
                push(
                    &mut result,
                    glyph,
                    Prepared {
                        key: physical.cache_key,
                        x: glyph_x.round() as i32,
                        y: physical.y,
                        end: line.start + glyph.end,
                        style,
                        scale,
                        underline: style.underline.then_some([
                            x + glyph.x * scale,
                            baseline + style.size as f32 / 12.0,
                            glyph.w.max(0.0) * scale,
                            (style.size as f32 / 16.0).max(1.0),
                        ]),
                        blank: glyph.w == 0.0
                            || state.content[line.start + glyph.start..line.start + glyph.end]
                                .chars()
                                .all(char::is_whitespace),
                    },
                )?;
            }
            for (ruby, shaped) in annotations {
                let mut left = f32::INFINITY;
                let mut right = f32::NEG_INFINITY;
                for glyph in run.glyphs.iter().filter(|glyph| {
                    line.start + glyph.start < ruby.range.end
                        && line.start + glyph.end > ruby.range.start
                }) {
                    left = left.min(glyph.x);
                    right = right.max(glyph.x + glyph.w);
                }
                if !left.is_finite() || !right.is_finite() {
                    return Err(Error::invalid("ruby base has no visible glyphs"));
                }
                let ruby_x = x + (left + right - shaped.width) * scale / 2.0;
                let ruby_y = baseline - ascent * 1.2 - shaped.descent;
                for glyph in &shaped.glyphs {
                    let physical = glyph.physical((0.0, ruby_y), 1.0);
                    push(
                        &mut result,
                        glyph,
                        Prepared {
                            key: physical.cache_key,
                            x: (ruby_x + physical.x as f32 * scale).round() as i32,
                            y: physical.y,
                            end: ruby.reveal_at,
                            style: ruby.style,
                            scale,
                            underline: ruby.style.underline.then_some([
                                ruby_x + glyph.x * scale,
                                ruby_y + ruby.style.size as f32 / 12.0,
                                glyph.w.max(0.0) * scale,
                                (ruby.style.size as f32 / 16.0).max(1.0),
                            ]),
                            blank: glyph.w == 0.0
                                || ruby.text[glyph.start..glyph.end]
                                    .chars()
                                    .all(char::is_whitespace),
                        },
                    )?;
                }
            }
            y += line_height.max(ascent + descent) + extra;
        }
        offset += paragraph.len();
        if let Some(separator) = state.content[offset..].chars().next() {
            offset += separator.len_utf8();
        }
    }
    Ok(result)
}

fn push(result: &mut Vec<Prepared>, glyph: &LayoutGlyph, prepared: Prepared) -> Result<()> {
    if glyph.glyph_id == 0 {
        return Err(Error::invalid(
            "game font cannot display a dialogue character",
        ));
    }
    if !glyph.w.is_finite() || glyph.w.abs() > 16384.0 || result.len() >= MAX_PAGE_BYTES {
        return Err(Error::Limit("text glyph layout"));
    }
    result
        .try_reserve(1)
        .map_err(|_| Error::Limit("glyph layout allocation"))?;
    result.push(prepared);
    Ok(())
}

struct RubyLayout {
    glyphs: Vec<LayoutGlyph>,
    width: f32,
    ascent: f32,
    descent: f32,
}
fn shape_ruby(
    ruby: &Ruby,
    fonts: &mut FontSystem,
    families: &HashMap<u8, String>,
) -> Result<RubyLayout> {
    let mut buffer = Buffer::new_empty(Metrics::new(
        ruby.style.size as f32,
        ruby.style.size as f32 * 1.25,
    ));
    buffer.set_wrap(Wrap::None);
    buffer.set_text(
        &ruby.text,
        &attrs(ruby.style, families, 0),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(fonts, false);
    let (ascent, descent) = buffer
        .line_layout(fonts, 0)
        .and_then(|lines| lines.first())
        .map_or((0.0, 0.0), |line| (line.max_ascent, line.max_descent));
    let run = buffer
        .layout_runs()
        .next()
        .ok_or_else(|| Error::invalid("ruby annotation produced no line"))?;
    Ok(RubyLayout {
        glyphs: run.glyphs.to_vec(),
        width: run.line_w,
        ascent,
        descent,
    })
}

fn wrap(
    state: &TextState,
    range: Range<usize>,
    glyphs: &[LayoutGlyph],
    width: f32,
) -> Result<Vec<Range<usize>>> {
    let text = &state.content[range.clone()];
    let mut prefix = vec![0.0f32; text.len() + 1];
    let mut boundary = vec![false; text.len() + 1];
    let mut word_break = vec![false; text.len() + 1];
    for glyph in glyphs {
        if !glyph.w.is_finite() || glyph.w.abs() > 16384.0 {
            return Err(Error::Limit("text glyph advance"));
        }
        prefix[glyph.end] += glyph.w.max(0.0);
        boundary[glyph.end] = true;
    }
    for index in 1..prefix.len() {
        prefix[index] += prefix[index - 1];
    }
    for (end, _) in unicode_linebreak::linebreaks(text) {
        word_break[end] = true;
    }
    let mut previous = None;
    for (_, span) in spans(state, range.clone()) {
        if let Some(group) = span.atomic {
            let start = span.range.start.max(range.start) - range.start;
            let end = span.range.end.min(range.end) - range.start;
            boundary[start + 1..end].fill(false);
            if previous == Some((group, start)) {
                boundary[start] = false;
            }
            previous = Some((group, end));
        } else {
            previous = None;
        }
    }
    boundary[text.len()] = true;
    let ends: Vec<usize> = boundary
        .iter()
        .enumerate()
        .filter_map(|(index, allowed)| (*allowed && index != 0).then_some(index))
        .collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut last = start;
        let mut word = start;
        let mut finish = text.len();
        for &end in ends.iter().skip(ends.partition_point(|end| *end <= start)) {
            if prefix[end] - prefix[start] > width {
                finish = if word > start {
                    word
                } else if last > start {
                    last
                } else {
                    end
                };
                break;
            }
            last = end;
            if word_break[end] {
                word = end;
            }
        }
        lines.push(range.start + start..range.start + finish);
        start = finish;
    }
    Ok(lines)
}
