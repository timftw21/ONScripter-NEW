//! The fork's {command:parameter:body} scopes, parsed once before display.
use super::*;

struct Scope {
    style: TextStyle,
    atomic: Option<u32>,
    ignored: bool,
    ruby: Option<(usize, String, TextStyle)>,
}

pub(super) fn parse(
    data: &str,
    default: TextStyle,
    presets: &HashMap<u32, TextPreset>,
    conditions: &[bool],
    asynchronous: bool,
    first_atomic: u32,
) -> Result<(VecDeque<Part>, usize)> {
    if data.len() > MAX_PAGE_BYTES {
        return Err(Error::Limit("dialogue"));
    }
    let mut text = StyledText::default();
    let mut parts = VecDeque::new();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut style = default;
    let mut atomic = None;
    let mut ignored = false;
    let mut cursor = 0;
    let mut controls = 0;
    let mut markers = 0;
    let mut group = first_atomic;
    while cursor < data.len() {
        let rest = &data[cursor..];
        match rest.as_bytes()[0] {
            b'{' => {
                controls += 1;
                if scopes.len() >= 64 {
                    return Err(Error::Limit("text scope depth"));
                }
                let name_end = rest[1..]
                    .find(|c: char| !c.is_ascii_alphabetic())
                    .map(|end| end + 1)
                    .ok_or_else(|| Error::invalid("unterminated text scope"))?;
                let name = &rest[1..name_end];
                let delimiter = rest[name_end..].chars().next().unwrap();
                if !matches!(delimiter, ':' | '}') {
                    return Err(Error::invalid(
                        "text scopes use {tag:parameter:body} or {tag:body}",
                    ));
                }
                cursor += name_end + usize::from(delimiter == ':');
                let mut parameter = "";
                if delimiter == ':' {
                    let body = &data[cursor..];
                    let end = body.find(':');
                    let brace = body.find(['{', '}']);
                    if let Some(end) = end.filter(|end| brace.is_none_or(|brace| *end < brace)) {
                        parameter = &body[..end];
                        cursor += end + 1;
                    }
                }
                let mut scope = Scope {
                    style,
                    atomic,
                    ignored,
                    ruby: None,
                };
                if !ignored {
                    match name {
                        "italic" | "i" => {
                            style.italic = true;
                            style.bold = false;
                        }
                        "bold" | "b" => {
                            style.bold = true;
                            style.italic = false;
                        }
                        "bolditalic" | "x" => {
                            style.bold = true;
                            style.italic = true;
                        }
                        "underline" | "u" => style.underline = true,
                        "color" | "colour" | "c" => style.color = color(parameter)?,
                        "shadowcolor" | "shadowcolour" | "v" => {
                            style.shadow_color = color(parameter)?
                        }
                        "bordercolor" | "bordercolour" | "r" => {
                            style.border_color = color(parameter)?
                        }
                        "fontsize" | "fontsizeabsolute" | "size" | "d" => {
                            style.size = unsigned(parameter)?
                        }
                        "fontsizepercent" | "fontsizepc" | "sizepercent" | "sizepc" | "e" => {
                            style.size = u32::try_from(
                                u64::from(style.size) * u64::from(unsigned(parameter)?) / 100,
                            )
                            .map_err(|_| Error::Limit("text style"))?;
                        }
                        "font" | "f" => {
                            style.font = u8::try_from(unsigned(parameter)?)
                                .map_err(|_| Error::Limit("text font number"))?
                        }
                        "characterspacing" | "charspacing" | "m" => {
                            style.spacing = integer(parameter)?
                        }
                        "border" | "borderwidth" | "o" => {
                            style.border = u16::try_from(u64::from(unsigned(parameter)?) * 25)
                                .map_err(|_| Error::Limit("text border"))?
                        }
                        "shadow" | "shadowdistance" | "s" => {
                            let (x, y) = parameter
                                .split_once(',')
                                .ok_or_else(|| Error::invalid("shadow requires x,y distances"))?;
                            style.shadow = (integer(x)?, integer(y)?);
                        }
                        "center" | "centre" | "ac" => style.centered = true,
                        "alignment" | "a" if parameter.starts_with('c') => style.centered = true,
                        // These are accepted no-ops in the native fork too.
                        "left" | "al" | "right" | "ar" | "alignment" | "a" => {}
                        "fit" | "j" => style.fitted = true,
                        "width" | "w" => style.wrap_width = Some(unsigned(parameter)?),
                        "nobreak" | "nobr" => {
                            group += 1;
                            atomic = Some(group);
                        }
                        "preset" | "p" => {
                            style = presets
                                .get(&unsigned(parameter)?)
                                .ok_or_else(|| Error::invalid("text preset is not defined"))?
                                .resolve(style)?;
                        }
                        "y" | "n" => {
                            let index = unsigned(parameter)? as usize;
                            if index >= 1024 {
                                return Err(Error::Limit("text condition index"));
                            }
                            ignored =
                                conditions.get(index).copied().unwrap_or(false) != (name == "y");
                        }
                        "ruby" | "h" => {
                            if parameter.is_empty() || parameter.chars().any(is_line_break) {
                                return Err(Error::invalid(
                                    "ruby requires a single-line annotation",
                                ));
                            }
                            if scopes.iter().any(|scope| scope.ruby.is_some()) {
                                return Err(Error::invalid("nested ruby is unsupported"));
                            }
                            let mut ruby_style = style;
                            ruby_style.size = (style.size * 3 / 5).max(1);
                            scope.ruby = Some((text.text.len(), parameter.to_owned(), ruby_style));
                            group += 1;
                            atomic = Some(group);
                        }
                        tag => {
                            return Err(Error::invalid(format!(
                                "text style {{{tag}}} is not implemented"
                            )));
                        }
                    }
                    style.validate()?;
                }
                scopes.push(scope);
            }
            b'}' => {
                let scope = scopes
                    .pop()
                    .ok_or_else(|| Error::invalid("unmatched closing text scope"))?;
                if let Some((start, annotation, ruby_style)) = scope.ruby {
                    let base = &text.text[start..];
                    if base.is_empty() || base.chars().any(is_line_break) {
                        return Err(Error::invalid("ruby requires a nonempty single-line base"));
                    }
                    let middle = base.chars().count().div_ceil(2) - 1;
                    let (offset, character) = base.char_indices().nth(middle).unwrap();
                    text.ruby.push(Ruby {
                        range: start..text.text.len(),
                        text: annotation,
                        style: ruby_style,
                        reveal_at: start + offset + character.len_utf8(),
                    });
                }
                style = scope.style;
                atomic = scope.atomic;
                ignored = scope.ignored;
                cursor += 1;
            }
            b'[' => {
                controls += 1;
                let end = rest
                    .find(']')
                    .ok_or_else(|| Error::invalid("unterminated dialogue control"))?;
                let tag = &rest[1..end];
                if tag == "br" {
                    push_text(&mut text, "\n", style, atomic)?;
                } else {
                    if scopes.iter().any(|scope| scope.ruby.is_some()) {
                        return Err(Error::invalid("ruby cannot cross a dialogue control"));
                    }
                    flush(&mut parts, &mut text);
                    parts.push_back(match tag {
                        "@" => Part::Click(false),
                        "\\" => Part::Click(true),
                        "*" if asynchronous => Part::Pause,
                        "#" if asynchronous => {
                            let index = markers;
                            markers += 1;
                            Part::Marker(index)
                        }
                        _ if tag.starts_with("!w") || tag.starts_with("!d") => {
                            Part::Wait(u64::from(unsigned(&tag[2..])?), tag.starts_with("!d"))
                        }
                        _ => {
                            return Err(Error::invalid(format!(
                                "dialogue control [{tag}] is not implemented"
                            )));
                        }
                    });
                }
                cursor += end + 1;
            }
            b'#' if rest.len() >= 7 && rest.as_bytes()[1..7].iter().all(u8::is_ascii_hexdigit) => {
                if !ignored {
                    style.color = color(&rest[1..7])?;
                }
                cursor += 7;
            }
            _ => {
                let first = rest.chars().next().unwrap().len_utf8();
                let end = rest[first..]
                    .find(['{', '}', '[', '#'])
                    .map_or(rest.len(), |end| end + first);
                if !ignored {
                    push_text(&mut text, &rest[..end], style, atomic)?;
                }
                cursor += end;
            }
        }
        if controls > 1024 {
            return Err(Error::Limit("dialogue controls"));
        }
    }
    if !scopes.is_empty() {
        return Err(Error::invalid("unterminated text scope"));
    }
    flush(&mut parts, &mut text);
    Ok((parts, markers))
}

fn flush(parts: &mut VecDeque<Part>, text: &mut StyledText) {
    if !text.text.is_empty() {
        parts.push_back(Part::Text(std::mem::take(text)));
    }
}

pub(super) fn push_text(
    text: &mut StyledText,
    value: &str,
    style: TextStyle,
    atomic: Option<u32>,
) -> Result<()> {
    if text.spans.len() >= MAX_SPANS {
        return Err(Error::Limit("dialogue spans"));
    }
    text.text
        .try_reserve(value.len())
        .map_err(|_| Error::Limit("dialogue allocation"))?;
    let start = text.text.len();
    text.text.push_str(value);
    if let Some(last) = text
        .spans
        .last_mut()
        .filter(|last| last.style == style && last.atomic == atomic)
    {
        last.range.end = text.text.len();
    } else {
        text.spans.push(TextSpan {
            range: start..text.text.len(),
            style,
            atomic,
        });
    }
    Ok(())
}

fn integer(value: &str) -> Result<i32> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::invalid("invalid integer in text style or delay"))
}
fn unsigned(value: &str) -> Result<u32> {
    u32::try_from(integer(value)?).map_err(|_| Error::invalid("text parameter must be nonnegative"))
}
fn color(value: &str) -> Result<[u8; 3]> {
    crate::scene::parse_color(&format!("#{value}"))
}
