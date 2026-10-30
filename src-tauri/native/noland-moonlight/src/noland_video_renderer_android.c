#if defined(__ANDROID__) || defined(NOLAND_ANDROID_VIDEO_TEST)

#include "noland_video_renderer.h"
#include "Limelight.h"

#if defined(NOLAND_ANDROID_VIDEO_TEST)
#include "../tests/android_video_fake_ndk.h"
#else
#include <android/native_window.h>
#include <media/NdkMediaCodec.h>
#include <media/NdkMediaFormat.h>
#endif
#include <pthread.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define NL_ANDROID_MAX_FRAME_BYTES (8U * 1024U * 1024U)
#define NL_ANDROID_MAX_CSD_BYTES (64U * 1024U)
#define NL_ANDROID_MAX_IN_FLIGHT 8U
#define NL_ANDROID_INPUT_TIMEOUT_US 2000
#define NL_ANDROID_INPUT_RETRY_US 100000U
#define NL_ANDROID_STARTUP_RETRY_US 500000U
#define NL_ANDROID_STALL_US 500000U
#define NL_ANDROID_STARTUP_STALL_US 2000000U

typedef struct nl_android_video_context {
  pthread_mutex_t mutex;
  pthread_t thread;
  atomic_bool running;
  bool thread_started;
  bool codec_started;
  bool need_idr;
  ANativeWindow* window; // Borrowed until detach returns; never acquire/release here.
  AMediaCodec* codec;
  int video_format;
  int width;
  int height;
  int fps;
  unsigned int in_flight;
  uint64_t last_progress_us;
  uint64_t codec_started_us;
  uint64_t generation;
  bool has_output;
  int pending_core;
  const char* error_reason;
  int64_t error_status;
  uint8_t csd[3][NL_ANDROID_MAX_CSD_BYTES]; // SPS, PPS, VPS, all Annex B.
  size_t csd_size[3];
} nl_android_video_context_t;

static nl_android_video_context_t* nl_android_video_context(nl_video_renderer_t* renderer) {
  return renderer != NULL ? renderer->platform_context : NULL;
}

static nl_android_video_context_t* nl_android_video_ensure_context(nl_video_renderer_t* renderer) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  if (ctx != NULL || renderer == NULL) return ctx;
  ctx = calloc(1, sizeof(*ctx));
  if (ctx == NULL) return NULL;
  if (pthread_mutex_init(&ctx->mutex, NULL) != 0) {
    free(ctx);
    return NULL;
  }
  atomic_init(&ctx->running, false);
  ctx->need_idr = true;
  // Generic cleanup preserves the descriptor across a stream reconnect.
  if (renderer->surface_attached && renderer->surface.surface_type == NL_SURFACE_ANDROID_NATIVE_WINDOW) {
    ctx->window = (ANativeWindow*)renderer->surface.window_handle;
  }
  renderer->platform_context = ctx;
  return ctx;
}

static const char* nl_android_video_mime(int format) {
  switch (format) {
    case VIDEO_FORMAT_H264: return "video/avc";
    case VIDEO_FORMAT_H265:
    case VIDEO_FORMAT_H265_MAIN10: return "video/hevc";
    default: return NULL;
  }
}

static void nl_android_video_log(nl_android_video_context_t* ctx, const char* event,
                                 const char* reason, int frame, int64_t status, uint64_t wait_us) {
  uint64_t now = LiGetMicroseconds();
  fprintf(stderr, "[noland-video] %s reason=%s frame=%d status=%lld wait_us=%llu "
      "inflight=%u pending=%d first_output=%d codec_age_us=%llu generation=%llu csd=%zu/%zu/%zu\n",
      event, reason, frame, (long long)status, (unsigned long long)wait_us,
      ctx->in_flight, ctx->pending_core, ctx->has_output ? 1 : 0,
      (unsigned long long)(ctx->codec_started ? now - ctx->codec_started_us : 0),
      (unsigned long long)ctx->generation, ctx->csd_size[0], ctx->csd_size[1], ctx->csd_size[2]);
}

static bool nl_android_video_error(nl_android_video_context_t* ctx, const char* reason, int64_t status) {
  ctx->error_reason = reason;
  ctx->error_status = status;
  return false;
}

// All codec access, including destruction on detach, is serialized by mutex.
static void nl_android_video_reset(nl_android_video_context_t* ctx) {
  if (ctx->codec != NULL) {
    if (ctx->codec_started) AMediaCodec_stop(ctx->codec);
    AMediaCodec_delete(ctx->codec);
    ctx->codec = NULL;
  }
  ctx->codec_started = false;
  ctx->need_idr = true;
  ctx->in_flight = 0;
  ctx->has_output = false;
  ++ctx->generation;
}

static void nl_android_video_recover(nl_android_video_context_t* ctx, const char* reason,
                                     int frame, int64_t status) {
  nl_android_video_log(ctx, "codec_reset", reason, frame, status, 0);
  nl_android_video_reset(ctx);
}

static bool nl_android_video_configure(nl_android_video_context_t* ctx) {
  AMediaFormat* format;
  media_status_t status;
  const char* mime = nl_android_video_mime(ctx->video_format);
  bool hevc = (ctx->video_format & VIDEO_FORMAT_MASK_H265) != 0;
  if (mime == NULL || ctx->window == NULL || ctx->csd_size[0] == 0 ||
      ctx->csd_size[1] == 0 || (hevc && ctx->csd_size[2] == 0)) {
    return nl_android_video_error(ctx, "missing_csd_or_surface", 0);
  }

  if (ctx->codec == NULL) ctx->codec = AMediaCodec_createDecoderByType(mime);
  if (ctx->codec == NULL) return nl_android_video_error(ctx, "decoder_unavailable", 0);
  format = AMediaFormat_new();
  if (format == NULL) return nl_android_video_error(ctx, "format_allocation", 0);
  AMediaFormat_setString(format, AMEDIAFORMAT_KEY_MIME, mime);
  AMediaFormat_setInt32(format, AMEDIAFORMAT_KEY_WIDTH, ctx->width);
  AMediaFormat_setInt32(format, AMEDIAFORMAT_KEY_HEIGHT, ctx->height);
  AMediaFormat_setInt32(format, AMEDIAFORMAT_KEY_FRAME_RATE, ctx->fps);
  AMediaFormat_setInt32(format, AMEDIAFORMAT_KEY_MAX_INPUT_SIZE, NL_ANDROID_MAX_FRAME_BYTES);
  // Unknown keys are ignored on older codecs. No post-API-24 symbols are used.
  AMediaFormat_setInt32(format, "low-latency", 1);
  AMediaFormat_setInt32(format, "priority", 0);
  if (hevc) {
    size_t size = ctx->csd_size[2] + ctx->csd_size[0] + ctx->csd_size[1];
    uint8_t* csd = malloc(size);
    if (csd == NULL) {
      AMediaFormat_delete(format);
      return nl_android_video_error(ctx, "csd_allocation", 0);
    }
    memcpy(csd, ctx->csd[2], ctx->csd_size[2]);
    memcpy(csd + ctx->csd_size[2], ctx->csd[0], ctx->csd_size[0]);
    memcpy(csd + ctx->csd_size[2] + ctx->csd_size[0], ctx->csd[1], ctx->csd_size[1]);
    AMediaFormat_setBuffer(format, "csd-0", csd, size);
    free(csd);
  } else {
    AMediaFormat_setBuffer(format, "csd-0", ctx->csd[0], ctx->csd_size[0]);
    AMediaFormat_setBuffer(format, "csd-1", ctx->csd[1], ctx->csd_size[1]);
  }
  status = AMediaCodec_configure(ctx->codec, format, ctx->window, NULL, 0);
  AMediaFormat_delete(format);
  if (status != AMEDIA_OK) return nl_android_video_error(ctx, "configure_failed", status);
  status = AMediaCodec_start(ctx->codec);
  if (status != AMEDIA_OK) return nl_android_video_error(ctx, "start_failed", status);
  ctx->codec_started = true;
  ctx->has_output = false;
  ctx->codec_started_us = ctx->last_progress_us = LiGetMicroseconds();
  nl_android_video_log(ctx, "codec_started", "awaiting_csd_and_output", 0, 0, 0);
  return true;
}

static int nl_android_csd_index(int type) {
  switch (type) {
    case BUFFER_TYPE_SPS: return 0;
    case BUFFER_TYPE_PPS: return 1;
    case BUFFER_TYPE_VPS: return 2;
    default: return -1;
  }
}

static bool nl_android_has_start_code(const uint8_t* data, size_t size) {
  return (size >= 3 && data[0] == 0 && data[1] == 0 && data[2] == 1) ||
         (size >= 4 && data[0] == 0 && data[1] == 0 && data[2] == 0 && data[3] == 1);
}

static bool nl_android_video_read_csd(nl_android_video_context_t* ctx,
                                      const DECODE_UNIT* du, size_t* picture_size) {
  size_t total = 0;
  unsigned int entries = 0;
  bool changed = false;
  *picture_size = 0;
  if (du == NULL || du->fullLength <= 0 ||
      du->fullLength > (int)NL_ANDROID_MAX_FRAME_BYTES) return false;
  for (PLENTRY entry = du->bufferList; entry != NULL; entry = entry->next) {
    int index = nl_android_csd_index(entry->bufferType);
    if (++entries > 65536U || entry->data == NULL || entry->length <= 0 ||
        (size_t)entry->length > NL_ANDROID_MAX_FRAME_BYTES - total) return false;
    total += (size_t)entry->length;
    if (index >= 0) {
      static const uint8_t start_code[] = {0, 0, 0, 1};
      bool annex_b = nl_android_has_start_code((const uint8_t*)entry->data, (size_t)entry->length);
      size_t prefix = annex_b ? 0U : sizeof(start_code);
      size_t size = (size_t)entry->length + prefix;
      if (du->frameType != FRAME_TYPE_IDR || size > NL_ANDROID_MAX_CSD_BYTES) return false;
      if (ctx->csd_size[index] != size ||
          (prefix != 0 && memcmp(ctx->csd[index], start_code, prefix) != 0) ||
          memcmp(ctx->csd[index] + prefix, entry->data, (size_t)entry->length) != 0) {
        if (prefix != 0) memcpy(ctx->csd[index], start_code, prefix);
        memcpy(ctx->csd[index] + prefix, entry->data, (size_t)entry->length);
        ctx->csd_size[index] = size;
        changed = true;
      }
    } else if (entry->bufferType == BUFFER_TYPE_PICDATA) {
      *picture_size += (size_t)entry->length;
    } else {
      return false;
    }
  }
  if (changed && ctx->codec_started) nl_android_video_recover(ctx, "csd_changed", du->frameNumber, 0);
  return total == (size_t)du->fullLength && *picture_size != 0;
}

static bool nl_android_video_drain(nl_video_renderer_t* renderer, nl_android_video_context_t* ctx) {
  if (!ctx->codec_started) return true;
  // Bound even a misbehaving codec's output loop so stop and detach can proceed.
  for (unsigned int i = 0; i < 32U; ++i) {
    AMediaCodecBufferInfo info;
    ssize_t index = AMediaCodec_dequeueOutputBuffer(ctx->codec, &info, 0);
    if (index >= 0) {
      bool render = (info.flags & (AMEDIACODEC_BUFFER_FLAG_CODEC_CONFIG | AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM)) == 0;
      media_status_t status = AMediaCodec_releaseOutputBuffer(ctx->codec, (size_t)index, render);
      if (status != AMEDIA_OK) return nl_android_video_error(ctx, "release_output_failed", status);
      if (render) {
        __atomic_add_fetch(&renderer->decoded_output_frame_count, 1U, __ATOMIC_RELAXED);
        if (!ctx->has_output) {
          ctx->has_output = true;
          nl_android_video_log(ctx, "output_ready", "first_decoded_frame", 0, 0, 0);
        }
        if (ctx->in_flight > 0) --ctx->in_flight;
        uint64_t now = LiGetMicroseconds();
        nl_latency_telemetry_record_decoder_output(&renderer->telemetry, (uint64_t)info.presentationTimeUs, now, 0, false);
        nl_latency_telemetry_record_render_submit(&renderer->telemetry, (uint64_t)info.presentationTimeUs, now);
      }
      ctx->last_progress_us = LiGetMicroseconds();
      if ((info.flags & AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM) != 0) {
        return nl_android_video_error(ctx, "unexpected_eos", info.flags);
      }
    } else if (index == AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED) {
      AMediaFormat* format = AMediaCodec_getOutputFormat(ctx->codec);
      if (format != NULL) AMediaFormat_delete(format);
      nl_android_video_log(ctx, "output_format", "format_changed", 0, 0, 0);
    } else if (index == AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED) {
      continue;
    } else {
      if (index == AMEDIACODEC_INFO_TRY_AGAIN_LATER) return true;
      return nl_android_video_error(ctx, "dequeue_output_failed", index);
    }
  }
  return true;
}

typedef enum nl_android_input_result {
  NL_ANDROID_INPUT_READY,
  NL_ANDROID_INPUT_CANCELED,
  NL_ANDROID_INPUT_EXPIRED,
  NL_ANDROID_INPUT_FAILED,
} nl_android_input_result_t;

// Enter/return holding mutex. The caller keeps the single core-owned frame
// until success or a bounded deadline; a 2 ms TRY_AGAIN is not a decode failure.
static nl_android_input_result_t nl_android_video_wait_input(nl_video_renderer_t* renderer,
    nl_android_video_context_t* ctx, int frame, ssize_t* input_index) {
  uint64_t generation = ctx->generation;
  uint64_t before = LiGetMicroseconds();
  uint64_t budget = ctx->has_output ? NL_ANDROID_INPUT_RETRY_US : NL_ANDROID_STARTUP_RETRY_US;
  bool logged = false;
  const struct timespec idle = {0, 1000000};
  for (;;) {
    if (!atomic_load(&ctx->running) || ctx->window == NULL || generation != ctx->generation) {
      nl_android_video_log(ctx, "input_canceled", "stop_or_surface_change", frame, 0, LiGetMicroseconds() - before);
      return NL_ANDROID_INPUT_CANCELED;
    }
    ctx->pending_core = LiGetPendingVideoFrames();
    if (!nl_android_video_drain(renderer, ctx)) return NL_ANDROID_INPUT_FAILED;
    bool full = ctx->in_flight >= NL_ANDROID_MAX_IN_FLIGHT;
    ssize_t index = full ? AMEDIACODEC_INFO_TRY_AGAIN_LATER
        : AMediaCodec_dequeueInputBuffer(ctx->codec, NL_ANDROID_INPUT_TIMEOUT_US);
    uint64_t waited = LiGetMicroseconds() - before;
    if (index >= 0) {
      if (logged) nl_android_video_log(ctx, "input_resumed", "buffer_available", frame, index, waited);
      nl_latency_telemetry_record_backpressure(&renderer->telemetry, waited, false);
      *input_index = index;
      return NL_ANDROID_INPUT_READY;
    }
    if (index != AMEDIACODEC_INFO_TRY_AGAIN_LATER) {
      nl_android_video_error(ctx, "dequeue_input_failed", index);
      return NL_ANDROID_INPUT_FAILED;
    }
    const char* reason = full ? "inflight_limit" : "input_busy_csd_or_decode";
    if (waited >= budget) {
      nl_android_video_log(ctx, "input_deadline_request_idr", reason, frame, index, waited);
      nl_latency_telemetry_record_backpressure(&renderer->telemetry, waited, true);
      return NL_ANDROID_INPUT_EXPIRED;
    }
    if (!logged && (!ctx->has_output || waited >= 10000U)) {
      nl_android_video_log(ctx, "input_wait", reason, frame, index, waited);
      logged = true;
    }
    // Detach must not wait for the entire retry budget. The generation check
    // prevents this held frame from reaching a replaced/re-attached window.
    pthread_mutex_unlock(&ctx->mutex);
    nanosleep(&idle, NULL);
    pthread_mutex_lock(&ctx->mutex);
  }
}

int nl_video_renderer_platform_submit_frame(nl_video_renderer_t* renderer, const void* raw_du,
                                             const nl_video_frame_metadata_t* frame) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  const DECODE_UNIT* du = raw_du;
  size_t picture_size = 0;
  int result = DR_NEED_IDR;
  if (ctx == NULL || du == NULL || frame == NULL) return result;
  pthread_mutex_lock(&ctx->mutex);
  if (!atomic_load(&ctx->running) || ctx->window == NULL) goto done;
  ctx->pending_core = LiGetPendingVideoFrames();
  ctx->error_reason = "invalid_decode_unit";
  ctx->error_status = du->fullLength;
  if (!nl_android_video_read_csd(ctx, du, &picture_size)) goto reset;
  if (ctx->need_idr && du->frameType != FRAME_TYPE_IDR) goto done;
  if (!ctx->codec_started && !nl_android_video_configure(ctx)) goto reset;

  // Core backlog alone is not evidence of a failed decoder. In particular,
  // CSD consumption and cold-start allocation can briefly fill the core queue.
  ssize_t index;
  nl_android_input_result_t input = nl_android_video_wait_input(renderer, ctx, du->frameNumber, &index);
  if (input == NL_ANDROID_INPUT_FAILED) goto reset;
  if (input == NL_ANDROID_INPUT_EXPIRED) {
    // The core flushes its stale queue on DR_NEED_IDR. Keep the healthy codec,
    // continue draining outstanding output, and resume on the requested IDR.
    ctx->need_idr = true;
    goto done;
  }
  if (input == NL_ANDROID_INPUT_CANCELED) goto done;
  size_t capacity = 0;
  uint8_t* buffer = AMediaCodec_getInputBuffer(ctx->codec, (size_t)index, &capacity);
  if (buffer == NULL || picture_size > capacity) {
    nl_android_video_error(ctx, buffer == NULL ? "input_buffer_null" : "input_buffer_too_small", (int64_t)capacity);
    fprintf(stderr, "[noland-video] input_size frame=%d picture_bytes=%zu capacity=%zu\n",
        du->frameNumber, picture_size, capacity);
    goto reset;
  }
  size_t offset = 0;
  for (PLENTRY entry = du->bufferList; entry != NULL; entry = entry->next) {
    if (entry->bufferType != BUFFER_TYPE_PICDATA) continue;
    memcpy(buffer + offset, entry->data, (size_t)entry->length);
    offset += (size_t)entry->length;
  }
  media_status_t status = AMediaCodec_queueInputBuffer(ctx->codec, (size_t)index, 0, offset,
                                                      du->presentationTimeUs, 0);
  if (status != AMEDIA_OK) {
    nl_android_video_error(ctx, "queue_input_failed", status);
    goto reset;
  }
  if (ctx->in_flight == 0) ctx->last_progress_us = LiGetMicroseconds();
  ++ctx->in_flight;
  ctx->need_idr = false;
  nl_latency_telemetry_set_queue_depths(&renderer->telemetry, (uint16_t)ctx->in_flight, 0);
  result = DR_OK;
  goto done;

reset:
  // Only fatal API/data errors recreate the codec here, never TRY_AGAIN/backlog.
  nl_android_video_recover(ctx, ctx->error_reason, du->frameNumber, ctx->error_status);
done:
  pthread_mutex_unlock(&ctx->mutex);
  return result;
}

static void* nl_android_video_thread(void* data) {
  nl_video_renderer_t* renderer = data;
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  const struct timespec idle = {0, 1000000};
  while (atomic_load(&ctx->running)) {
    VIDEO_FRAME_HANDLE handle;
    PDECODE_UNIT du;
    bool request_idr = false;
    pthread_mutex_lock(&ctx->mutex);
    ctx->pending_core = LiGetPendingVideoFrames();
    if (!nl_android_video_drain(renderer, ctx)) {
      nl_android_video_recover(ctx, ctx->error_reason, 0, ctx->error_status);
      request_idr = true;
    } else if (ctx->in_flight != 0 && LiGetMicroseconds() - ctx->last_progress_us >
               (ctx->has_output ? NL_ANDROID_STALL_US : NL_ANDROID_STARTUP_STALL_US)) {
      nl_android_video_recover(ctx, ctx->has_output ? "output_stalled" : "startup_no_output", 0, 0);
      request_idr = true;
    }
    pthread_mutex_unlock(&ctx->mutex);
    if (request_idr) LiRequestIdrFrame();

    // Polling keeps output draining during receive gaps and makes shutdown
    // independent of the core queue's user-wake consumption semantics.
    if (!LiPollNextVideoFrame(&handle, &du)) {
      nanosleep(&idle, NULL);
      continue;
    }
    int result = DR_NEED_IDR;
    if (du != NULL && atomic_load(&ctx->running)) {
      nl_video_frame_metadata_t frame = {
        .frame_number = du->frameNumber,
        .frame_type = du->frameType,
        .full_length = du->fullLength,
        .host_processing_latency = du->frameHostProcessingLatency,
        .receive_time_us = du->receiveTimeUs,
        .enqueue_time_us = du->enqueueTimeUs,
        .presentation_time_us = du->presentationTimeUs,
        .rtp_timestamp = du->rtpTimestamp,
        .hdr_active = du->hdrActive ? 1U : 0U,
        .colorspace = du->colorspace,
      };
      result = renderer->frame_processor != NULL
          ? renderer->frame_processor(renderer->frame_processor_user_data, du, &frame)
          : nl_video_renderer_submit_frame(renderer, du, &frame);
    }
    // The only completion site: no core-owned pointer survives this iteration.
    LiCompleteVideoFrame(handle, result);
  }
  return NULL;
}

void nl_video_renderer_platform_attach_surface(nl_video_renderer_t* renderer,
                                               const nl_surface_descriptor_t* surface) {
  nl_android_video_context_t* ctx = nl_android_video_ensure_context(renderer);
  if (ctx == NULL) return;
  pthread_mutex_lock(&ctx->mutex);
  ANativeWindow* window = surface != NULL && surface->surface_type == NL_SURFACE_ANDROID_NATIVE_WINDOW
      ? (ANativeWindow*)surface->window_handle : NULL;
  if (window != ctx->window) {
    nl_android_video_recover(ctx, "surface_changed", 0, 0);
    ctx->window = window;
  }
  pthread_mutex_unlock(&ctx->mutex);
  // The next pulled frame completes with DR_NEED_IDR if it isn't a fresh IDR.
}

void nl_video_renderer_platform_detach_surface(nl_video_renderer_t* renderer) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  if (ctx == NULL) return;
  pthread_mutex_lock(&ctx->mutex);
  nl_android_video_recover(ctx, "surface_detached", 0, 0);
  ctx->window = NULL;
  pthread_mutex_unlock(&ctx->mutex);
}

int nl_video_renderer_platform_setup(nl_video_renderer_t* renderer, int video_format,
                                    int width, int height, int redraw_rate) {
  const char* mime = nl_android_video_mime(video_format);
  nl_android_video_context_t* ctx = nl_android_video_ensure_context(renderer);
  if (ctx == NULL || mime == NULL || width <= 0 || height <= 0 || redraw_rate <= 0) return -1;
  nl_video_renderer_platform_stop(renderer);
  pthread_mutex_lock(&ctx->mutex);
  nl_android_video_reset(ctx);
  memset(ctx->csd_size, 0, sizeof(ctx->csd_size));
  ctx->video_format = video_format;
  ctx->width = width;
  ctx->height = height;
  ctx->fps = redraw_rate;
  // Fail setup if this device has no decoder. Configuration waits for IDR CSD.
  ctx->codec = AMediaCodec_createDecoderByType(mime);
  bool available = ctx->codec != NULL;
  pthread_mutex_unlock(&ctx->mutex);
  if (!available) fprintf(stderr, "[noland-video] No MediaCodec decoder for %s\n", mime);
  return available ? 0 : -1;
}

void nl_video_renderer_platform_start(nl_video_renderer_t* renderer) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  if (ctx == NULL || ctx->thread_started) return;
  atomic_store(&ctx->running, true);
  int error = pthread_create(&ctx->thread, NULL, nl_android_video_thread, renderer);
  if (error != 0) {
    atomic_store(&ctx->running, false);
    renderer->started = false;
    fprintf(stderr, "[noland-video] Cannot start decoder thread: %d\n", error);
    return;
  }
  ctx->thread_started = true;
}

void nl_video_renderer_platform_stop(nl_video_renderer_t* renderer) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  if (ctx == NULL) return;
  atomic_store(&ctx->running, false);
  if (ctx->thread_started) {
    pthread_join(ctx->thread, NULL);
    ctx->thread_started = false;
  }
  pthread_mutex_lock(&ctx->mutex);
  nl_android_video_reset(ctx);
  pthread_mutex_unlock(&ctx->mutex);
}

void nl_video_renderer_platform_cleanup(nl_video_renderer_t* renderer) {
  nl_android_video_context_t* ctx = nl_android_video_context(renderer);
  if (ctx == NULL) return;
  nl_video_renderer_platform_stop(renderer);
  pthread_mutex_destroy(&ctx->mutex);
  free(ctx);
  renderer->platform_context = NULL;
}

void nl_video_renderer_platform_set_overlay_text(nl_video_renderer_t* renderer, const char* text) {
  // The native Surface is exclusively owned by MediaCodec; overlays belong to UI.
  (void)renderer;
  (void)text;
}

#endif
