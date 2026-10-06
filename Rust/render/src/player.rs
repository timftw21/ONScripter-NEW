//! Shared native event loop; decoding work never runs on the presentation thread.

use std::{
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use onscripter_core::{
    Error, Limits, Result,
    assets::{Asset, AssetStore, ReadSeek, Storage},
    scene::{Effect, ImageKey, Scene},
    script::Program,
    text::{TextCommand, TextState},
    vm::{Event, Vm},
};
use sdl3::{
    event::{Event as NativeEvent, WindowEvent},
    keyboard::Keycode,
    mouse::MouseButton,
};

use crate::{
    Renderer,
    image::{DecodedImage, decode},
    native_error,
};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub size: Option<(u32, u32)>,
    pub hidden: bool,
    pub frames: Option<u64>,
    pub capture: Option<PathBuf>,
    pub nsa_offset: u64,
    pub font: Option<String>,
}

enum Waiting {
    Timer(u64),
    Click,
    Transition,
    Image { effect: Option<Effect>, line: u32 },
    Text,
    TextSignal(i32),
}

pub fn play<S: Storage>(
    program: &Program,
    mut assets: AssetStore<S>,
    options: Options,
) -> Result<()> {
    let limits = assets.limits;
    let size = match options.size {
        Some(size) => size,
        None => program.logical_size()?,
    };
    crate::image::pixel_bytes(size.0, size.1, limits)?;
    sdl3::hint::set("SDL_GPU_DRIVER", "vulkan");
    sdl3::hint::set("SDL_RENDER_VSYNC", "1");
    let sdl = sdl3::init().map_err(native_error)?;
    let video = sdl.video().map_err(native_error)?;
    let bounds = video
        .get_primary_display()
        .and_then(|display| display.get_usable_bounds())
        .map_err(native_error)?;
    let scale = ((bounds.width() as f64 * 0.9 / f64::from(size.0))
        .min(bounds.height() as f64 * 0.9 / f64::from(size.1)))
    .min(1.0);
    let window_size = (
        (f64::from(size.0) * scale).max(1.0) as u32,
        (f64::from(size.1) * scale).max(1.0) as u32,
    );
    let mut builder = video.window("onscripter-new — Rust", window_size.0, window_size.1);
    builder
        .position_centered()
        .vulkan()
        .resizable()
        .high_pixel_density();
    if options.hidden {
        builder.hidden();
    }
    #[cfg(target_os = "android")]
    builder.fullscreen();
    let window = builder.build().map_err(native_error)?;
    let canvas = sdl3::render::create_renderer(window, Some(c"gpu")).map_err(native_error)?;
    let creator = canvas.texture_creator();
    let mut renderer = Renderer::new(canvas, &creator, size, limits)?;
    let mut events = sdl.event_pump().map_err(native_error)?;
    let worker = ImageWorker::new(limits)?;
    let mut vm = Vm::new(program, limits);
    vm.set_archive_offset(options.nsa_offset);
    let mut scene = Scene::default();
    let mut text = TextState::new(size, options.font.clone());
    let mut text_revision = text.revision;
    let mut text_line = 0;
    let mut waiting = None;
    let mut loading: Option<ImageKey> = None;
    let mut pending_event = None;
    let mut backgrounded = false;
    let mut paused_at = None;
    let started = Instant::now();
    let mut paused = Duration::ZERO;
    let mut frame_deadline = started;
    let mut work_deadline = started;
    let mut budget = 0usize;
    let mut dirty = true;
    let mut finished = false;
    let mut captured = false;
    let mut frames = 0u64;
    println!(
        "Renderer: SDL3 GPU / Vulkan; {}×{} logical pixels",
        size.0, size.1
    );

    'running: loop {
        for event in pending_event.take().into_iter().chain(events.poll_iter()) {
            match event {
                NativeEvent::Quit { .. }
                | NativeEvent::Window {
                    win_event: WindowEvent::CloseRequested,
                    ..
                }
                | NativeEvent::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                NativeEvent::KeyDown {
                    keycode: Some(Keycode::Return | Keycode::Space),
                    repeat: false,
                    ..
                }
                | NativeEvent::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    ..
                }
                | NativeEvent::FingerUp { .. }
                    if text.busy() || matches!(waiting, Some(Waiting::Click)) =>
                {
                    if text
                        .input(elapsed_ms(started, paused, paused_at))
                        .map_err(|error| at_line(text_line, error))?
                    {
                        dirty = true;
                    } else if matches!(waiting, Some(Waiting::Click)) {
                        waiting = None;
                        vm.resume();
                    }
                }
                NativeEvent::AppWillEnterBackground { .. }
                | NativeEvent::Window {
                    win_event: WindowEvent::Minimized,
                    ..
                } => {
                    if !backgrounded {
                        paused_at = Some(Instant::now());
                    }
                    backgrounded = true;
                }
                NativeEvent::AppDidEnterForeground { .. }
                | NativeEvent::Window {
                    win_event: WindowEvent::Restored,
                    ..
                } => {
                    if let Some(start) = paused_at.take() {
                        paused += start.elapsed();
                    }
                    backgrounded = false;
                    dirty = true;
                    frame_deadline = Instant::now();
                }
                NativeEvent::Window {
                    win_event:
                        WindowEvent::Exposed
                        | WindowEvent::Resized(_, _)
                        | WindowEvent::PixelSizeChanged(_, _),
                    ..
                } => dirty = true,
                NativeEvent::RenderDeviceReset { .. } | NativeEvent::RenderTargetsReset { .. } => {
                    let now = elapsed_ms(started, paused, paused_at);
                    renderer.reset(&scene, now, &assets)?;
                    renderer
                        .prepare_text(&text, &scene, &assets)
                        .map_err(|error| at_line(text_line, error))?;
                    dirty = true;
                }
                _ => {}
            }
        }
        if backgrounded {
            pending_event = events.wait_event_timeout(Duration::from_millis(100));
            continue;
        }
        let now = elapsed_ms(started, paused, paused_at);
        dirty |= text
            .advance(now)
            .map_err(|error| at_line(text_line, error))?;
        if matches!(waiting, Some(Waiting::Text)) && !text.busy()
            || matches!(waiting, Some(Waiting::TextSignal(index)) if text.signal_ready(index))
        {
            waiting = None;
            vm.resume();
        }
        if Instant::now() >= frame_deadline {
            let update = renderer.update(now)?;
            dirty |= update.dirty;
            if update.transition_finished && matches!(waiting, Some(Waiting::Transition)) {
                waiting = None;
                vm.resume();
            }
        }
        if matches!(waiting, Some(Waiting::Timer(deadline)) if now >= deadline) {
            waiting = None;
            vm.resume();
        }
        if let Some(key) = loading.as_ref() {
            match worker.results.try_recv() {
                Ok(result) => {
                    let line = match &waiting {
                        Some(Waiting::Image { line, .. }) => *line,
                        _ => 0,
                    };
                    renderer
                        .upload(
                            key.clone(),
                            result.map_err(|error| at_line(line, error))?,
                            &scene,
                        )
                        .map_err(|error| at_line(line, error))?;
                    loading = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    return Err(Error::invalid("image decoder stopped unexpectedly"));
                }
            }
        }
        if loading.is_none() && matches!(waiting, Some(Waiting::Image { .. })) {
            let (effect, line) = match waiting.as_ref() {
                Some(Waiting::Image { effect, line }) => (*effect, *line),
                _ => (None, 0),
            };
            if let Some(key) = renderer.next_image(&scene) {
                let asset = assets
                    .open(&key.name)
                    .map_err(|error| at_line(line, error))?
                    .ok_or_else(|| {
                        at_line(
                            line,
                            Error::invalid(format!("image {} was not found", key.name)),
                        )
                    })?;
                worker
                    .jobs
                    .send((asset, key.clone()))
                    .map_err(|_| Error::invalid("image decoder stopped unexpectedly"))?;
                loading = Some(key);
            } else {
                waiting = None;
                if let Some(effect) = effect {
                    renderer
                        .commit(&scene, effect, now, &assets)
                        .map_err(|error| at_line(line, error))?;
                    dirty = true;
                    if effect.duration_ms != 0 {
                        waiting = Some(Waiting::Transition);
                    }
                }
                if waiting.is_none() {
                    vm.resume();
                }
            }
        }
        if Instant::now() >= work_deadline {
            budget = limits.instructions_per_tick;
            work_deadline = Instant::now() + Duration::from_nanos(16_666_667);
        }
        while waiting.is_none() && !finished && budget != 0 {
            match vm.run_with_budget(&mut budget)? {
                Event::Yield => break,
                Event::Finished => {
                    finished = true;
                    println!("Script finished.");
                }
                Event::Caption(caption) => renderer
                    .canvas
                    .window_mut()
                    .set_title(&caption)
                    .map_err(native_error)?,
                Event::Click => waiting = Some(Waiting::Click),
                Event::Wait(duration) => {
                    waiting = Some(Waiting::Timer(now.saturating_add(duration)))
                }
                Event::Archives { directory, offset } => {
                    assets.mount_archives(&directory, offset)?;
                    vm.resume();
                }
                Event::FileExists { variable, name } => {
                    vm.set_number(variable, i32::from(assets.open(&name)?.is_some()))?;
                    vm.resume();
                }
                Event::Scene { command, line } => {
                    let effect = scene
                        .apply(command, now, limits)
                        .map_err(|error| at_line(line, error))?;
                    waiting = Some(Waiting::Image { effect, line });
                }
                Event::Text { command, line } => {
                    text_line = line;
                    let signal = match command {
                        TextCommand::WaitDialogue(index) => Some(index),
                        _ => None,
                    };
                    let dialogue = text
                        .apply(command, now)
                        .map_err(|error| at_line(line, error))?;
                    dirty = true;
                    if dialogue {
                        waiting = Some(signal.map_or(Waiting::Text, Waiting::TextSignal));
                    } else {
                        vm.resume();
                    }
                }
                Event::Blocked => return Err(Error::invalid("unresolved script event")),
            }
        }
        if text.revision != text_revision {
            dirty = true;
            text_revision = text.revision;
        }
        renderer
            .prepare_text(&text, &scene, &assets)
            .map_err(|error| at_line(text_line, error))?;
        let frame_due = Instant::now() >= frame_deadline;
        let capture_ready = options.capture.is_some()
            && !captured
            && options.frames.is_none()
            && ((finished && !text.busy())
                || matches!(waiting, Some(Waiting::Click))
                || (text.waiting_click && !text.animating()));
        if frame_due
            && (dirty
                || renderer.animated()
                || text.animating()
                || options.frames.is_some()
                || capture_ready)
        {
            frame_deadline = Instant::now() + Duration::from_nanos(16_666_667);
            let last = options.frames.is_some_and(|limit| frames + 1 >= limit);
            let capture = if capture_ready || last {
                options.capture.as_deref()
            } else {
                None
            };
            renderer.present(now, capture)?;
            captured |= capture.is_some();
            frames += 1;
            dirty = false;
            if last {
                break;
            }
        }
        let active = !finished && waiting.is_none();
        let milliseconds = if active {
            if budget == 0 {
                work_deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .min(16) as u32
            } else {
                0
            }
        } else if renderer.animated() || text.animating() || options.frames.is_some() || dirty {
            frame_deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(16) as u32
        } else {
            100
        };
        let milliseconds = if matches!(waiting, Some(Waiting::Image { .. })) {
            milliseconds.min(if loading.is_some() { 5 } else { 1 })
        } else {
            milliseconds
        };
        let milliseconds = match waiting {
            Some(Waiting::Timer(deadline)) => {
                milliseconds.min(deadline.saturating_sub(now).min(100) as u32)
            }
            _ => milliseconds,
        };
        let milliseconds = text.wait_deadline().map_or(milliseconds, |deadline| {
            milliseconds.min(deadline.saturating_sub(now).min(100) as u32)
        });
        // SDL's wait wakes promptly for input; idle scenes do not render repeatedly.
        pending_event =
            events.wait_event_timeout(Duration::from_millis(u64::from(milliseconds.max(1))));
    }
    println!(
        "Rendered {frames} frames; {} visible sprites; {} scene draw calls; {} texture bytes",
        renderer.stats.sprites, renderer.stats.draw_calls, renderer.stats.texture_bytes
    );
    Ok(())
}

fn elapsed_ms(started: Instant, paused: Duration, paused_at: Option<Instant>) -> u64 {
    paused_at
        .unwrap_or_else(Instant::now)
        .saturating_duration_since(started)
        .saturating_sub(paused)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}
fn at_line(line: u32, error: Error) -> Error {
    Error::Script {
        line,
        message: error.to_string(),
    }
}

type DecodeJob = (Asset, ImageKey);

struct ImageWorker {
    jobs: SyncSender<DecodeJob>,
    results: Receiver<Result<DecodedImage>>,
    cancelled: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ImageWorker {
    fn new(limits: Limits) -> Result<Self> {
        let (jobs, incoming) = mpsc::sync_channel::<DecodeJob>(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let thread = thread::Builder::new()
            .name("image-decoder".into())
            .spawn(move || {
                while let Ok((mut asset, key)) = incoming.recv() {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    asset.reader = Box::new(CancellableReader {
                        inner: asset.reader,
                        cancelled: flag.clone(),
                    });
                    let result = decode(asset, &key, limits);
                    if flag.load(Ordering::Relaxed) || outgoing.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            jobs,
            results,
            cancelled,
            thread: Some(thread),
        })
    }
}

impl Drop for ImageWorker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

struct CancellableReader {
    inner: Box<dyn ReadSeek>,
    cancelled: Arc<AtomicBool>,
}
impl CancellableReader {
    fn check(&self) -> io::Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(io::Error::other("image load cancelled"))
        } else {
            Ok(())
        }
    }
}
impl Read for CancellableReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.read(bytes)
    }
}
impl Seek for CancellableReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check()?;
        self.inner.seek(position)
    }
}
