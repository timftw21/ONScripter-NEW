use std::{
    env,
    io::{self, Read, Write},
    path::PathBuf,
    process::ExitCode,
    time::Instant,
};

use onscripter_core::{
    Error, Limits, Result,
    archive::{ArchiveIndex, ArchiveKind},
    assets::{AssetStore, Storage},
    script::{Program, ScriptSource},
    vm::{Event, Vm},
};
use onscripter_platform::FileStorage;

const HELP: &str = "Rust rewrite development runner (audio and full game compatibility are pending)

Usage:
  onscripter-new-rust inspect <game-directory|script> [options]
  onscripter-new-rust run <game-directory|script> [options]
  onscripter-new-rust play <game-directory|script> [options]
  onscripter-new-rust archive <archive> [--nsa-offset <bytes>]

Options:
  --script <name>       Select a script (default: script.file, then 0.txt)
  --overlay <folder>    Add a maintained loose-asset root; repeat in priority order
  --asset <name>        Inspect the source and first bytes of an asset
  --nsa-offset <bytes>  Account for the ns2/ns3 NSA header prefix
  --ticks <count>       Bound headless execution (default: 1000)
  --size <width>x<height>  Override the script's declared canvas size for play
  --frames <count>      Close play after this many presented frames
  --capture <png>       Capture the final frame, or the first click/end in play
  --hidden             Keep the play window hidden for automated verification
  --font <asset>       Use a game-relative font asset instead of fonts/default.ttf/otf

inspect loads and indexes data without executing scripts.
run supports core arithmetic, variables, branches, subroutines and asset queries.
play adds backgrounds, sprites, cells, animation and instant/crossfade presentation.
Unsupported commands stop with their source line; they are never skipped.";

struct Options {
    action: String,
    input: PathBuf,
    script: Option<String>,
    overlays: Vec<PathBuf>,
    asset: Option<String>,
    nsa_offset: u64,
    ticks: usize,
    playback: onscripter_render::player::Options,
}

fn options() -> Result<Option<Options>> {
    let mut args = env::args_os().skip(1);
    let Some(action) = args.next() else {
        println!("{HELP}");
        return Ok(None);
    };
    if action == "--help" || action == "-h" {
        println!("{HELP}");
        return Ok(None);
    }
    let action = action
        .into_string()
        .map_err(|_| Error::invalid("invalid action"))?;
    if !["inspect", "run", "play", "archive"].contains(&action.as_str()) {
        return Err(Error::invalid(
            "expected inspect, run, play, or archive; use --help",
        ));
    }
    let input = PathBuf::from(
        args.next()
            .ok_or_else(|| Error::invalid("missing game directory/script/archive; use --help"))?,
    );
    let mut options = Options {
        action,
        input,
        script: None,
        overlays: Vec::new(),
        asset: None,
        nsa_offset: 0,
        ticks: 1000,
        playback: onscripter_render::player::Options::default(),
    };
    while let Some(flag) = args.next() {
        let flag = flag
            .into_string()
            .map_err(|_| Error::invalid("invalid option name"))?;
        if flag == "--hidden" {
            options.playback.hidden = true;
            continue;
        }
        if ![
            "--script",
            "--overlay",
            "--asset",
            "--nsa-offset",
            "--ticks",
            "--size",
            "--frames",
            "--capture",
            "--font",
        ]
        .contains(&flag.as_str())
        {
            return Err(Error::invalid(format!("unknown option {flag}")));
        }
        let value = args
            .next()
            .ok_or_else(|| Error::invalid(format!("missing value for {flag}")))?;
        if flag == "--overlay" {
            options.overlays.push(PathBuf::from(value));
            continue;
        }
        if flag == "--capture" {
            options.playback.capture = Some(PathBuf::from(value));
            continue;
        }
        let value = value
            .into_string()
            .map_err(|_| Error::invalid("option value must be UTF-8"))?;
        match flag.as_str() {
            "--script" => options.script = Some(value),
            "--asset" => options.asset = Some(value),
            "--font" => options.playback.font = Some(value),
            "--nsa-offset" => {
                options.nsa_offset = value
                    .parse()
                    .map_err(|_| Error::invalid("invalid NSA offset"))?
            }
            "--ticks" => {
                options.ticks = value
                    .parse()
                    .map_err(|_| Error::invalid("invalid tick count"))?;
                if options.ticks == 0 {
                    return Err(Error::invalid("tick count must be positive"));
                }
            }
            "--size" => {
                let (width, height) = value
                    .split_once('x')
                    .ok_or_else(|| Error::invalid("size must be WIDTHxHEIGHT"))?;
                options.playback.size = Some((
                    width
                        .parse()
                        .map_err(|_| Error::invalid("invalid canvas width"))?,
                    height
                        .parse()
                        .map_err(|_| Error::invalid("invalid canvas height"))?,
                ));
            }
            "--frames" => {
                let frames = value
                    .parse::<u64>()
                    .map_err(|_| Error::invalid("invalid frame count"))?;
                if frames == 0 {
                    return Err(Error::invalid("frame count must be positive"));
                }
                options.playback.frames = Some(frames);
            }
            _ => unreachable!(),
        }
    }
    if options.action != "play"
        && (options.playback.hidden
            || options.playback.frames.is_some()
            || options.playback.capture.is_some()
            || options.playback.size.is_some()
            || options.playback.font.is_some())
    {
        return Err(Error::invalid("rendering options require play"));
    }
    Ok(Some(options))
}

fn run(options: Options) -> Result<()> {
    let limits = Limits::default();
    let start = Instant::now();
    if options.action == "archive" {
        let kind = match options
            .input
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("sar") => ArchiveKind::Sar,
            Some("nsa") => ArchiveKind::Nsa,
            Some("ns2") => ArchiveKind::Ns2,
            _ => {
                return Err(Error::invalid(
                    "archive extension must be .sar, .nsa, or .ns2",
                ));
            }
        };
        let mut file = std::fs::File::open(&options.input)?;
        let index = ArchiveIndex::read(&mut file, kind, options.nsa_offset, limits)?;
        println!(
            "{kind:?}: {} entries; {} archive bytes; {} index budget bytes; {:.2} ms",
            index.len(),
            index.archive_bytes,
            index.index_bytes,
            start.elapsed().as_secs_f64() * 1000.0
        );
        return Ok(());
    }
    let metadata = std::fs::metadata(&options.input)?;
    let (root, input_script) = if metadata.is_dir() {
        (options.input.clone(), None)
    } else {
        let name = options
            .input
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| Error::invalid("script filename must be UTF-8"))?
            .to_owned();
        (
            options
                .input
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."))
                .to_owned(),
            Some(name),
        )
    };
    let mut roots = options.overlays;
    roots.push(root);
    let storage = FileStorage::new(&roots)?;
    let script_name = match options.script.or(input_script) {
        Some(name) => name,
        None if storage.open("script.file")?.is_some() => "script.file".into(),
        None => "0.txt".into(),
    };
    let mut reader = storage
        .open(&script_name)?
        .ok_or_else(|| Error::invalid(format!("script {script_name} was not found")))?;
    let source = ScriptSource::read(&mut reader, limits)?;
    let program = Program::parse(source, limits)?;
    println!(
        "{script_name}: {} source bytes; {} instructions; {} labels; {} encoding; {:.2} ms",
        program.source().text.len(),
        program.instructions().len(),
        program.label_count(),
        if program.source().compressed {
            "ONS2 compressed"
        } else {
            "UTF-8"
        },
        start.elapsed().as_secs_f64() * 1000.0
    );
    let mut assets = AssetStore::new(storage, limits);
    if options.action == "inspect" || options.asset.is_some() {
        assets.mount_archives("", options.nsa_offset)?;
        for (name, index) in assets.archives() {
            println!("{name}: {} indexed entries", index.len());
        }
    }
    if let Some(name) = options.asset {
        let Some(mut asset) = assets.open(&name)? else {
            return Err(Error::invalid(format!("asset {name} was not found")));
        };
        let mut prefix = [0; 16];
        let count = asset.reader.read(&mut prefix)?;
        println!(
            "{name}: {}; {} bytes; prefix {:02x?}",
            asset.source,
            asset.length,
            &prefix[..count]
        );
    }
    if options.action == "inspect" {
        let counts = program.command_counts();
        let mut counts = counts.into_iter().collect::<Vec<_>>();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        println!(
            "{} distinct top-level commands; 20 most frequent:",
            counts.len()
        );
        for (name, count) in counts.into_iter().take(20) {
            println!("  {name}: {count}");
        }
        return Ok(());
    }
    if options.action == "play" {
        let mut playback = options.playback;
        playback.nsa_offset = options.nsa_offset;
        return onscripter_render::player::play(&program, assets, playback);
    }
    let mut vm = Vm::new(&program, limits);
    vm.set_archive_offset(options.nsa_offset);
    let mut wait_until = None;
    for _ in 0..options.ticks {
        if let Some(deadline) = wait_until.take() {
            while Instant::now() < deadline {
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(std::time::Duration::from_millis(50)),
                );
            }
            vm.resume();
        }
        match vm.run_tick()? {
            Event::Yield => std::thread::yield_now(),
            Event::Finished => {
                println!("Script finished.");
                return Ok(());
            }
            Event::Caption(caption) => println!("Caption: {caption}"),
            Event::Click => {
                wait_for_enter()?;
                vm.resume();
            }
            Event::Wait(duration) => {
                wait_until = Some(
                    Instant::now()
                        .checked_add(std::time::Duration::from_millis(duration))
                        .ok_or_else(|| {
                            Error::invalid("wait duration exceeds the host clock range")
                        })?,
                );
            }
            Event::Archives { directory, offset } => {
                assets.mount_archives(&directory, offset)?;
                vm.resume();
            }
            Event::FileExists { variable, name } => {
                vm.set_number(variable, i32::from(assets.open(&name)?.is_some()))?;
                vm.resume();
            }
            Event::Blocked => {
                return Err(Error::invalid(
                    "host did not resolve a pending script event",
                ));
            }
            Event::Scene { line, .. } | Event::Text { line, .. } => {
                return Err(Error::Script {
                    line,
                    message: "rendering command requires play instead of the headless run command"
                        .into(),
                });
            }
        }
    }
    Err(Error::invalid(
        "headless execution tick limit reached; increase --ticks if needed",
    ))
}

fn wait_for_enter() -> Result<()> {
    print!("[Enter to continue] ");
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Err(Error::invalid("input ended while the script was waiting"));
    }
    Ok(())
}

fn main() -> ExitCode {
    match options().and_then(|options| options.map_or(Ok(()), run)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("onscripter-new-rust: {error}");
            ExitCode::FAILURE
        }
    }
}
