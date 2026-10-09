/**
 *  SDL3GPUShaders.hpp
 *  ONScripter-RU
 *
 *  Native SDL3_GPU shaders used by the SDL2_gpu compatibility backend.
 *  The fixed shaders derive from SDL's built-in GPU renderer shaders, under
 *  SDL's zlib license. Effect shaders implement Resources/Shaders; their Metal
 *  headers are generated from the SPIR-V by generate_sdl3_metal_shaders.py.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#pragma once

#include "Engine/Graphics/SDL3GPUShaders/alpha_outside_textures.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/alpha_outside_textures.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/blend_by_mask.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/blend_by_mask.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/blur_h.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/blur_h.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/blur_v.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/blur_v.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/breakup.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/breakup.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/color_modification.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/color_modification.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/colour_conversion.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/colour_conversion.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/crop_by_mask.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/crop_by_mask.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_trvswave.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_trvswave.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_warp.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_warp.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_whirl.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/effect_whirl.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/glass_smash.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/glass_smash.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/glyph_gradient.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/glyph_gradient.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/merge_alpha.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/merge_alpha.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/multiply_alpha.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/multiply_alpha.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/pixelate.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/pixelate.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/render_subtitles.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/render_subtitles.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/text_fade.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/text_fade.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/texture_rgba.frag.dxil.h"
#include "Engine/Graphics/SDL3GPUShaders/texture_rgba.frag.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/texture_rgba.frag.spv.h"
#include "Engine/Graphics/SDL3GPUShaders/tri_texture.vert.dxil.h"
#include "Engine/Graphics/SDL3GPUShaders/tri_texture.vert.msl.h"
#include "Engine/Graphics/SDL3GPUShaders/tri_texture.vert.spv.h"
