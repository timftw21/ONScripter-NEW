//! Unicode shaping and a fixed-size GPU glyph atlas. Pages are laid out once.
use crate::native_error;
use cosmic_text::{CacheKey, CacheKeyFlags, FontSystem, Weight, fontdb};
use onscripter_core::{
    Error, Result,
    assets::{AssetStore, Storage},
    text::{TextState, TextStyle, TextWindow},
};
use sdl3::{
    pixels::{Color, FColor, PixelFormat},
    rect::Rect,
    render::{BlendMode, ScaleMode, Texture, TextureCreator, Vertex, VertexIndices, WindowCanvas},
    video::WindowContext,
};
use std::{collections::HashMap, sync::Arc};
use swash::{
    scale::{Render, ScaleContext, Source},
    zeno::{Angle, Format, Join, Stroke, Transform, Vector},
};

mod layout;

const ATLAS_SIZE: u32 = 2048;
pub const ATLAS_BYTES: usize = ATLAS_SIZE as usize * ATLAS_SIZE as usize * 4;
const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FONT_MEMORY: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Glyph {
    rect: Rect,
    left: i32,
    top: i32,
    empty: bool,
}
struct Positioned {
    glyph: Glyph,
    border: Option<Glyph>,
    x: i32,
    y: i32,
    end: usize,
    style: TextStyle,
    scale: f32,
    underline: Option<[f32; 4]>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    key: CacheKey,
    border: u16,
}

pub struct TextRenderer<'a> {
    atlas: Texture<'a>,
    fonts: FontSystem,
    scale: ScaleContext,
    families: HashMap<u8, String>,
    font_bytes: usize,
    glyphs: HashMap<GlyphKey, Glyph>,
    positioned: Vec<Positioned>,
    vertices: Vec<Vertex>,
    pen: (u32, u32, u32),
    revision: Option<u64>,
    window: TextWindow,
    visible: bool,
    revealed: usize,
}

impl<'a> TextRenderer<'a> {
    pub fn new<S: Storage>(
        creator: &'a TextureCreator<WindowContext>,
        state: &TextState,
        assets: &AssetStore<S>,
    ) -> Result<Self> {
        let mut db = fontdb::Database::new();
        let (family, font_bytes) =
            load_font(assets, 0, state.font.as_deref(), &mut db, MAX_FONT_MEMORY)?;
        db.set_sans_serif_family(&family);
        let fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let mut atlas = creator
            .create_texture_static(PixelFormat::RGBA32, ATLAS_SIZE, ATLAS_SIZE)
            .map_err(native_error)?;
        atlas.set_blend_mode(BlendMode::Blend);
        atlas.set_scale_mode(ScaleMode::Linear);
        let mut renderer = Self {
            atlas,
            fonts,
            scale: ScaleContext::new(),
            families: [(0, family)].into_iter().collect(),
            font_bytes,
            glyphs: HashMap::new(),
            positioned: Vec::new(),
            vertices: Vec::new(),
            pen: (1, 1, 0),
            revision: None,
            window: state.window.clone(),
            visible: state.visible,
            revealed: state.revealed,
        };
        renderer.clear_atlas()?;
        Ok(renderer)
    }

    fn clear_atlas(&mut self) -> Result<()> {
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(ATLAS_BYTES)
            .map_err(|_| Error::Limit("glyph atlas allocation"))?;
        pixels.resize(ATLAS_BYTES, 0);
        self.atlas
            .update(None, &pixels, ATLAS_SIZE as usize * 4)
            .map_err(native_error)?;
        // One opaque texel lets text decorations join the same atlas batch.
        self.atlas
            .update(Rect::new(0, 0, 1, 1), &[255; 4], 4)
            .map_err(native_error)?;
        self.glyphs.clear();
        self.pen = (1, 1, 0);
        Ok(())
    }

    pub fn prepare<S: Storage>(&mut self, state: &TextState, assets: &AssetStore<S>) -> Result<()> {
        self.visible = state.visible;
        self.revealed = state.revealed;
        self.window = state.window.clone();
        if self.revision == Some(state.layout_revision) {
            return Ok(());
        }
        if state.content.is_empty() {
            self.positioned.clear();
            self.revision = Some(state.layout_revision);
            return Ok(());
        }
        for style in state
            .spans
            .iter()
            .map(|span| span.style)
            .chain(state.ruby.iter().map(|ruby| ruby.style))
        {
            if !self.families.contains_key(&style.font) {
                let (family, bytes) = load_font(
                    assets,
                    style.font,
                    None,
                    self.fonts.db_mut(),
                    MAX_FONT_MEMORY - self.font_bytes,
                )?;
                self.font_bytes += bytes;
                self.families.insert(style.font, family);
            }
        }
        let prepared = layout::prepare(state, &mut self.fonts, &self.families)?;
        // Repack only when accumulated glyphs fill the atlas, keeping the current page intact.
        for attempt in 0..2 {
            self.positioned.clear();
            let mut full = false;
            for positioned in &prepared {
                let cached = match self.glyph(positioned.key, positioned.blank, 0)? {
                    Some(glyph) => glyph,
                    None => {
                        full = true;
                        break;
                    }
                };
                let border = if positioned.style.border != 0 {
                    match self.glyph(positioned.key, positioned.blank, positioned.style.border)? {
                        Some(glyph) => Some(glyph),
                        None => {
                            full = true;
                            break;
                        }
                    }
                } else {
                    None
                };
                self.positioned
                    .try_reserve(1)
                    .map_err(|_| Error::Limit("glyph layout allocation"))?;
                self.positioned.push(Positioned {
                    glyph: cached,
                    border,
                    x: positioned.x,
                    y: positioned.y,
                    end: positioned.end,
                    style: positioned.style,
                    scale: positioned.scale,
                    underline: positioned.underline,
                });
            }
            if !full {
                self.revision = Some(state.layout_revision);
                return Ok(());
            }
            if attempt == 0 {
                self.clear_atlas()?;
            }
        }
        Err(Error::Limit("dialogue glyph atlas"))
    }

    fn glyph(&mut self, key: CacheKey, blank: bool, border: u16) -> Result<Option<Glyph>> {
        let cache_key = GlyphKey { key, border };
        if let Some(glyph) = self.glyphs.get(&cache_key) {
            return Ok(Some(*glyph));
        }
        if self.glyphs.len() >= 4096 {
            return Ok(None);
        }
        let font = self
            .fonts
            .get_font(key.font_id, key.font_weight)
            .ok_or_else(|| Error::invalid("dialogue font is unavailable"))?;
        let mut scaler = self
            .scale
            .builder(font.as_swash())
            .size(f32::from_bits(key.font_size_bits))
            .hint(true);
        let variable_weight = font
            .as_swash()
            .variations()
            .find_by_tag(swash::Tag::from_be_bytes(*b"wght"));
        if let Some(variation) = variable_weight {
            scaler = scaler.normalized_coords(font.as_swash().variations().normalized_coords([(
                swash::Tag::from_be_bytes(*b"wght"),
                f32::from(key.font_weight.0).clamp(variation.min_value(), variation.max_value()),
            )]));
        }
        let mut scaler = scaler.build();
        // Check outline bounds before the rasterizer allocates a bitmap. Embedded
        // color bitmaps are excluded from this initial scalable-font path.
        let transform = key
            .flags
            .contains(CacheKeyFlags::FAKE_ITALIC)
            .then(|| Transform::skew(Angle::from_degrees(14.0), Angle::from_degrees(0.0)));
        if let Some(mut outline) = scaler.scale_outline(key.glyph_id) {
            if let Some(transform) = &transform {
                outline.transform(transform);
            }
            let bounds = outline.bounds();
            if outline.points().len() > 100_000
                || outline.points().iter().any(|point| {
                    !point.x.is_finite()
                        || !point.y.is_finite()
                        || point.x.abs() > 16384.0
                        || point.y.abs() > 16384.0
                })
                || !bounds.min.x.is_finite()
                || !bounds.min.y.is_finite()
                || !bounds.max.x.is_finite()
                || !bounds.max.y.is_finite()
                || bounds.max.x - bounds.min.x + f32::from(border) / 32.0 > 1000.0
                || bounds.max.y - bounds.min.y + f32::from(border) / 32.0 > 1000.0
            {
                return Err(Error::Limit("glyph outline"));
            }
        }
        let mut render = Render::new(&[Source::Outline]);
        render
            .format(Format::Alpha)
            .embolden(
                if variable_weight.is_none()
                    && key.font_weight >= Weight::BOLD
                    && self
                        .fonts
                        .db()
                        .face(key.font_id)
                        .is_some_and(|face| face.weight < Weight::BOLD)
                {
                    f32::from_bits(key.font_size_bits) / 32.0
                } else {
                    0.0
                },
            )
            .offset(Vector::new(key.x_bin.as_float(), key.y_bin.as_float()))
            .transform(transform);
        if border != 0 {
            let mut stroke = Stroke::new(f32::from(border) / 32.0);
            stroke.join(Join::Round);
            render.style(swash::zeno::Style::Stroke(stroke));
        }
        let image = render.render(&mut scaler, key.glyph_id);
        let Some(image) = image else {
            if !blank {
                return Err(Error::invalid(
                    "game font glyph has no supported scalable outline",
                ));
            }
            let glyph = Glyph {
                rect: Rect::new(1, 0, 1, 1),
                left: 0,
                top: 0,
                empty: true,
            };
            self.glyphs.insert(cache_key, glyph);
            return Ok(Some(glyph));
        };
        let (width, height) = (image.placement.width, image.placement.height);
        if width > 1024 || height > 1024 || image.data.len() != width as usize * height as usize {
            return Err(Error::Limit("glyph bitmap"));
        }
        if self.pen.0 + width + 1 >= ATLAS_SIZE {
            self.pen.0 = 1;
            self.pen.1 += self.pen.2 + 1;
            self.pen.2 = 0;
        }
        if self.pen.1 + height + 1 >= ATLAS_SIZE || self.glyphs.len() >= 4096 {
            return Ok(None);
        }
        let rect = Rect::new(
            self.pen.0 as i32,
            self.pen.1 as i32,
            width.max(1),
            height.max(1),
        );
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(image.data.len() * 4)
            .map_err(|_| Error::Limit("glyph upload allocation"))?;
        for alpha in image.data {
            rgba.extend_from_slice(&[255, 255, 255, alpha]);
        }
        if width != 0 && height != 0 {
            self.atlas
                .update(rect, &rgba, width as usize * 4)
                .map_err(native_error)?;
        }
        let glyph = Glyph {
            rect,
            left: image.placement.left,
            top: image.placement.top,
            empty: width == 0 || height == 0,
        };
        self.glyphs
            .try_reserve(1)
            .map_err(|_| Error::Limit("glyph cache allocation"))?;
        self.glyphs.insert(cache_key, glyph);
        self.pen.0 += width + 1;
        self.pen.2 = self.pen.2.max(height);
        Ok(Some(glyph))
    }

    pub fn draw(&mut self, canvas: &mut WindowCanvas) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        let (x, y, width, height) = self.window.bounds;
        let clip = canvas.clip_rect();
        canvas.set_clip_rect(Rect::new(x, y, width, height));
        let result = self.draw_clipped(canvas);
        canvas.set_clip_rect(clip);
        result
    }

    fn draw_clipped(&mut self, canvas: &mut WindowCanvas) -> Result<()> {
        let (x, y, width, height) = self.window.bounds;
        canvas.set_blend_mode(BlendMode::Mul);
        canvas.set_draw_color(Color::RGB(
            self.window.color[0],
            self.window.color[1],
            self.window.color[2],
        ));
        canvas
            .fill_rect(Rect::new(x, y, width, height))
            .map_err(native_error)?;
        canvas.set_blend_mode(BlendMode::None);
        for layer in 0..3 {
            self.vertices.clear();
            for positioned in &self.positioned {
                if positioned.end > self.revealed {
                    continue;
                }
                let style = positioned.style;
                let (color, offset) = match layer {
                    0 if style.shadow != (0, 0) => (style.shadow_color, style.shadow),
                    0 => continue,
                    1 if positioned.border.is_some() => (style.border_color, (0, 0)),
                    1 => continue,
                    _ => (style.color, (0, 0)),
                };
                if layer != 1 {
                    append_glyph(
                        &mut self.vertices,
                        positioned,
                        positioned.glyph,
                        offset,
                        color,
                    )?;
                }
                if layer != 2
                    && let Some(border) = positioned.border
                {
                    append_glyph(&mut self.vertices, positioned, border, offset, color)?;
                }
                if layer != 1
                    && let Some(mut rect) = positioned.underline
                {
                    rect[0] += offset.0 as f32 * positioned.scale;
                    rect[1] += offset.1 as f32;
                    append_quad(
                        &mut self.vertices,
                        rect,
                        [0.5 / ATLAS_SIZE as f32; 4],
                        color,
                    )?;
                }
            }
            if !self.vertices.is_empty() {
                canvas
                    .render_geometry(&self.vertices, Some(&self.atlas), VertexIndices::Sequential)
                    .map_err(native_error)?;
            }
        }
        Ok(())
    }
}

fn append_glyph(
    vertices: &mut Vec<Vertex>,
    positioned: &Positioned,
    glyph: Glyph,
    offset: (i32, i32),
    color: [u8; 3],
) -> Result<()> {
    if glyph.empty {
        return Ok(());
    }
    append_quad(
        vertices,
        [
            positioned.x as f32 + (glyph.left + offset.0) as f32 * positioned.scale,
            positioned.y as f32 - glyph.top as f32 + offset.1 as f32,
            glyph.rect.width() as f32 * positioned.scale,
            glyph.rect.height() as f32,
        ],
        [
            glyph.rect.x() as f32 / ATLAS_SIZE as f32,
            glyph.rect.y() as f32 / ATLAS_SIZE as f32,
            (glyph.rect.x() + glyph.rect.width() as i32) as f32 / ATLAS_SIZE as f32,
            (glyph.rect.y() + glyph.rect.height() as i32) as f32 / ATLAS_SIZE as f32,
        ],
        color,
    )
}

fn append_quad(
    vertices: &mut Vec<Vertex>,
    rect: [f32; 4],
    uv: [f32; 4],
    color: [u8; 3],
) -> Result<()> {
    vertices
        .try_reserve(6)
        .map_err(|_| Error::Limit("text geometry allocation"))?;
    let first = vertices.len();
    crate::quad(vertices, rect, uv, 255);
    for vertex in &mut vertices[first..] {
        vertex.color = FColor::RGBA(
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0,
            1.0,
        );
    }
    Ok(())
}

fn load_font<S: Storage>(
    assets: &AssetStore<S>,
    number: u8,
    override_name: Option<&str>,
    db: &mut fontdb::Database,
    budget: usize,
) -> Result<(String, usize)> {
    let base = if number == 0 {
        "default".to_owned()
    } else {
        format!("font{number}")
    };
    let candidates = override_name.map_or_else(
        || vec![format!("fonts/{base}.otf"), format!("fonts/{base}.ttf")],
        |name| vec![name.to_owned()],
    );
    let mut data = None;
    for name in candidates {
        if let Some(asset) = assets.open(&name)? {
            data = Some(asset.read(MAX_FONT_BYTES.min(budget))?);
            break;
        }
    }
    let data = data.ok_or_else(|| {
        Error::invalid(format!(
            "game font was not found: {base}; supply its fonts/*.otf or fonts/*.ttf asset"
        ))
    })?;
    if data.starts_with(b"ttcf") {
        let count = data
            .get(8..12)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .ok_or_else(|| Error::invalid("truncated font collection"))?;
        if count == 0 || count > 256 || 12 + count as usize * 4 > data.len() {
            return Err(Error::Limit("font collection faces"));
        }
    }
    let bytes = data.len();
    let family = format!("onscripter-font-{number}");
    let faces = db.load_font_source(fontdb::Source::Binary(Arc::new(data)));
    if faces.is_empty() {
        return Err(Error::invalid("game font contains no usable font face"));
    }
    // Distinct game slots can contain fonts with identical family names.
    // Give each slot its own family so shaping always selects the requested asset.
    for id in faces {
        let mut face = db
            .face(id)
            .cloned()
            .ok_or_else(|| Error::invalid("game font face is unavailable"))?;
        db.remove_face(id);
        for (name, _) in &mut face.families {
            name.clone_from(&family);
        }
        db.push_face_info(face);
    }
    Ok((family, bytes))
}
