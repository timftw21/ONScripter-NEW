/**
 *  HardwareDecoder.cpp
 *  ONScripter-RU
 *
 *  Contains Media Engine hardware decoding support.
 *
 *  Consult LICENSE file for licensing terms and copyright holders.
 */

#include "Engine/Media/Controller.hpp"

#if defined(IOS) || defined(MACOSX)

extern "C" {
#include <libavutil/hwcontext.h>
}

namespace HardwareDecoderVT {

static void reg() {}

static AVPixelFormat init(AVCodecContext *context, const AVPixelFormat *format) {
	if (MediaProcController::HardwareDecoderIFace::hasFormat(format, AV_PIX_FMT_VIDEOTOOLBOX)) {
		AVBufferRef *device = nullptr;
		AVBufferRef *frames = nullptr;
		int ret = av_hwdevice_ctx_create(&device, AV_HWDEVICE_TYPE_VIDEOTOOLBOX, nullptr, nullptr, 0);
		if (ret >= 0)
			ret = avcodec_get_hw_frames_parameters(context, device, AV_PIX_FMT_VIDEOTOOLBOX, &frames);
		av_buffer_unref(&device);
		if (ret >= 0)
			ret = av_hwframe_ctx_init(frames);
		if (ret >= 0) {
			av_buffer_unref(&context->hw_frames_ctx);
			context->hw_frames_ctx = frames;
			sendToLog(LogLevel::Info, "Successfully initialised VT decoder\n");
			return AV_PIX_FMT_VIDEOTOOLBOX;
		}
		av_buffer_unref(&frames);
		sendToLog(LogLevel::Warn, "Unable to initialise VT decoder; using software decoding\n");
	}

	return MediaProcController::HardwareDecoderIFace::defaultFormat(format);
}

static const AVCodec *findDecoder(AVCodecContext *) {
	return nullptr;
}

static void deinit(AVCodecContext *context) {
	// FFmpeg releases the frame pool and device when the codec context is freed.
	(void)context;
}

static AVFrame *process(AVFrame *dFrame, AVFrame *&tempFrame) {
	if (dFrame->format != AV_PIX_FMT_VIDEOTOOLBOX)
		return dFrame;

	if (tempFrame)
		av_frame_unref(tempFrame);
	else
		tempFrame = av_frame_alloc();
	if (!tempFrame)
		return nullptr;

	// Use FFmpeg's transfer path for every pixel format exposed by VideoToolbox.
	if (av_hwframe_transfer_data(tempFrame, dFrame, 0) < 0 ||
	    av_frame_copy_props(tempFrame, dFrame) < 0)
		return nullptr;

	av_frame_unref(dFrame);
	av_frame_move_ref(dFrame, tempFrame);
	return dFrame;
}
} // namespace HardwareDecoderVT

#elif defined(DROID)

namespace HardwareDecoderMC {
#include <jni.h>

extern "C" {
#include <libavcodec/mediacodec.h>
#include <libavcodec/jni.h>
}

static JavaVM *getJavaVM() {
	static JavaVM *vm{nullptr};

	if (vm)
		return vm;

	auto env = static_cast<JNIEnv *>(SDL_AndroidGetJNIEnv());
	if (env)
		env->GetJavaVM(&vm);
	else
		sendToLog(LogLevel::Error, "Failed to get JNIEnv\n");

	return vm;
}

static void reg() {
	auto vm = getJavaVM();

	if (vm) {
		int err = av_jni_set_java_vm(vm, nullptr);
		if (err)
			sendToLog(LogLevel::Error, "Failed to set java vm for hw accelerated decoding\n");
	} else {
		sendToLog(LogLevel::Error, "No java vm available for hw accelerated decoding\n");
	}
}

static AVPixelFormat init(AVCodecContext *context, const AVPixelFormat *format) {
	if (MediaProcController::HardwareDecoderIFace::hasFormat(format, AV_PIX_FMT_MEDIACODEC)) {
		auto hwctx = context->hwaccel_context ? context->hwaccel_context : av_mediacodec_alloc_context();
		if (hwctx) {
			context->hwaccel_context = hwctx;
			sendToLog(LogLevel::Info, "Successfully initialised MC decoder\n");
			return AV_PIX_FMT_MEDIACODEC;
		} else {
			sendToLog(LogLevel::Error, "Failed to allocate MC decoder context\n");
		}
	}

	return MediaProcController::HardwareDecoderIFace::defaultFormat(format);
}

static const AVCodec *findDecoder(AVCodecContext *context) {
	switch (context->codec_id) {
		case AV_CODEC_ID_H264:
			return avcodec_find_decoder_by_name("h264_mediacodec");
		case AV_CODEC_ID_HEVC:
			return avcodec_find_decoder_by_name("hevc_mediacodec");
		case AV_CODEC_ID_MPEG4:
			return avcodec_find_decoder_by_name("mpeg4_mediacodec");
		case AV_CODEC_ID_VP8:
			return avcodec_find_decoder_by_name("vp8_mediacodec");
		case AV_CODEC_ID_VP9:
			return avcodec_find_decoder_by_name("vp9_mediacodec");
		default:
			return nullptr;
	}
}

static void deinit(AVCodecContext *context) {
	if (context->hwaccel_context)
		av_mediacodec_default_free(context);
}

static AVFrame *process(AVFrame *dFrame, AVFrame *&tempFrame) {
	(void)tempFrame;
	return dFrame;
}
} // namespace HardwareDecoderMC
#endif

const std::unordered_set<int> MediaProcController::HardwareDecoderIFace::hardwareAcceleratedFormats {
#if defined(LINUX)
#ifdef AV_PIX_FMT_VDPAU_H264
	AV_PIX_FMT_VDPAU_H264,
#endif
#ifdef AV_PIX_FMT_VDPAU_MPEG1
	AV_PIX_FMT_VDPAU_MPEG1,
#endif
#ifdef AV_PIX_FMT_VDPAU_MPEG2
	AV_PIX_FMT_VDPAU_MPEG2,
#endif
#ifdef AV_PIX_FMT_VDPAU_WMV3
	AV_PIX_FMT_VDPAU_WMV3,
#endif
#ifdef AV_PIX_FMT_VDPAU_VC1
	AV_PIX_FMT_VDPAU_VC1,
#endif
#ifdef AV_PIX_FMT_VDPAU
	AV_PIX_FMT_VDPAU,
#endif
#ifdef AV_PIX_FMT_VAAPI_MOCO
	AV_PIX_FMT_VAAPI_MOCO,
#endif
#ifdef AV_PIX_FMT_VAAPI_IDCT
	AV_PIX_FMT_VAAPI_IDCT,
#endif
#ifdef AV_PIX_FMT_VAAPI
	AV_PIX_FMT_VAAPI,
#endif
#elif defined(WIN32)
#ifdef AV_PIX_FMT_DXVA2_VLD
	AV_PIX_FMT_DXVA2_VLD,
#endif
#ifdef AV_PIX_FMT_D3D11VA_VLD
	AV_PIX_FMT_D3D11VA_VLD,
#endif
#elif defined(IOS) || defined(MACOSX)
#ifdef AV_PIX_FMT_VDA_VLD
	// VDA support is disabled in our builds, and ffmpeg 4.x has it removed.
	// Let code compile without it at the very least.
	AV_PIX_FMT_VDA_VLD,
#endif
	AV_PIX_FMT_VIDEOTOOLBOX
#elif defined(DROID)
	AV_PIX_FMT_MEDIACODEC
#endif
};

const std::unordered_set<int> MediaProcController::HardwareDecoderIFace::hwConvertedFormats{
    AV_PIX_FMT_NV12,
    AV_PIX_FMT_YUV420P};

void MediaProcController::HardwareDecoderIFace::reg() {
#if defined(IOS) || defined(MACOSX)
	HardwareDecoderVT::reg();
#elif defined(DROID)
	HardwareDecoderMC::reg();
#endif
}

AVPixelFormat MediaProcController::HardwareDecoderIFace::init(AVCodecContext *context, const AVPixelFormat *format) {
#if defined(IOS) || defined(MACOSX)
	return HardwareDecoderVT::init(context, format);
#elif defined(DROID)
	return HardwareDecoderMC::init(context, format);
#else
	(void)context;
	return defaultFormat(format);
#endif
}

const AVCodec *MediaProcController::HardwareDecoderIFace::findDecoder(AVCodecContext *context) {
#if defined(IOS) || defined(MACOSX)
	return HardwareDecoderVT::findDecoder(context);
#elif defined(DROID)
	return HardwareDecoderMC::findDecoder(context);
#else
	(void)context;
	return nullptr;
#endif
}

void MediaProcController::HardwareDecoderIFace::deinit(AVCodecContext *context) {
	if (!context)
		return;

#if defined(IOS) || defined(MACOSX)
	HardwareDecoderVT::deinit(context);
#elif defined(DROID)
	return HardwareDecoderMC::deinit(context);
#endif
}

AVFrame *MediaProcController::HardwareDecoderIFace::process(AVFrame *hwFrame, AVFrame *&tmpFrame) {
#if defined(IOS) || defined(MACOSX)
	return HardwareDecoderVT::process(hwFrame, tmpFrame);
#elif defined(DROID)
	return HardwareDecoderMC::process(hwFrame, tmpFrame);
#else
	(void)tmpFrame;
	return hwFrame;
#endif
}
