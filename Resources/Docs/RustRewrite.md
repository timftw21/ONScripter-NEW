# Rust rewrite design

Rebuild `onscripter-new` in Rust for smooth desktop and mobile performance,
resistance to malicious game data, and a small, maintainable engine.

## Scope and compatibility

Initially retain the [maintained targets](ProjectStatus.md): Windows 10+ x86-64
and Android 11+ ARM64. Keep Linux, macOS, and iOS portability in mind; claim
support only after build and device verification.

The first compatibility target is the existing Umineko Project experience:
scripts, compressed scripts, archives, loose-file precedence, text, effects,
media, menus, controls, and backlog. Inventory required commands, encodings,
formats, and save versions before implementation. Expand game compatibility
when needed.

The fork's maintained scripts, modified assets, and loose overrides are part
of the product. Preserve their loading order and patch behavior. Validation
checks file safety, not whether content matches an original game's checksum.

Import supported legacy saves without overwriting them. Map their script
positions to the new interpreter. Keep rewrite saves separate and versioned,
with atomic writes and a previous-valid-save backup.

## Engine structure

Use one Cargo workspace: a Rust core, small native adapters, and thin launchers.
The core owns scripts, state, assets, scenes, and saves; it forbids `unsafe`
and platform types. Adapters own graphics, device audio, storage, and lifecycle
events. Dependencies flow toward the core.

Parse scripts into compact instructions with source locations; resolve static
commands and labels once while preserving dynamic expressions and commands.
The interpreter explicitly yields for input, timers, and media. Errors identify
the script location.

One main thread owns state and rendering. A bounded worker pool handles loading
and decoding, with cancellation of obsolete work. Audio callbacks never
allocate or block.

Proposed stack: SDL3 for platform services and its
[GPU API](https://wiki.libsdl.org/SDL3/CategoryGPU) for Vulkan, Direct3D 12,
and Metal. Write fresh adapters and compile engine shaders during the build.
The initial 2D adapter uses SDL3's GPU renderer and its built-in shaders;
add custom GPU passes only when required by measured performance or game effects.
Choose text/media libraries from compatibility requirements; retain a minimal
FFmpeg build only where necessary.

## Performance

- Batch draws without changing visual order; cache glyphs and reuse GPU buffers
  and textures. Avoid routine GPU readbacks.
- Index archives once, stream media, and prefetch within fixed budgets. Bound
  CPU/GPU caches, decoded frames, and queues; evict rebuildable resources under
  memory pressure.
- Keep game timing independent of refresh rate and video synchronized to audio.
  Sleep when idle, suspend rendering in the background, and recover after
  surface loss and rotation.

Target stable 60 FPS (16.7 ms frame budget) on reference Windows hardware and a
midrange Android phone; evaluate higher refresh rates separately. Establish
devices, GPU requirements, workloads, and memory ceilings first. Compare frame
percentiles, startup, save/load, memory, audio underruns, and sustained power
use against the C++ baseline. Optimize measured bottlenecks.

## Security boundaries

Treat all game files, configuration, and saves as untrusted. Validate structure,
offsets, arithmetic, dimensions, and decompressed sizes before allocation. Limit
memory, parser nesting, call depth, and instructions per tick. Long scripts
yield; malformed input produces an error rather than a panic.

Storage adapters expose the game roots, maintained overrides, configuration,
and saves the engine needs. Validate game-supplied paths and keep writes within
those intended locations. Android's
[folder access framework](https://developer.android.com/training/data-storage/shared/documents-files)
supplies storage handles without all-files permission; non-seekable inputs
require a copy within a disk quota.

Preserve existing links, configuration tools, and required Lua behavior through
explicit engine interfaces. Validate their arguments and avoid shell-string
execution. Security restrictions must not disable maintained game features,
patches, or modified assets, or add permission prompts to ordinary gameplay.

Review every necessary `unsafe` adapter for ownership and lifetimes. Keep
native codecs and font parsers minimal and patched; prefer memory-safe
implementations when compatible. Evaluate process isolation against platform,
performance, and compatibility requirements. Rust and worker threads alone
do not sandbox native libraries.

## Build and delivery

Use Cargo plus thin native/Gradle packaging steps. Pin toolchains and dependencies,
verify native downloads, and review security and licenses. Preserve attribution.
Ship required features only; avoid general-purpose frameworks and duplicated
rendering or codec stacks.

1. **Establish the contract:** inventory compatibility, measure baselines, and
   validate GPU coverage, storage, native parser safety, and dependency choices.
2. **Build a playable slice:** scripts, archives, text, sprites, audio, input,
   and save/restore on Windows and Android.
3. **Reach parity:** complete effects, video, menus, and legacy save import.
   Replace the C++ engine after each target passes compatibility, performance,
   resource-budget, and lifecycle checks.

Reuse the synthetic fixture and focus fuzzing on security boundaries. Verify
representative scenes and complete-game play with legally supplied data. Retain
the fork's distributable modified assets and patches; keep other game data
outside the repository.
