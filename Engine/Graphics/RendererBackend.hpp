/**
 *  RendererBackend.hpp
 *  ONScripter-RU
 *
 *  Renderer backend boundary.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#pragma once

#include "Support/SDLCompat.hpp"

#define ONS_RENDERER_BACKEND_SDL3_GPU 1
#include "Engine/Graphics/SDL3GPUCompat.hpp"

using RenderBatchFlag = GPU_BatchFlagEnum;
using RenderBlendMode = GPU_BlendMode;
using RenderDriverId = GPU_RendererID;
using RenderFileFormat = GPU_FileFormatEnum;
using RenderFormat = GPU_FormatEnum;
using RenderImage = GPU_Image;
using RenderInitFlags = GPU_InitFlagEnum;
using RenderRect = GPU_Rect;
using RenderShaderBlock = GPU_ShaderBlock;
using RenderShaderType = GPU_ShaderEnum;
using RenderTarget = GPU_Target;
using RenderWindowFlags = GPU_WindowFlagEnum;

#if defined(MACOSX) || defined(IOS)
inline constexpr const char *NativeGPUDriver = "metal";
inline constexpr const char *NativeGPURenderer = "Metal";
#else
inline constexpr const char *NativeGPUDriver = "vulkan";
inline constexpr const char *NativeGPURenderer = "Vulkan";
#endif
