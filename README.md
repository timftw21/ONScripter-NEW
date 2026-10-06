# onscripter-new

**A modern engine for playing Umineko Project on current PCs and Android
devices.**

`onscripter-new` is based on [ONScripter-RU](https://github.com/umineko-project/onscripter-ru),
the engine used by Umineko Project. It keeps compatibility with the game while
improving performance, frame pacing, media playback, menus, and support for
modern systems.

Release packages ship the engine together with maintained scripts and modified
loose assets. You need an existing legal Umineko Project installation for the
rest of the game.

[**Download the latest release**](https://github.com/timftw21/onscripter-new/releases/latest)

## Contents

- [Building from source](#building-from-source)
- [Why use onscripter-new?](#why-use-onscripter-new)
- [How is it different from ONScripter-RU?](#how-is-it-different-from-onscripter-ru)
- [Installation](#installation)
  - [Windows](#windows)
  - [Android](#android)
  - [Verifying downloads](#verifying-downloads)
- [Saves and compatibility](#saves-and-compatibility)
- [Credits](#credits)

## Building from source

Most players should use the release packages. For contributors and platform
maintainers, install a C++23 compiler, GNU Make, CMake, Meson, Ninja, NASM,
`pkg-config`, and the normal POSIX build utilities. On Windows, use the MSYS2
UCRT64 environment. A normal host build is:

```sh
./configure --release-build --std=gnu++23
make -j8
```

Android has its own build, run and debug guide, including the Android Studio
workflow: [Resources/Docs/Android.md](Resources/Docs/Android.md).

The maintained platform notes and exact Windows prerequisites live in
[Resources/Docs/ProjectStatus.md](Resources/Docs/ProjectStatus.md).

Security-sensitive native tests are independent of copyrighted game data:

```sh
cmake -S Tests -B DerivedData/tests -G Ninja
cmake --build DerivedData/tests
ctest --test-dir DerivedData/tests --output-on-failure
```

See [Tests/README.md](Tests/README.md) for sanitizer and fuzzing options and
[SECURITY.md](SECURITY.md) for the vulnerability-reporting and trust-boundary
policy.

### Rust rewrite development

The `rewrite` branch contains the Rust core, a headless runner, an SDL3 GPU
renderer, Unicode dialogue, and an Android package. Audio, video, advanced
effects, and full game compatibility are
still being implemented. The [rewrite design](Resources/Docs/RustRewrite.md)
describes the direction.

The workspace pins Rust 1.99.0; `rustup` installs the selected toolchain and
components automatically. Windows builds require Visual Studio C++ build tools
and CMake. SDL3 is built and linked statically by Cargo; image decoding uses
Rust libraries. Rendering requires a Vulkan-capable graphics driver.

```sh
cargo run --release -- inspect <game-directory-or-script>
cargo run --release -- run Tests/Fixtures/SmokeGame/0.txt
cargo run --release -- play <game-directory-or-script>
```

Use `--overlay <folder>` for additional maintained assets, in priority order.
Loose assets override archive entries, including modified assets shipped with
this fork. `cargo run -- --help` lists the development runner's options.

`play` supports `bg`, `lsp`/`lsph`, `csp`, `vsp`, `msp`/`amsp`, `cell`,
`allsphide`/`allspresume`, and `print`. Scene changes are staged until `print`;
`bg` also presents the scene. Effects 0/1 are immediate, and effect 10 is a
crossfade; `effect` can name these transitions. PNG, JPEG, WebP, and BMP images
support alpha, legacy split masks, color keys, horizontal/vertical cells, and
positive-duration cell animations. Unsupported tags and effects report an error.

Dialogue uses the fork's raw `d` command: `d Hello, world![@]`. Colons and
semicolons remain text. `[@]` waits for input, `[\]` waits and clears the page,
and `[br]` inserts a line break. `[!w200]` waits 200 milliseconds; `[!d200]`
allows input to skip that delay. `text_speed` accepts the fork's 0–10 scale;
input first completes the current segment, then advances its wait. `setwindow`,
`setwindow3`, and `setwindow4` support color-backed windows, with `setwindow2`, `br`,
`textclear`, `texton`/`textoff`, and `textshow`/`texthide` for control. Each page is
laid out before its timed reveal begins, so inline pauses preserve text positions.
Visibility and window-color changes reuse the layout. Glyphs, outlines, and
decorations share a bounded GPU atlas.

Nested formatting follows the fork's syntax: `{b:Bold}`, `{i:Italic}`,
`{u:Underlined}`, `{c:ff0000:Red}`, `{d:32:Larger}`, `{f:1:Another font}`, and
`{h:にほん:日本}` for ruby. Font size percentages, character spacing, colored
shadows/outlines, centering, wrap widths, no-break scopes, and fit-to-width are
also supported, with the native short and long tag names. `preset_define`
defines reusable `{p:0:Styled}` presets; `d_condition` controls `{y:0:Yes}` and
`{n:0:No}` scopes. Pages, annotations, nesting, font data, and glyph caches have
explicit memory or size limits.

`d2` displays dialogue while the script continues. `[#]` signals a numbered
marker for `wait_on_d 0`; `[*]` pauses dialogue until `d_continue`. Use
`wait_on_d -1` to wait for completion, or `d_dispose` to cancel active dialogue.
The game supplies `fonts/default.otf` or `fonts/default.ttf`; `--font <asset>`
selects another game-relative default font, including collections. Numbered
slots load `fonts/font1.otf`/`.ttf` through `font9` as needed and use the same
modified-asset precedence. Gradients, parallel text runs, speaker-name windows,
image-backed windows, custom window padding, and inline script calls remain
unsupported and report errors.

The canvas follows the script's `;mode` header and is letterboxed when the window
changes size. Use `--size WIDTHxHEIGHT` to override it. Enter, Space, left click,
or touch advances `click`; Escape closes the window. Static scenes sleep while
idle, and backgrounding pauses the game clock. Decoding runs on one worker with
bounded queues; textures are cached within a memory budget and adjacent sprites
sharing a texture are drawn together without changing their order.

For a bounded rendering run, add `--frames 120 --capture frame.png`. Captures use
the logical canvas size. `--hidden` keeps the window hidden for verification;
captures are the only routine that reads pixels back from the GPU.

For Android ARM64, open `Rust/android` in Android Studio or run its
`gradlew assembleDebug` (`gradlew.bat` on Windows). This builds SDL3 and Rust
before packaging `Rust/android/build/outputs/apk/debug/onscripter-rust-debug.apk`.
Requirements: SDK 36, NDK 28.2.13676358, Java 17+, CMake 3.28+, Ninja, and Rust.
Set `ANDROID_HOME` to the SDK, or configure `local.properties` in that project.
The separate `org.onscripter.rewrite` app selects game and optional modified-asset
folders through Android's picker and preserves loose-file/archive precedence.
APK builds and library alignment are checked; phone validation remains pending.

## Why use onscripter-new?

- Smoother animation and rain effects, especially on high-refresh displays.
- Faster menus, text rendering, save/load operations, and scene composition.
- Modern Vulkan-based graphics through SDL3, with hardware-assisted video
  playback and color conversion where supported.
- More predictable RAM use during long sessions and video playback.
- Current Windows and Android builds with fewer legacy runtime dependencies.
- Umineko Project-specific fixes and polish for Config, the pause menu, Message
  Browser, file verification, controls, and other in-game screens.
- Optional Discord Rich Presence on desktop.

The goal is simple: preserve the Umineko Project experience while making the
engine feel at home on modern hardware.

## How is it different from ONScripter-RU?

ONScripter-RU remains the foundation of this project. The two projects now have
different priorities:

| | ONScripter-RU | onscripter-new |
| --- | --- | --- |
| **Purpose** | The original customized engine behind Umineko Project, with its established compatibility and behavior. | A modern continuation focused on current Umineko Project releases. |
| **Graphics** | Retains older rendering paths for wider compatibility. | Uses SDL3 and Vulkan, with native shaders and GPU-accelerated video conversion. |
| **Performance** | Favors established behavior across many scripts and platforms. | Tunes rendering, rain, text, menus, saves, media, and memory use around Umineko Project. |
| **Game experience** | Stays close to the upstream engine and original project UI. | Includes maintained Umineko-specific interface, script, control, and quality-of-life fixes. |
| **Platforms** | Supports a wider range of older systems and build configurations. | Provides modern 64-bit Windows and Android packages, with other platforms available to build from source. |
| **Best choice when…** | You need the original engine, its broader historical configurations, or an older-system build. | You want the maintained modern engine and release package for Umineko Project. |

Both engines are closely tied to Umineko Project. `onscripter-new` is not a
clean-sheet replacement; it trades some of ONScripter-RU's older platform and
renderer flexibility for a smaller, modern stack and more active optimization
of the current game package.

## Installation

### Windows

Requirements: 64-bit Windows 10 or newer.

1. Download `onscripter-new-windows-x86_64.zip` from the latest release.
2. Back up your game folder and saves.
3. Extract the archive into your Umineko Project folder, allowing it to replace
   the included engine, language scripts, and maintained loose assets.
4. Run `onscripter-new.exe`.

The Windows build is self-contained; no separate SDL or Vulkan runtime files
need to be copied beside the executable. A working graphics driver with Vulkan
support is required.

### Android

Requirements: Android 11 or newer.

1. Download `onscripter-new-android.apk` from the latest release.
2. Download all required Umineko Project files and place them in a folder titled "ONScripter-RU" on the root of your phone.
3. Extract the required files within the folder.
4. Install the APK.
5. Launch **onscripter-new**.

The APK contains the engine, not the game. Extract
`onscripter-new-android-assets.zip` from the same release into your game folder,
replacing the included files. This ZIP supplies the same English, Witch Hunt,
and Russian scripts and all loose assets included in the Windows package.

### Verifying downloads

Every release includes `SHA256SUMS.txt`. You can use it to confirm that the
Windows and Android downloads arrived unchanged.

## Saves and compatibility

The engine is designed for compatible Umineko Project release data and keeps
support for saves created by the preceding ONScripter-RU-based builds. As with
any engine or script update, keeping a backup of your saves and game directory
is recommended.

English, Witch Hunt, and Russian now share the `UminekoPS3ficationEn` save
folder. Existing English saves and progress are used automatically. If your
progress is in a separate `UminekoPS3ficationWh` or `UminekoPS3ficationRu`
folder, back up all profiles and copy your preferred profile's contents into
`UminekoPS3ficationEn` before switching languages. Separate profiles are not
merged automatically.

This fork deliberately favors Umineko Project over compatibility with unrelated
ONScripter games. For other titles, use ONScripter-RU or the engine recommended
by that project.

## Credits

`onscripter-new` builds on the work of:

- Ogapee and the original ONScripter contributors
- “Uncle” Mion Sonozaki and ONScripter-RU contributors
- Umineko Project
- The SDL, FFmpeg, and other open-source library communities

AI-assisted tools were used for parts of code review, modernization,
documentation, and release verification. Changes and release builds were
reviewed and tested locally before publication.
