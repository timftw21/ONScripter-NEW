//! SDL3 GPU rendering and bounded image decoding.

pub mod image;
pub mod player;
pub mod text;

use std::{collections::HashMap, path::Path};

use onscripter_core::{
    Error, Limits, Result,
    assets::{AssetStore, Storage},
    scene::{Background, Blend, Effect, ImageKey, Scene},
    text::TextState,
};
use sdl3::{
    pixels::{Color, FColor, PixelFormat},
    render::{
        BlendMode, FPoint, ScaleMode, Texture, TextureCreator, Vertex, VertexIndices, WindowCanvas,
    },
    video::WindowContext,
};

struct CachedTexture<'a> {
    texture: Texture<'a>,
    width: u32,
    height: u32,
    bytes: usize,
    used: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub draw_calls: usize,
    pub sprites: usize,
    pub texture_bytes: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Update {
    pub dirty: bool,
    pub transition_finished: bool,
}

/// SDL's GPU renderer supplies the GPU pipeline and reusable upload buffers.
/// Only adjacent sprites using the same texture are batched, preserving order.
pub struct Renderer<'a> {
    pub canvas: WindowCanvas,
    creator: &'a TextureCreator<WindowContext>,
    cache: HashMap<ImageKey, CachedTexture<'a>>,
    front: Texture<'a>,
    back: Texture<'a>,
    visible: Scene,
    incoming: Option<Scene>,
    transition: Option<(u64, u32)>,
    front_animating: bool,
    back_animating: bool,
    limits: Limits,
    size: (u32, u32),
    resident_bytes: usize,
    target_bytes: usize,
    serial: u64,
    vertices: Vec<Vertex>,
    text: Option<text::TextRenderer<'a>>,
    pub stats: FrameStats,
}

impl<'a> Renderer<'a> {
    pub fn new(
        mut canvas: WindowCanvas,
        creator: &'a TextureCreator<WindowContext>,
        size: (u32, u32),
        limits: Limits,
    ) -> Result<Self> {
        if canvas.renderer_name != "gpu" {
            return Err(Error::invalid("SDL3 GPU renderer is required"));
        }
        let target_bytes = image::pixel_bytes(size.0, size.1, limits)?
            .checked_mul(2)
            .filter(|&size| size < limits.texture_bytes)
            .ok_or(Error::Limit("render targets"))?;
        canvas
            .set_logical_size(
                size.0,
                size.1,
                sdl3_sys::render::SDL_LOGICAL_PRESENTATION_LETTERBOX,
            )
            .map_err(native_error)?;
        let mut front = make_target(creator, size)?;
        let mut back = make_target(creator, size)?;
        front.set_blend_mode(BlendMode::None);
        back.set_blend_mode(BlendMode::Blend);
        for target in [&mut front, &mut back] {
            canvas
                .with_texture_canvas(target, |canvas| {
                    canvas.set_draw_color(Color::RGB(0, 0, 0));
                    canvas.clear();
                })
                .map_err(native_error)?;
        }
        let mut vertices = Vec::new();
        vertices
            .try_reserve(6 * (limits.sprites as usize + 1))
            .map_err(|_| Error::Limit("sprite geometry allocation"))?;
        Ok(Self {
            canvas,
            creator,
            cache: HashMap::new(),
            front,
            back,
            visible: Scene::default(),
            incoming: None,
            transition: None,
            front_animating: false,
            back_animating: false,
            limits,
            size,
            resident_bytes: 0,
            target_bytes,
            serial: 0,
            vertices,
            text: None,
            stats: FrameStats::default(),
        })
    }

    pub fn prepare<S: Storage>(&mut self, scene: &Scene, assets: &AssetStore<S>) -> Result<()> {
        self.serial = self.serial.wrapping_add(1);
        for key in scene.images() {
            if let Some(cached) = self.cache.get_mut(key) {
                cached.used = self.serial;
                continue;
            }
            let asset = assets
                .open(&key.name)?
                .ok_or_else(|| Error::invalid(format!("image {} was not found", key.name)))?;
            let decoded = image::decode(asset, key, self.limits)?;
            self.upload(key.clone(), decoded, scene)?;
        }
        Ok(())
    }

    pub fn next_image(&self, scene: &Scene) -> Option<ImageKey> {
        scene
            .images()
            .find(|key| !self.cache.contains_key(*key))
            .cloned()
    }

    fn make_room(&mut self, bytes: usize, scene: &Scene) -> Result<()> {
        let available = self.limits.texture_bytes - self.target_bytes - self.text_bytes();
        if bytes > available {
            return Err(Error::Limit("GPU textures"));
        }
        while self.resident_bytes > available - bytes {
            let victim = self
                .cache
                .iter()
                .filter(|(key, _)| {
                    !scene.images().any(|image| image == *key)
                        && !(self.front_animating
                            && self.visible.images().any(|image| image == *key))
                        && !(self.back_animating
                            && self.incoming.as_ref().is_some_and(|incoming| {
                                incoming.images().any(|image| image == *key)
                            }))
                })
                .min_by_key(|(_, texture)| texture.used)
                .map(|(key, _)| key.clone());
            let Some(victim) = victim else {
                return Err(Error::Limit("active GPU textures"));
            };
            if let Some(texture) = self.cache.remove(&victim) {
                self.resident_bytes -= texture.bytes;
            }
        }
        Ok(())
    }

    pub fn upload(
        &mut self,
        key: ImageKey,
        decoded: image::DecodedImage,
        scene: &Scene,
    ) -> Result<()> {
        let bytes = image::pixel_bytes(decoded.width, decoded.height, self.limits)?;
        if decoded.pixels.len() != bytes {
            return Err(Error::invalid(
                "decoded image byte count differs from its dimensions",
            ));
        }
        self.serial = self.serial.wrapping_add(1);
        self.make_room(bytes, scene)?;
        let mut texture = self
            .creator
            .create_texture_static(PixelFormat::RGBA32, decoded.width, decoded.height)
            .map_err(native_error)?;
        texture
            .update(None, &decoded.pixels, decoded.width as usize * 4)
            .map_err(native_error)?;
        texture.set_scale_mode(ScaleMode::Linear);
        texture.set_blend_mode(match key.blend {
            Blend::Normal => BlendMode::Blend,
            Blend::Add => BlendMode::Add,
            Blend::Multiply => BlendMode::Mul,
        });
        self.cache
            .try_reserve(1)
            .map_err(|_| Error::Limit("texture cache allocation"))?;
        self.cache.insert(
            key.clone(),
            CachedTexture {
                texture,
                width: decoded.width,
                height: decoded.height,
                bytes,
                used: self.serial,
            },
        );
        self.resident_bytes += bytes;
        Ok(())
    }

    pub fn commit<S: Storage>(
        &mut self,
        scene: &Scene,
        effect: Effect,
        now: u64,
        assets: &AssetStore<S>,
    ) -> Result<()> {
        if self.transition.is_some() {
            return Err(Error::invalid("previous scene transition is still running"));
        }
        self.prepare(scene, assets)?;
        let mut result = Ok(FrameStats::default());
        self.canvas
            .with_texture_canvas(&mut self.back, |canvas| {
                result = draw_scene(
                    canvas,
                    scene,
                    &self.cache,
                    self.size,
                    now,
                    &mut self.vertices,
                );
            })
            .map_err(native_error)?;
        self.stats = result?;
        if effect.duration_ms == 0 {
            std::mem::swap(&mut self.front, &mut self.back);
            self.front.set_blend_mode(BlendMode::None);
            self.back.set_blend_mode(BlendMode::Blend);
            self.visible = scene.clone();
            self.front_animating = scene.animated(now);
        } else {
            self.incoming = Some(scene.clone());
            self.transition = Some((now, effect.duration_ms));
            self.back_animating = scene.animated(now);
        }
        Ok(())
    }

    /// Draw the final animation cell once, then allow the host to sleep.
    pub fn update(&mut self, now: u64) -> Result<Update> {
        let completed = self
            .transition
            .is_some_and(|(start, duration)| now.saturating_sub(start) >= u64::from(duration));
        if completed {
            std::mem::swap(&mut self.front, &mut self.back);
            self.front.set_blend_mode(BlendMode::None);
            self.back.set_blend_mode(BlendMode::Blend);
            self.front.set_alpha_mod(255);
            if let Some(scene) = self.incoming.take() {
                self.visible = scene;
            }
            self.transition = None;
            self.front_animating = self.back_animating;
            self.back_animating = false;
        }
        let mut dirty = completed;
        if self.front_animating {
            let mut result = Ok(FrameStats::default());
            self.canvas
                .with_texture_canvas(&mut self.front, |canvas| {
                    result = draw_scene(
                        canvas,
                        &self.visible,
                        &self.cache,
                        self.size,
                        now,
                        &mut self.vertices,
                    );
                })
                .map_err(native_error)?;
            self.stats = result?;
            self.front_animating = self.visible.animated(now);
            dirty = true;
        }
        if let Some(scene) = &self.incoming
            && self.back_animating
        {
            let mut result = Ok(FrameStats::default());
            self.canvas
                .with_texture_canvas(&mut self.back, |canvas| {
                    result = draw_scene(
                        canvas,
                        scene,
                        &self.cache,
                        self.size,
                        now,
                        &mut self.vertices,
                    );
                })
                .map_err(native_error)?;
            self.stats = result?;
            self.back_animating = scene.animated(now);
            dirty = true;
        }
        Ok(Update {
            dirty,
            transition_finished: completed,
        })
    }

    pub fn animated(&self) -> bool {
        self.transition.is_some() || self.front_animating || self.back_animating
    }

    fn text_bytes(&self) -> usize {
        if self.text.is_some() {
            text::ATLAS_BYTES
        } else {
            0
        }
    }

    pub fn prepare_text<S: Storage>(
        &mut self,
        state: &TextState,
        scene: &Scene,
        assets: &AssetStore<S>,
    ) -> Result<()> {
        if self.text.is_none() && state.visible {
            self.make_room(text::ATLAS_BYTES, scene)?;
            self.text = Some(text::TextRenderer::new(self.creator, state, assets)?);
        }
        if let Some(text) = &mut self.text {
            text.prepare(state, assets)?;
        }
        Ok(())
    }

    pub fn present(&mut self, now: u64, capture: Option<&Path>) -> Result<()> {
        if let Some((start, duration)) = self.transition {
            let alpha = (now.saturating_sub(start).min(u64::from(duration)) * 255
                / u64::from(duration)) as u8;
            self.back.set_alpha_mod(alpha);
        }
        if let Some(path) = capture {
            self.capture(path)?;
        }
        compose_frame(
            &mut self.canvas,
            &self.front,
            self.transition.map(|_| &self.back),
        )?;
        if let Some(text) = &mut self.text {
            text.draw(&mut self.canvas)?;
        }
        if !self.canvas.present() {
            return Err(native_error(sdl3::get_error()));
        }
        self.stats.texture_bytes = self.resident_bytes + self.target_bytes + self.text_bytes();
        Ok(())
    }

    /// Read the logical canvas, whose GPU dimensions remain stable across window resizes.
    /// The extra target and readback exist only for an explicit developer capture.
    fn capture(&mut self, path: &Path) -> Result<()> {
        let (width, height) = self.size;
        let bytes = image::pixel_bytes(width, height, self.limits)?;
        if bytes
            > self.limits.texture_bytes
                - self.resident_bytes
                - self.target_bytes
                - self.text_bytes()
        {
            return Err(Error::Limit("capture GPU target"));
        }
        let mut target = make_target(self.creator, self.size)?;
        let mut packed = Err(Error::invalid("capture was not rendered"));
        self.canvas
            .with_texture_canvas(&mut target, |canvas| {
                packed = compose_frame(canvas, &self.front, self.transition.map(|_| &self.back))
                    .and_then(|_| {
                        if let Some(text) = &mut self.text {
                            text.draw(canvas)?;
                        }
                        capture_pixels(canvas, self.size, self.limits)
                    });
            })
            .map_err(native_error)?;
        ::image::save_buffer_with_format(
            path,
            &packed?,
            width,
            height,
            ::image::ColorType::Rgba8,
            ::image::ImageFormat::Png,
        )
        .map_err(native_error)
    }

    pub fn reset<S: Storage>(
        &mut self,
        scene: &Scene,
        now: u64,
        assets: &AssetStore<S>,
    ) -> Result<()> {
        self.cache.clear();
        self.resident_bytes = 0;
        self.text = None;
        self.front = make_target(self.creator, self.size)?;
        self.back = make_target(self.creator, self.size)?;
        self.front.set_blend_mode(BlendMode::None);
        self.back.set_blend_mode(BlendMode::Blend);
        let visible = self.visible.clone();
        self.prepare(&visible, assets)?;
        let mut result = Ok(FrameStats::default());
        self.canvas
            .with_texture_canvas(&mut self.front, |canvas| {
                result = draw_scene(
                    canvas,
                    &visible,
                    &self.cache,
                    self.size,
                    now,
                    &mut self.vertices,
                );
            })
            .map_err(native_error)?;
        self.stats = result?;
        if let Some(incoming) = self.incoming.clone() {
            self.prepare(&incoming, assets)?;
            let mut result = Ok(FrameStats::default());
            self.canvas
                .with_texture_canvas(&mut self.back, |canvas| {
                    result = draw_scene(
                        canvas,
                        &incoming,
                        &self.cache,
                        self.size,
                        now,
                        &mut self.vertices,
                    );
                })
                .map_err(native_error)?;
            self.stats = result?;
        }
        self.prepare(scene, assets)
    }
}

fn compose_frame(
    canvas: &mut WindowCanvas,
    front: &Texture<'_>,
    back: Option<&Texture<'_>>,
) -> Result<()> {
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    canvas.copy(front, None, None).map_err(native_error)?;
    if let Some(back) = back {
        canvas.copy(back, None, None).map_err(native_error)?;
    }
    Ok(())
}

fn capture_pixels(canvas: &mut WindowCanvas, size: (u32, u32), limits: Limits) -> Result<Vec<u8>> {
    let surface = canvas
        .read_pixels(None)
        .map_err(native_error)?
        .convert_format(PixelFormat::RGBA32)
        .map_err(native_error)?;
    if (surface.width(), surface.height()) != size {
        return Err(Error::invalid(
            "capture dimensions differ from the logical canvas",
        ));
    }
    let bytes = image::pixel_bytes(size.0, size.1, limits)?;
    if (surface.pitch() as usize) < size.0 as usize * 4 {
        return Err(Error::invalid("capture row is shorter than its width"));
    }
    let mut packed = Vec::new();
    packed
        .try_reserve_exact(bytes)
        .map_err(|_| Error::Limit("capture allocation"))?;
    surface.with_lock(|pixels| {
        for row in pixels
            .chunks_exact(surface.pitch() as usize)
            .take(size.1 as usize)
        {
            packed.extend_from_slice(&row[..size.0 as usize * 4]);
        }
    });
    Ok(packed)
}

fn make_target(creator: &TextureCreator<WindowContext>, size: (u32, u32)) -> Result<Texture<'_>> {
    let mut texture = creator
        .create_texture_target(PixelFormat::RGBA32, size.0, size.1)
        .map_err(native_error)?;
    texture.set_scale_mode(ScaleMode::Linear);
    Ok(texture)
}

fn draw_scene(
    canvas: &mut WindowCanvas,
    scene: &Scene,
    cache: &HashMap<ImageKey, CachedTexture<'_>>,
    size: (u32, u32),
    now: u64,
    vertices: &mut Vec<Vertex>,
) -> Result<FrameStats> {
    let color = match scene.background() {
        Background::Color(color) => *color,
        _ => [0; 3],
    };
    canvas.set_draw_color(Color::RGB(color[0], color[1], color[2]));
    canvas.clear();
    let mut stats = FrameStats::default();
    if let Background::Image(image) = scene.background() {
        let texture = cache
            .get(&image.key)
            .ok_or_else(|| Error::invalid("background texture is missing"))?;
        vertices.clear();
        quad(
            vertices,
            [
                ((i64::from(size.0) - i64::from(texture.width)) / 2) as f32,
                ((i64::from(size.1) - i64::from(texture.height)) / 2) as f32,
                texture.width as f32,
                texture.height as f32,
            ],
            [0.0, 0.0, 1.0, 1.0],
            255,
        );
        canvas
            .render_geometry(vertices, Some(&texture.texture), VertexIndices::Sequential)
            .map_err(native_error)?;
        stats.draw_calls += 1;
    }
    vertices.clear();
    let mut batch: Option<&ImageKey> = None;
    for sprite in scene
        .sprites()
        .rev()
        .filter(|sprite| !scene.hidden() && sprite.visible && sprite.opacity != 0)
    {
        let key = &sprite.image.key;
        let texture = cache
            .get(key)
            .ok_or_else(|| Error::invalid("sprite texture is missing"))?;
        let width = texture.width / if key.vertical { 1 } else { key.cells };
        let height = texture.height / if key.vertical { key.cells } else { 1 };
        let x = sprite.x as f32;
        let y = sprite.y as f32;
        if x + width as f32 <= 0.0
            || y + height as f32 <= 0.0
            || x >= size.0 as f32
            || y >= size.1 as f32
        {
            continue;
        }
        if batch.is_some_and(|previous| previous != key) {
            flush(canvas, cache, batch, vertices, &mut stats)?;
        }
        batch = Some(key);
        let cell = sprite.cell_at(now) as f32;
        let step = 1.0 / key.cells as f32;
        let uv = if key.vertical {
            [0.0, cell * step, 1.0, (cell + 1.0) * step]
        } else {
            [cell * step, 0.0, (cell + 1.0) * step, 1.0]
        };
        quad(
            vertices,
            [x, y, width as f32, height as f32],
            uv,
            sprite.opacity,
        );
        stats.sprites += 1;
    }
    flush(canvas, cache, batch, vertices, &mut stats)?;
    Ok(stats)
}

fn flush(
    canvas: &mut WindowCanvas,
    cache: &HashMap<ImageKey, CachedTexture<'_>>,
    key: Option<&ImageKey>,
    vertices: &mut Vec<Vertex>,
    stats: &mut FrameStats,
) -> Result<()> {
    if let Some(key) = key.filter(|_| !vertices.is_empty()) {
        let texture = cache
            .get(key)
            .ok_or_else(|| Error::invalid("batch texture is missing"))?;
        canvas
            .render_geometry(vertices, Some(&texture.texture), VertexIndices::Sequential)
            .map_err(native_error)?;
        stats.draw_calls += 1;
    }
    vertices.clear();
    Ok(())
}

fn quad(vertices: &mut Vec<Vertex>, rect: [f32; 4], uv: [f32; 4], opacity: u8) {
    let [x, y, width, height] = rect;
    let [u0, v0, u1, v1] = uv;
    let color = FColor::RGBA(1.0, 1.0, 1.0, f32::from(opacity) / 255.0);
    let points = [
        (x, y, u0, v0),
        (x + width, y, u1, v0),
        (x + width, y + height, u1, v1),
        (x, y + height, u0, v1),
    ];
    for corner in [0, 1, 2, 0, 2, 3] {
        let (x, y, u, v) = points[corner];
        vertices.push(Vertex {
            position: FPoint::new(x, y),
            color,
            tex_coord: FPoint::new(u, v),
        });
    }
}

pub fn native_error(error: impl std::fmt::Display) -> Error {
    Error::invalid(format!("SDL3 rendering: {error}"))
}
