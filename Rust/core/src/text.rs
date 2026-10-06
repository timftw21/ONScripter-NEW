//! Bounded dialogue state; native font and GPU types belong to the renderer.
use crate::{Error, Result};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
};

mod markup;

pub const MAX_PAGE_BYTES: usize = 64 * 1024;
pub const MAX_SPANS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: u8,
    pub size: u32,
    pub color: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub border: u16, // 1/64 pixel, matching the fork's font outlines.
    pub border_color: [u8; 3],
    pub shadow: (i32, i32),
    pub shadow_color: [u8; 3],
    pub spacing: i32,
    pub line_height: Option<u32>,
    pub wrap_width: Option<u32>,
    pub centered: bool,
    pub fitted: bool,
}

impl TextStyle {
    pub fn for_window(window: &TextWindow) -> Self {
        Self {
            font: 0,
            size: window.font_size,
            color: [255; 3],
            bold: window.bold,
            italic: false,
            underline: false,
            border: 0,
            border_color: [0; 3],
            shadow: if window.shadow { (2, 2) } else { (0, 0) },
            shadow_color: [0; 3],
            spacing: 0,
            line_height: None,
            wrap_width: None,
            centered: false,
            fitted: false,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.font > 9
            || !(1..=256).contains(&self.size)
            || self.border > 2048
            || self.shadow.0.unsigned_abs() > 256
            || self.shadow.1.unsigned_abs() > 256
            || self.spacing.unsigned_abs() > 256
            || self
                .line_height
                .is_some_and(|height| !(1..=1024).contains(&height))
            || self
                .wrap_width
                .is_some_and(|width| !(1..=16384).contains(&width))
        {
            return Err(Error::Limit("text style"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub style: TextStyle,
    pub atomic: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Ruby {
    pub range: Range<usize>,
    pub text: String,
    pub style: TextStyle,
    pub reveal_at: usize,
}

#[derive(Debug)]
pub struct TextPreset {
    pub font: u8,
    pub size: Option<u32>,
    pub color: [u8; 3],
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub border: bool,
    pub border_width: Option<u16>,
    pub border_color: [u8; 3],
    pub shadow: bool,
    pub shadow_x: Option<i32>,
    pub shadow_y: Option<i32>,
    pub shadow_color: [u8; 3],
    pub spacing: i32,
    pub line_height: Option<u32>,
    pub wrap_width: Option<u32>,
}

impl TextPreset {
    fn resolve(&self, previous: TextStyle) -> Result<TextStyle> {
        let style = TextStyle {
            font: self.font,
            size: self.size.unwrap_or(previous.size),
            color: self.color,
            bold: self.bold,
            italic: self.italic,
            underline: self.underline,
            border: if self.border {
                self.border_width.unwrap_or(previous.border)
            } else {
                0
            },
            border_color: self.border_color,
            shadow: if self.shadow {
                (
                    self.shadow_x.unwrap_or(previous.shadow.0),
                    self.shadow_y.unwrap_or(previous.shadow.1),
                )
            } else {
                (0, 0)
            },
            shadow_color: self.shadow_color,
            spacing: self.spacing,
            line_height: self.line_height.or(previous.line_height),
            wrap_width: self.wrap_width.or(previous.wrap_width),
            centered: false,
            fitted: false,
        };
        style.validate()?;
        Ok(style)
    }
}

#[derive(Default, Clone)]
struct StyledText {
    text: String,
    spans: Vec<TextSpan>,
    ruby: Vec<Ruby>,
}

#[derive(Clone, Debug)]
pub struct TextWindow {
    pub origin: (i32, i32),
    pub bounds: (i32, i32, u32, u32),
    pub font_size: u32,
    pub bold: bool,
    pub shadow: bool,
    pub color: [u8; 3],
}

#[derive(Debug)]
pub enum TextCommand {
    Window(TextWindow),
    WindowAdvanced {
        window: TextWindow,
        italic: bool,
        underline: bool,
        border: bool,
        spacing: i32,
        line_height: Option<u32>,
        wrap_width: u32,
        speed: u8,
    },
    WindowColor([u8; 3]),
    Dialogue(String),
    DialogueAsync(String),
    Preset {
        number: u32,
        preset: TextPreset,
    },
    Condition {
        index: usize,
        value: bool,
    },
    WaitDialogue(i32),
    ContinueDialogue,
    DisposeDialogue,
    Speed(u8),
    Visible(bool),
    Clear,
    NewLine,
}

enum Part {
    Text(StyledText),
    Click(bool),
    Wait(u64, bool),
    Marker(usize),
    Pause,
}

pub struct TextState {
    pub window: TextWindow,
    pub content: String,
    pub visible: bool,
    pub revealed: usize,
    pub revision: u64,
    pub layout_revision: u64,
    pub base_style: TextStyle,
    pub font: Option<String>,
    pub spans: Vec<TextSpan>,
    pub ruby: Vec<Ruby>,
    speed: u8,
    parts: VecDeque<Part>,
    times: Vec<(u64, usize)>,
    started: u64,
    segment_end: usize,
    pub waiting_click: bool,
    clear_after_click: bool,
    asynchronous: bool,
    suspended: bool,
    passes: i32,
    markers: Vec<i32>,
    wait: Option<(u64, bool)>,
    presets: HashMap<u32, TextPreset>,
    conditions: Vec<bool>,
}

impl TextState {
    pub fn new(size: (u32, u32), font: Option<String>) -> Self {
        let margin = (size.0 / 40).max(4) as i32;
        let top = (size.1 * 2 / 3) as i32;
        let window = TextWindow {
            origin: (margin, top + margin),
            bounds: (0, top, size.0, size.1 - top as u32),
            font_size: (size.1 / 25).clamp(12, 64),
            bold: false,
            shadow: true,
            color: [0; 3],
        };
        Self {
            base_style: TextStyle::for_window(&window),
            window,
            content: String::new(),
            visible: false,
            revealed: 0,
            revision: 0,
            layout_revision: 0,
            font,
            speed: 6,
            parts: VecDeque::new(),
            times: Vec::new(),
            started: 0,
            segment_end: 0,
            waiting_click: false,
            clear_after_click: false,
            spans: Vec::new(),
            ruby: Vec::new(),
            asynchronous: false,
            suspended: false,
            passes: 0,
            markers: Vec::new(),
            wait: None,
            presets: HashMap::new(),
            conditions: Vec::new(),
        }
    }

    pub fn apply(&mut self, command: TextCommand, now: u64) -> Result<bool> {
        match command {
            TextCommand::Dialogue(_) | TextCommand::DialogueAsync(_) => {
                let asynchronous = matches!(command, TextCommand::DialogueAsync(_));
                let data = match command {
                    TextCommand::Dialogue(data) | TextCommand::DialogueAsync(data) => data,
                    _ => unreachable!(),
                };
                // A second d/d2 while d2 is active consumes its line without
                // replacing the dialogue, matching the fork's dialogueCommand.
                if !self.busy() {
                    let first_atomic = self
                        .spans
                        .iter()
                        .filter_map(|span| span.atomic)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1025)
                        .ok_or(Error::Limit("text scopes"))?;
                    let (parts, markers) = markup::parse(
                        &data,
                        self.base_style,
                        &self.presets,
                        &self.conditions,
                        asynchronous,
                        first_atomic,
                    )?;
                    self.parts = parts;
                    self.markers = vec![0; markers];
                    self.asynchronous = asynchronous;
                    self.passes = 0;
                    self.suspended = false;
                    self.wait = None;
                    self.visible = true;
                    self.discard_future();
                    self.prepare_page()?;
                    self.advance(now)?;
                }
                return Ok(!asynchronous && self.busy());
            }
            TextCommand::Preset { number, preset } => {
                preset.resolve(self.base_style)?;
                if !self.presets.contains_key(&number) && self.presets.len() >= 1024 {
                    return Err(Error::Limit("text presets"));
                }
                self.presets
                    .try_reserve(1)
                    .map_err(|_| Error::Limit("text preset allocation"))?;
                self.presets.insert(number, preset);
                return Ok(false);
            }
            TextCommand::Condition { index, value } => {
                if index >= 1024 {
                    return Err(Error::Limit("text conditions"));
                }
                self.conditions
                    .resize(self.conditions.len().max(index + 1), false);
                self.conditions[index] = value;
                return Ok(false);
            }
            TextCommand::WaitDialogue(index) => return self.wait_on(index),
            TextCommand::ContinueDialogue => {
                if !self.asynchronous || !self.busy() {
                    return Err(Error::invalid("d_continue requires active d2 dialogue"));
                }
                self.passes = self
                    .passes
                    .checked_add(1)
                    .filter(|value| *value <= 1024)
                    .ok_or(Error::Limit("dialogue continuations"))?;
                self.advance(now)?;
                return Ok(false);
            }
            TextCommand::DisposeDialogue => {
                if !self.busy() {
                    return Err(Error::invalid("d_dispose requires active dialogue"));
                }
                self.parts.clear();
                self.wait = None;
                self.suspended = false;
                self.waiting_click = false;
                self.revealed = self.segment_end;
                self.times.clear();
                return Ok(false);
            }
            TextCommand::WindowAdvanced {
                window,
                italic,
                underline,
                border,
                spacing,
                line_height,
                wrap_width,
                speed,
            } => {
                let mut style = self.base_style;
                style.size = window.font_size;
                style.bold = window.bold;
                style.italic = italic;
                style.underline = underline;
                style.spacing = if spacing == -999 { 0 } else { spacing };
                style.line_height = line_height;
                style.wrap_width = Some(wrap_width);
                if !border {
                    style.border = 0;
                }
                if !window.shadow {
                    style.shadow = (0, 0);
                }
                style.validate()?;
                if speed > 10 {
                    return Err(Error::Limit("text speed"));
                }
                self.apply(TextCommand::Window(window), now)?;
                self.base_style = style;
                self.speed = speed;
            }
            TextCommand::Window(window) => {
                if !(1..=256).contains(&window.font_size)
                    || !(1..=16384).contains(&window.bounds.2)
                    || !(1..=16384).contains(&window.bounds.3)
                    || [
                        window.origin.0,
                        window.origin.1,
                        window.bounds.0,
                        window.bounds.1,
                    ]
                    .iter()
                    .any(|coordinate| coordinate.unsigned_abs() > 16384)
                {
                    return Err(Error::Limit("text window"));
                }
                self.window = window;
                self.base_style = TextStyle::for_window(&self.window);
                self.clear();
                self.prepare_page()?;
            }
            TextCommand::WindowColor(color) => self.window.color = color,
            TextCommand::Speed(speed) => {
                if speed > 10 {
                    return Err(Error::Limit("text speed"));
                }
                self.speed = speed;
            }
            TextCommand::Visible(visible) => self.visible = visible,
            TextCommand::Clear => {
                self.clear();
                self.prepare_page()?;
            }
            TextCommand::NewLine => self.insert_newline()?,
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(false)
    }

    fn clear(&mut self) {
        self.content.clear();
        self.spans.clear();
        self.ruby.clear();
        self.revealed = 0;
        self.segment_end = 0;
        self.times.clear();
        self.revision = self.revision.wrapping_add(1);
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }

    fn start_text(&mut self, value: &str, now: u64) -> Result<()> {
        let previous = self.segment_end;
        let end = previous.saturating_add(value.len());
        if self.content.get(previous..end) != Some(value) {
            return Err(Error::invalid(
                "dialogue segment is outside its prepared page",
            ));
        }
        self.times.clear();
        self.times
            .try_reserve(value.chars().count())
            .map_err(|_| Error::Limit("dialogue timing allocation"))?;
        let mut elapsed = 0u64;
        for (offset, character) in value.char_indices() {
            elapsed += character_delay(character, self.speed);
            self.times
                .push((elapsed, previous + offset + character.len_utf8()));
        }
        self.revealed = previous;
        self.segment_end = end;
        self.started = now;
        Ok(())
    }

    fn append_layout(&mut self, mut value: StyledText) -> Result<()> {
        let offset = self.content.len();
        let annotations: usize = self
            .ruby
            .iter()
            .chain(&value.ruby)
            .map(|ruby| ruby.text.len())
            .sum();
        if offset
            .saturating_add(value.text.len())
            .saturating_add(annotations)
            > MAX_PAGE_BYTES
        {
            return Err(Error::Limit("dialogue page"));
        }
        if self.spans.len() + value.spans.len() > MAX_SPANS
            || self.ruby.len() + value.ruby.len() > 1024
        {
            return Err(Error::Limit("dialogue spans"));
        }
        self.spans
            .try_reserve(value.spans.len())
            .map_err(|_| Error::Limit("dialogue spans allocation"))?;
        self.ruby
            .try_reserve(value.ruby.len())
            .map_err(|_| Error::Limit("ruby allocation"))?;
        self.content
            .try_reserve(value.text.len())
            .map_err(|_| Error::Limit("dialogue allocation"))?;
        self.content.push_str(&value.text);
        for span in &mut value.spans {
            span.range.start += offset;
            span.range.end += offset;
        }
        for ruby in &mut value.ruby {
            ruby.range.start += offset;
            ruby.range.end += offset;
            ruby.reveal_at += offset;
        }
        self.spans.extend(value.spans);
        self.ruby.extend(value.ruby);
        self.revision = self.revision.wrapping_add(1);
        self.layout_revision = self.layout_revision.wrapping_add(1);
        Ok(())
    }

    fn prepare_page(&mut self) -> Result<()> {
        let chunks: Vec<StyledText> = self
            .parts
            .iter()
            .take_while(|part| !matches!(part, Part::Click(true)))
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
        for chunk in chunks {
            self.append_layout(chunk)?;
        }
        Ok(())
    }

    fn discard_future(&mut self) {
        if self.content.len() == self.revealed {
            return;
        }
        self.content.truncate(self.revealed);
        self.spans.retain(|span| span.range.start < self.revealed);
        for span in &mut self.spans {
            span.range.end = span.range.end.min(self.revealed);
        }
        self.ruby.retain(|ruby| ruby.range.end <= self.revealed);
        self.segment_end = self.revealed;
        self.revision = self.revision.wrapping_add(1);
        self.layout_revision = self.layout_revision.wrapping_add(1);
    }

    fn insert_newline(&mut self) -> Result<()> {
        let bytes: usize =
            self.content.len() + self.ruby.iter().map(|ruby| ruby.text.len()).sum::<usize>();
        if bytes >= MAX_PAGE_BYTES || self.spans.len() >= MAX_SPANS {
            return Err(Error::Limit("dialogue page"));
        }
        let position = self.segment_end;
        self.content
            .try_reserve(1)
            .map_err(|_| Error::Limit("dialogue allocation"))?;
        self.content.insert(position, '\n');
        for span in &mut self.spans {
            if span.range.start >= position {
                span.range.start += 1;
                span.range.end += 1;
            }
        }
        for ruby in &mut self.ruby {
            if ruby.range.start >= position {
                ruby.range.start += 1;
                ruby.range.end += 1;
                ruby.reveal_at += 1;
            }
        }
        let index = self
            .spans
            .partition_point(|span| span.range.start < position);
        self.spans.insert(
            index,
            TextSpan {
                range: position..position + 1,
                style: self.base_style,
                atomic: None,
            },
        );
        self.segment_end += 1;
        self.revealed = self.segment_end;
        self.times.clear();
        self.revision = self.revision.wrapping_add(1);
        self.layout_revision = self.layout_revision.wrapping_add(1);
        Ok(())
    }

    fn next_part(&mut self, now: u64) -> Result<()> {
        match self.parts.pop_front() {
            Some(Part::Text(value)) => self.start_text(&value.text, now)?,
            Some(Part::Click(clear)) => {
                self.waiting_click = true;
                self.clear_after_click = clear;
            }
            Some(Part::Wait(duration, skippable)) => {
                self.wait = Some((now.saturating_add(duration), skippable))
            }
            Some(Part::Marker(index)) => self.markers[index] += 1,
            Some(Part::Pause) => {
                self.passes -= 1;
                self.suspended = self.passes < 0;
            }
            None => {}
        }
        Ok(())
    }

    pub fn animating(&self) -> bool {
        self.revealed < self.segment_end
    }
    pub fn busy(&self) -> bool {
        self.animating()
            || self.waiting_click
            || !self.parts.is_empty()
            || self.suspended
            || self.wait.is_some()
    }

    pub fn wait_deadline(&self) -> Option<u64> {
        self.wait.map(|(deadline, _)| deadline)
    }

    fn wait_on(&mut self, index: i32) -> Result<bool> {
        if index < -1 {
            return Err(Error::invalid("wait_on_d index must be -1 or a [#] index"));
        }
        if !self.busy() {
            return Ok(false);
        }
        if index == -1 {
            return Ok(true);
        }
        let marker = self
            .markers
            .get_mut(index as usize)
            .ok_or_else(|| Error::invalid("wait_on_d index has no matching [#]"))?;
        *marker = marker
            .checked_sub(1)
            .ok_or(Error::Limit("dialogue marker waits"))?;
        Ok(*marker < 0)
    }

    pub fn signal_ready(&self, index: i32) -> bool {
        !self.busy()
            || (index >= 0
                && self
                    .markers
                    .get(index as usize)
                    .is_some_and(|marker| *marker >= 0))
    }

    pub fn advance(&mut self, now: u64) -> Result<bool> {
        let old = self.revealed;
        let revision = self.revision;
        let elapsed = now.saturating_sub(self.started);
        let count = self.times.partition_point(|(time, _)| *time <= elapsed);
        if let Some((_, end)) = count.checked_sub(1).and_then(|index| self.times.get(index)) {
            self.revealed = self.revealed.max(*end);
        }
        if self.wait.is_some_and(|(deadline, _)| now >= deadline) {
            self.wait = None;
        }
        if self.suspended && self.passes >= 0 {
            self.suspended = false;
        }
        while !self.animating()
            && !self.waiting_click
            && !self.suspended
            && self.wait.is_none()
            && !self.parts.is_empty()
        {
            self.next_part(now)?;
            let count = self.times.partition_point(|(time, _)| *time == 0);
            if let Some((_, end)) = count.checked_sub(1).and_then(|index| self.times.get(index)) {
                self.revealed = self.revealed.max(*end);
            }
            if self.wait.is_some_and(|(deadline, _)| now >= deadline) {
                self.wait = None;
            }
        }
        Ok(old != self.revealed || revision != self.revision)
    }

    /// First input completes the current segment; subsequent input advances its click.
    pub fn input(&mut self, now: u64) -> Result<bool> {
        if self.animating() {
            self.revealed = self.segment_end;
            self.times.clear();
        } else if self.waiting_click {
            self.waiting_click = false;
            if self.clear_after_click {
                self.clear();
                self.prepare_page()?;
            }
        } else if self.wait.is_some_and(|(_, skippable)| skippable) {
            self.wait = None;
        } else {
            return Ok(false);
        }
        self.advance(now)?;
        Ok(true)
    }
}

/// Hard line and paragraph separators accepted by dialogue layout.
pub const fn is_line_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn character_delay(character: char, speed: u8) -> u64 {
    let base = match character {
        '⅓' => 13,
        ',' => 100,
        ';' | ':' | '—' => 145,
        '.' | '?' | '!' => 170,
        '\u{3000}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' | '\u{ff00}'..='\u{ffef}' => 60,
        _ => 20,
    };
    (base - base * i32::from(speed) / 10).max(0) as u64
}
