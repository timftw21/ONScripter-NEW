# Native tests and fuzzers

The standalone test project exercises security-sensitive components without
requiring copyrighted game data or initializing the SDL runtime.

Configure and run the deterministic tests:

```sh
cmake -S Tests -B DerivedData/tests -G Ninja
cmake --build DerivedData/tests
ctest --test-dir DerivedData/tests --output-on-failure
```

On Linux with Clang, enable ASan/UBSan and the libFuzzer target:

```sh
CC=clang CXX=clang++ cmake -S Tests -B DerivedData/tests-sanitize -G Ninja \
  -DONS_ENABLE_SANITIZERS=ON -DONS_BUILD_FUZZERS=ON
cmake --build DerivedData/tests-sanitize
ctest --test-dir DerivedData/tests-sanitize --output-on-failure
DerivedData/tests-sanitize/fuzz_archive_parser -runs=10000
```

Fuzz findings should be minimized and retained as regression cases before a
fix is merged. The suite covers archive indexes, command-line validation,
serialized save data, the bounded legacy regular-expression matcher, and
multidimensional variable-copy correctness.

The SDL3 GPU benchmark first checks texture upload, rendering, and readback
pixel accuracy. It covers padded rows at an odd texture width and a full-HD
texture uploaded in multiple staging chunks. A pixel mismatch exits with an
error before timing begins. On Metal and Vulkan it also compiles and renders
all 18 built-in effects, checking pixels against the CPU reference with distinct
sampler inputs, alpha, scalar/vector uniforms, and subtitle arrays. The native
draw must succeed without a CPU fallback. Run this on a machine with a GPU and
window access:

```sh
DerivedData/MacOSX-aarch64/onscripter-new --sdl3-benchmark \
  --sdl3-benchmark-iterations 3 --sdl3-benchmark-width 320 \
  --sdl3-benchmark-height 240
```

Regenerate or check the embedded Metal effects with Khronos SPIRV-Cross:

```sh
python3 Scripts/generate_sdl3_metal_shaders.py --spirv-cross /path/to/spirv-cross
python3 Scripts/generate_sdl3_metal_shaders.py --spirv-cross /path/to/spirv-cross --check
```

The generator documents the translator revision used. Texture slots and uniform
offsets are preserved from the existing SPIR-V, so both GPU backends share the
same resource layout.
