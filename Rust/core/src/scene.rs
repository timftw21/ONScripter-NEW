//! Scene state and image tags without native graphics types.

use std::{collections::BTreeMap, sync::Arc};

use crate::{Error, Limits, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transparency {
    Copy,
    Alpha,
    TopLeft,
    TopRight,
    Color([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    Normal,
    Add,
    Multiply,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageKey {
    pub name: Arc<str>,
    pub transparency: Transparency,
    pub blend: Blend,
    pub cells: u32,
    pub vertical: bool,
}

#[derive(Clone, Debug)]
pub struct ImageSpec {
    pub key: ImageKey,
    animation: Option<Animation>,
}

#[derive(Clone, Debug)]
struct Animation {
    // End times allow a binary search even for large sheets and missed frames.
    frames: Vec<(u64, u32)>,
    repeat: bool,
}

impl ImageSpec {
    pub fn parse(input: &str, limits: Limits) -> Result<Self> {
        let maximum_tag = limits.filename_bytes.saturating_add(
            (limits.sprites as usize)
                .saturating_mul(24)
                .saturating_add(128),
        );
        if input.len() > maximum_tag {
            return Err(Error::Limit("image tag"));
        }
        let mut key = ImageKey {
            name: Arc::from(input),
            transparency: Transparency::TopLeft,
            blend: Blend::Normal,
            cells: 1,
            vertical: false,
        };
        let mut animation = None;
        if let Some(tag) = input.strip_prefix(':') {
            let (tag, filename) = tag
                .split_once(';')
                .ok_or_else(|| Error::invalid("image tag has no filename separator"))?;
            key.name = Arc::from(filename);
            let (mode, details) = tag.trim().split_once('/').unwrap_or((tag.trim(), ""));
            match mode {
                "a" => key.transparency = Transparency::Alpha,
                "c" => key.transparency = Transparency::Copy,
                "l" => key.transparency = Transparency::TopLeft,
                "r" => key.transparency = Transparency::TopRight,
                "d" => {
                    key.transparency = Transparency::Copy;
                    key.blend = Blend::Add;
                }
                "u" => {
                    key.transparency = Transparency::Copy;
                    key.blend = Blend::Multiply;
                }
                color if color.starts_with('#') => {
                    key.transparency = Transparency::Color(parse_color(color)?);
                }
                _ => {
                    return Err(Error::invalid(format!(
                        "image tag {mode} is not implemented"
                    )));
                }
            }
            if !details.is_empty() {
                let (count, rest) = details.split_once(',').unwrap_or((details, ""));
                key.cells = count
                    .trim()
                    .parse()
                    .map_err(|_| Error::invalid("invalid cell count"))?;
                if key.cells == 0 || key.cells > limits.sprites {
                    return Err(Error::Limit("image cells"));
                }
                if !rest.is_empty() {
                    let (durations, rest) = if let Some(list) = rest.strip_prefix('<') {
                        let (list, rest) = list
                            .split_once(">")
                            .ok_or_else(|| Error::invalid("unterminated cell durations"))?;
                        (
                            list,
                            rest.strip_prefix(',')
                                .ok_or_else(|| Error::invalid("missing animation mode"))?,
                        )
                    } else {
                        rest.split_once(',')
                            .ok_or_else(|| Error::invalid("missing animation mode"))?
                    };
                    let (mode, vertical) = rest.split_once(',').unwrap_or((rest, "0"));
                    let mode = mode
                        .trim()
                        .parse::<u8>()
                        .map_err(|_| Error::invalid("invalid animation mode"))?;
                    key.vertical = match vertical.trim() {
                        "0" => false,
                        "1" => true,
                        _ => return Err(Error::invalid("invalid vertical-cell flag")),
                    };
                    if mode > 3 {
                        return Err(Error::invalid("invalid animation mode"));
                    }
                    let mut parsed_durations = Vec::new();
                    for value in durations.split(',') {
                        if parsed_durations.len() >= key.cells as usize {
                            return Err(Error::Limit("cell durations"));
                        }
                        parsed_durations
                            .try_reserve(1)
                            .map_err(|_| Error::Limit("cell duration allocation"))?;
                        parsed_durations.push(
                            value
                                .trim()
                                .parse::<u32>()
                                .map_err(|_| Error::invalid("invalid cell duration"))?,
                        );
                    }
                    let durations = parsed_durations;
                    if durations.len() != 1 && durations.len() != key.cells as usize {
                        return Err(Error::invalid(
                            "cell duration count differs from cell count",
                        ));
                    }
                    if mode != 3 && key.cells > 1 {
                        if durations.contains(&0) {
                            return Err(Error::invalid(
                                "animated cells require positive durations",
                            ));
                        }
                        let mut frames = Vec::new();
                        frames
                            .try_reserve(key.cells as usize * 2)
                            .map_err(|_| Error::Limit("cell animation allocation"))?;
                        let mut end = 0u64;
                        let sequence =
                            (0..key.cells).chain((1..key.cells - 1).rev().filter(|_| mode == 2));
                        for cell in sequence {
                            end += u64::from(
                                durations[if durations.len() == 1 {
                                    0
                                } else {
                                    cell as usize
                                }],
                            );
                            frames.push((end, cell));
                        }
                        animation = Some(Animation {
                            frames,
                            repeat: mode != 1,
                        });
                    }
                }
            }
        }
        if key.name.is_empty() || key.name.len() > limits.filename_bytes {
            return Err(Error::Limit("image filename"));
        }
        Ok(Self { key, animation })
    }

    pub fn background(input: &str, limits: Limits) -> Result<Self> {
        let mut image = Self::parse(input, limits)?;
        image.key.transparency = Transparency::Copy;
        image.key.blend = Blend::Normal;
        image.key.cells = 1;
        image.key.vertical = false;
        image.animation = None;
        Ok(image)
    }

    fn metadata_bytes(&self) -> usize {
        self.key.name.len()
            + 128
            + self.animation.as_ref().map_or(0, |animation| {
                animation.frames.capacity() * std::mem::size_of::<(u64, u32)>()
            })
    }

    fn cell_start(&self, initial: u32) -> u64 {
        if initial == 0 {
            return 0;
        }
        self.animation
            .as_ref()
            .and_then(|animation| {
                animation
                    .frames
                    .iter()
                    .position(|(_, cell)| *cell == initial)
                    .map(|index| {
                        if index == 0 {
                            0
                        } else {
                            animation.frames[index - 1].0
                        }
                    })
            })
            .unwrap_or(0)
    }

    fn cell_at(&self, elapsed: u64, initial: u32) -> u32 {
        let Some(animation) = &self.animation else {
            return initial;
        };
        let initial_time = self.cell_start(initial);
        let period = animation.frames.last().map_or(1, |frame| frame.0);
        let time = elapsed.saturating_add(initial_time);
        let time = if animation.repeat {
            time % period
        } else {
            time.min(period - 1)
        };
        animation.frames[animation.frames.partition_point(|frame| frame.0 <= time)].1
    }
}

pub fn parse_color(value: &str) -> Result<[u8; 3]> {
    let digits = value
        .strip_prefix('#')
        .ok_or_else(|| Error::invalid("color must start with #"))?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::invalid("color must contain six hex digits"));
    }
    let color = u32::from_str_radix(digits, 16).map_err(|_| Error::invalid("invalid color"))?;
    Ok([(color >> 16) as u8, (color >> 8) as u8, color as u8])
}

#[derive(Clone, Debug)]
pub enum Background {
    Color([u8; 3]),
    Image(Arc<ImageSpec>),
}

#[derive(Clone, Debug)]
pub struct Sprite {
    pub image: Arc<ImageSpec>,
    pub x: i32,
    pub y: i32,
    pub opacity: u8,
    pub visible: bool,
    pub cell: u32,
    pub loaded_at: u64,
}

impl Sprite {
    pub fn cell_at(&self, now: u64) -> u32 {
        self.image
            .cell_at(now.saturating_sub(self.loaded_at), self.cell)
    }

    pub fn animated(&self, now: u64) -> bool {
        self.visible
            && self.opacity != 0
            && self.image.animation.as_ref().is_some_and(|animation| {
                animation.repeat
                    || animation.frames.last().is_some_and(|frame| {
                        now.saturating_sub(self.loaded_at)
                            .saturating_add(self.image.cell_start(self.cell))
                            < frame.0
                    })
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Effect {
    pub duration_ms: u32,
}

#[derive(Debug)]
pub enum SceneCommand {
    Background {
        name: String,
        effect: Effect,
    },
    Load {
        slot: u32,
        image: String,
        x: i32,
        y: i32,
        opacity: u8,
        visible: bool,
    },
    Clear {
        first: i32,
        last: i32,
    },
    Visible {
        slot: u32,
        visible: bool,
    },
    Move {
        slot: u32,
        x: i32,
        y: i32,
        opacity: Option<i32>,
        relative: bool,
    },
    Cell {
        slot: u32,
        cell: u32,
    },
    HideAll(bool),
    Present(Effect),
}

#[derive(Clone, Debug)]
pub struct Scene {
    background: Background,
    sprites: BTreeMap<u32, Sprite>,
    hidden: bool,
    metadata_bytes: usize,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            background: Background::Color([0; 3]),
            sprites: BTreeMap::new(),
            hidden: false,
            metadata_bytes: 0,
        }
    }
}

impl Scene {
    pub fn background(&self) -> &Background {
        &self.background
    }
    pub fn sprites(&self) -> impl DoubleEndedIterator<Item = &Sprite> {
        self.sprites.values()
    }
    pub fn hidden(&self) -> bool {
        self.hidden
    }
    pub fn apply(
        &mut self,
        command: SceneCommand,
        now: u64,
        limits: Limits,
    ) -> Result<Option<Effect>> {
        let check_slot = |slot: u32| -> Result<()> {
            if slot >= limits.sprites {
                Err(Error::Limit("sprite slot"))
            } else {
                Ok(())
            }
        };
        match command {
            SceneCommand::Background { name, effect } => {
                let background = match name.as_str() {
                    "black" => Background::Color([0; 3]),
                    "white" => Background::Color([255; 3]),
                    color if color.starts_with('#') => Background::Color(parse_color(color)?),
                    _ => Background::Image(Arc::new(ImageSpec::background(&name, limits)?)),
                };
                let cost = |background: &Background| match background {
                    Background::Image(image) => image.metadata_bytes(),
                    _ => 0,
                };
                self.metadata_bytes = self
                    .metadata_bytes
                    .saturating_sub(cost(&self.background))
                    .checked_add(cost(&background))
                    .filter(|&bytes| bytes <= limits.state_bytes)
                    .ok_or(Error::Limit("scene state"))?;
                self.background = background;
                return Ok(Some(effect));
            }
            SceneCommand::Load {
                slot,
                image,
                x,
                y,
                opacity,
                visible,
            } => {
                check_slot(slot)?;
                let image = Arc::new(ImageSpec::parse(&image, limits)?);
                let previous = self
                    .sprites
                    .get(&slot)
                    .map_or(0, |sprite| sprite.image.metadata_bytes() + 128);
                self.metadata_bytes = self
                    .metadata_bytes
                    .saturating_sub(previous)
                    .checked_add(image.metadata_bytes() + 128)
                    .filter(|&bytes| bytes <= limits.state_bytes)
                    .ok_or(Error::Limit("scene state"))?;
                self.sprites.insert(
                    slot,
                    Sprite {
                        image,
                        x,
                        y,
                        opacity,
                        visible,
                        cell: 0,
                        loaded_at: now,
                    },
                );
            }
            SceneCommand::Clear { first, last } => {
                if first == -1 {
                    for sprite in self.sprites.values() {
                        self.metadata_bytes -= sprite.image.metadata_bytes() + 128;
                    }
                    self.sprites.clear();
                } else {
                    let (first, last) = (first.min(last), first.max(last));
                    check_slot(
                        u32::try_from(first).map_err(|_| Error::invalid("negative sprite slot"))?,
                    )?;
                    check_slot(
                        u32::try_from(last).map_err(|_| Error::invalid("negative sprite slot"))?,
                    )?;
                    self.sprites.retain(|slot, sprite| {
                        let keep = *slot < first as u32 || *slot > last as u32;
                        if !keep {
                            self.metadata_bytes -= sprite.image.metadata_bytes() + 128;
                        }
                        keep
                    });
                }
            }
            SceneCommand::Visible { slot, visible } => {
                check_slot(slot)?;
                if let Some(sprite) = self.sprites.get_mut(&slot) {
                    sprite.visible = visible;
                }
            }
            SceneCommand::Move {
                slot,
                x,
                y,
                opacity,
                relative,
            } => {
                check_slot(slot)?;
                if let Some(sprite) = self.sprites.get_mut(&slot) {
                    sprite.x = if relative {
                        sprite.x.wrapping_add(x)
                    } else {
                        x
                    };
                    sprite.y = if relative {
                        sprite.y.wrapping_add(y)
                    } else {
                        y
                    };
                    if let Some(opacity) = opacity {
                        sprite.opacity = (if relative {
                            i32::from(sprite.opacity).saturating_add(opacity)
                        } else {
                            opacity
                        })
                        .clamp(0, 255) as u8;
                    }
                }
            }
            SceneCommand::Cell { slot, cell } => {
                check_slot(slot)?;
                if let Some(sprite) = self.sprites.get_mut(&slot) {
                    sprite.cell = cell.min(sprite.image.key.cells - 1);
                    sprite.loaded_at = now;
                }
            }
            SceneCommand::HideAll(hidden) => self.hidden = hidden,
            SceneCommand::Present(effect) => return Ok(Some(effect)),
        }
        Ok(None)
    }

    pub fn animated(&self, now: u64) -> bool {
        !self.hidden && self.sprites.values().any(|sprite| sprite.animated(now))
    }

    pub fn images(&self) -> impl Iterator<Item = &ImageKey> {
        let background = match &self.background {
            Background::Image(image) => Some(&image.key),
            _ => None,
        };
        background
            .into_iter()
            .chain(self.sprites.values().map(|sprite| &sprite.image.key))
    }
}
