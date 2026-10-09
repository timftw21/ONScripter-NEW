# ONScripter-NEW

**A modern engine for playing Umineko Project on current PCs and Android
devices.**

`ONScripter-NEW` is based on [ONScripter-RU](https://github.com/umineko-project/onscripter-ru),
the engine used by Umineko Project. It keeps compatibility with the game while
improving performance, frame pacing, media playback, menus, and support for
modern systems.

This repository contains the engine only. It does **not** include Umineko game
data, artwork, audio, or other copyrighted assets. You need an existing legal
Umineko Project installation.

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

On Apple Silicon, `configure` selects arm64 and places the build in
`DerivedData/MacOSX-aarch64`. macOS builds require macOS 14 or newer.
The macOS renderer uses Metal. All 18 built-in effects have
native Metal shaders, including blur, transitions, text, and video conversion.
The shader sources are embedded; no shader translator is needed at runtime.

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

## Why use ONScripter-NEW?

- Smoother animation and rain effects, especially on high-refresh displays.
- Faster menus, text rendering, save/load operations, and scene composition.
- Modern graphics through SDL3 (Vulkan on Windows/Android, Metal on macOS), with hardware-assisted video
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

| | ONScripter-RU | ONScripter-NEW |
| --- | --- | --- |
| **Purpose** | The original customized engine behind Umineko Project, with its established compatibility and behavior. | A modern continuation focused on current Umineko Project releases. |
| **Graphics** | Retains older rendering paths for wider compatibility. | Uses SDL3 and Vulkan, with native shaders and GPU-accelerated video conversion. |
| **Performance** | Favors established behavior across many scripts and platforms. | Tunes rendering, rain, text, menus, saves, media, and memory use around Umineko Project. |
| **Game experience** | Stays close to the upstream engine and original project UI. | Includes maintained Umineko-specific interface, script, control, and quality-of-life fixes. |
| **Platforms** | Supports a wider range of older systems and build configurations. | Provides modern 64-bit Windows and Android packages, with other platforms available to build from source. |
| **Best choice when…** | You need the original engine, its broader historical configurations, or an older-system build. | You want the maintained modern engine and release package for Umineko Project. |

Both engines are closely tied to Umineko Project. `ONScripter-NEW` is not a
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

Desktop builds use local saves by default. Pass `--enable-icloud` to use
iCloud storage on macOS or Windows; `--disable-icloud` selects local storage.
If an existing installation saved to iCloud, use `--enable-icloud` to continue
using those saves.

This fork deliberately favors Umineko Project over compatibility with unrelated
ONScripter games. For other titles, use ONScripter-RU or the engine recommended
by that project.

## Credits

`ONScripter-NEW` builds on the work of:

- Ogapee and the original ONScripter contributors
- “Uncle” Mion Sonozaki and ONScripter-RU contributors
- Umineko Project
- The SDL, FFmpeg, and other open-source library communities

AI-assisted tools were used for parts of code review, modernization,
documentation, and release verification. Changes and release builds were
reviewed and tested locally before publication.
