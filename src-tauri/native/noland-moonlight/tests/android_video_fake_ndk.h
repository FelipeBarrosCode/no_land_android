#ifndef NOLAND_ANDROID_VIDEO_FAKE_NDK_H
#define NOLAND_ANDROID_VIDEO_FAKE_NDK_H

// Minimal API-24 MediaCodec surface used by the host-side fault-injection tests.
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

typedef struct ANativeWindow ANativeWindow;
typedef struct AMediaCodec AMediaCodec;
typedef struct AMediaFormat AMediaFormat;
typedef int32_t media_status_t;
typedef struct AMediaCodecBufferInfo {
  int32_t offset;
  int32_t size;
  int64_t presentationTimeUs;
  uint32_t flags;
} AMediaCodecBufferInfo;

#define AMEDIA_OK 0
#define AMEDIACODEC_INFO_TRY_AGAIN_LATER (-1)
#define AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED (-2)
#define AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED (-3)
#define AMEDIACODEC_BUFFER_FLAG_CODEC_CONFIG 2U
#define AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM 4U
#define AMEDIAFORMAT_KEY_MIME "mime"
#define AMEDIAFORMAT_KEY_WIDTH "width"
#define AMEDIAFORMAT_KEY_HEIGHT "height"
#define AMEDIAFORMAT_KEY_FRAME_RATE "frame-rate"
#define AMEDIAFORMAT_KEY_MAX_INPUT_SIZE "max-input-size"

AMediaCodec* AMediaCodec_createDecoderByType(const char* mime);
media_status_t AMediaCodec_configure(AMediaCodec* codec, const AMediaFormat* format,
                                     ANativeWindow* window, void* crypto, uint32_t flags);
media_status_t AMediaCodec_start(AMediaCodec* codec);
media_status_t AMediaCodec_stop(AMediaCodec* codec);
media_status_t AMediaCodec_delete(AMediaCodec* codec);
ssize_t AMediaCodec_dequeueInputBuffer(AMediaCodec* codec, int64_t timeout_us);
uint8_t* AMediaCodec_getInputBuffer(AMediaCodec* codec, size_t index, size_t* capacity);
media_status_t AMediaCodec_queueInputBuffer(AMediaCodec* codec, size_t index,
    off_t offset, size_t size, uint64_t time_us, uint32_t flags);
ssize_t AMediaCodec_dequeueOutputBuffer(AMediaCodec* codec, AMediaCodecBufferInfo* info, int64_t timeout_us);
media_status_t AMediaCodec_releaseOutputBuffer(AMediaCodec* codec, size_t index, bool render);
AMediaFormat* AMediaCodec_getOutputFormat(AMediaCodec* codec);
AMediaFormat* AMediaFormat_new(void);
media_status_t AMediaFormat_delete(AMediaFormat* format);
void AMediaFormat_setString(AMediaFormat* format, const char* name, const char* value);
void AMediaFormat_setInt32(AMediaFormat* format, const char* name, int32_t value);
void AMediaFormat_setBuffer(AMediaFormat* format, const char* name, const void* value, size_t size);

#endif
