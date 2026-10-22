// Includes the backend to exercise its actual state machine with a fake NDK.
// Run with: python3 tests/run_android_video_renderer_tests.py
#include <assert.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifndef NOLAND_ANDROID_VIDEO_SOURCE
#define NOLAND_ANDROID_VIDEO_SOURCE "../src/noland_video_renderer_android.c"
#endif
#include NOLAND_ANDROID_VIDEO_SOURCE

struct AMediaCodec { uint8_t input[1024]; };
struct AMediaFormat { int unused; };

static struct {
  atomic_int creates, stops, deletes, queued, input_calls, polls, completions, idrs;
  atomic_int busy_left, always_busy, input_error, output_error, queue_error, configure_error;
  atomic_int output_tokens, capacity, pending_frames, core_ready, completion_status;
  DECODE_UNIT* core_unit;
} fake;

uint64_t LiGetMicroseconds(void) {
  struct timespec now;
  clock_gettime(CLOCK_MONOTONIC, &now);
  return (uint64_t)now.tv_sec * 1000000U + (uint64_t)now.tv_nsec / 1000U;
}

static void sleep_us(unsigned int us) {
  struct timespec delay = {(time_t)(us / 1000000U), (long)(us % 1000000U) * 1000L};
  nanosleep(&delay, NULL);
}

int LiGetPendingVideoFrames(void) { return atomic_load(&fake.pending_frames); }
void LiRequestIdrFrame(void) { atomic_fetch_add(&fake.idrs, 1); }
bool LiPollNextVideoFrame(VIDEO_FRAME_HANDLE* handle, PDECODE_UNIT* unit) {
  if (atomic_exchange(&fake.core_ready, 0) == 0) return false;
  *handle = fake.core_unit;
  *unit = fake.core_unit;
  atomic_fetch_add(&fake.polls, 1);
  return true;
}
void LiCompleteVideoFrame(VIDEO_FRAME_HANDLE handle, int status) {
  assert(handle == fake.core_unit);
  atomic_store(&fake.completion_status, status);
  atomic_fetch_add(&fake.completions, 1);
}

AMediaCodec* AMediaCodec_createDecoderByType(const char* mime) {
  assert(strcmp(mime, "video/avc") == 0);
  atomic_fetch_add(&fake.creates, 1);
  return calloc(1, sizeof(AMediaCodec));
}
media_status_t AMediaCodec_configure(AMediaCodec* codec, const AMediaFormat* format,
    ANativeWindow* window, void* crypto, uint32_t flags) {
  (void)crypto; (void)flags;
  assert(codec && format && window);
  return atomic_load(&fake.configure_error);
}
media_status_t AMediaCodec_start(AMediaCodec* codec) { assert(codec); return AMEDIA_OK; }
media_status_t AMediaCodec_stop(AMediaCodec* codec) {
  assert(codec); atomic_fetch_add(&fake.stops, 1); return AMEDIA_OK;
}
media_status_t AMediaCodec_delete(AMediaCodec* codec) {
  assert(codec); atomic_fetch_add(&fake.deletes, 1); free(codec); return AMEDIA_OK;
}
ssize_t AMediaCodec_dequeueInputBuffer(AMediaCodec* codec, int64_t timeout_us) {
  assert(codec && timeout_us <= 2000);
  atomic_fetch_add(&fake.input_calls, 1);
  if (atomic_load(&fake.input_error) != 0) return atomic_load(&fake.input_error);
  if (atomic_load(&fake.always_busy) || atomic_load(&fake.busy_left) > 0) {
    if (atomic_load(&fake.busy_left) > 0) atomic_fetch_sub(&fake.busy_left, 1);
    sleep_us(2000);
    return AMEDIACODEC_INFO_TRY_AGAIN_LATER;
  }
  return 0;
}
uint8_t* AMediaCodec_getInputBuffer(AMediaCodec* codec, size_t index, size_t* capacity) {
  assert(codec && index == 0);
  *capacity = (size_t)atomic_load(&fake.capacity);
  return codec->input;
}
media_status_t AMediaCodec_queueInputBuffer(AMediaCodec* codec, size_t index,
    off_t offset, size_t size, uint64_t time_us, uint32_t flags) {
  (void)time_us; (void)flags;
  assert(codec && index == 0 && offset == 0 && size > 0 && size <= sizeof(codec->input));
  if (atomic_load(&fake.queue_error)) return atomic_load(&fake.queue_error);
  atomic_fetch_add(&fake.queued, 1);
  return AMEDIA_OK;
}
ssize_t AMediaCodec_dequeueOutputBuffer(AMediaCodec* codec, AMediaCodecBufferInfo* info, int64_t timeout_us) {
  assert(codec && timeout_us == 0);
  if (atomic_load(&fake.output_error)) return atomic_load(&fake.output_error);
  if (atomic_load(&fake.output_tokens) == 0) return AMEDIACODEC_INFO_TRY_AGAIN_LATER;
  atomic_fetch_sub(&fake.output_tokens, 1);
  *info = (AMediaCodecBufferInfo){.size = 8, .presentationTimeUs = 1000};
  return 0;
}
media_status_t AMediaCodec_releaseOutputBuffer(AMediaCodec* codec, size_t index, bool render) {
  assert(codec && index == 0 && render); return AMEDIA_OK;
}
AMediaFormat* AMediaCodec_getOutputFormat(AMediaCodec* codec) { assert(codec); return AMediaFormat_new(); }
AMediaFormat* AMediaFormat_new(void) { return calloc(1, sizeof(AMediaFormat)); }
media_status_t AMediaFormat_delete(AMediaFormat* format) { free(format); return AMEDIA_OK; }
void AMediaFormat_setString(AMediaFormat* f, const char* n, const char* v) { (void)f; (void)n; (void)v; }
void AMediaFormat_setInt32(AMediaFormat* f, const char* n, int32_t v) { (void)f; (void)n; (void)v; }
void AMediaFormat_setBuffer(AMediaFormat* f, const char* n, const void* v, size_t s) { (void)f; (void)n; (void)v; (void)s; }

void nl_latency_telemetry_record_decoder_output(nl_latency_telemetry_t* t, uint64_t p, uint64_t n, uint16_t q, bool b) {
  (void)t; (void)p; (void)n; (void)q; (void)b;
}
void nl_latency_telemetry_record_render_submit(nl_latency_telemetry_t* t, uint64_t p, uint64_t n) {
  (void)t; (void)p; (void)n;
}
void nl_latency_telemetry_record_backpressure(nl_latency_telemetry_t* t, uint64_t d, bool a) {
  (void)t; (void)d; (void)a;
}
void nl_latency_telemetry_set_queue_depths(nl_latency_telemetry_t* t, uint16_t d, uint16_t r) {
  (void)t; (void)d; (void)r;
}
int nl_video_renderer_submit_frame(nl_video_renderer_t* r, const void* du, const nl_video_frame_metadata_t* f) {
  return nl_video_renderer_platform_submit_frame(r, du, f);
}

static char sps[] = {0, 0, 0, 1, 0x67, 1};
static char pps[] = {0, 0, 0, 1, 0x68, 1};
static char pic[] = {0, 0, 0, 1, 0x65, 1};
static LENTRY picture = {NULL, pic, sizeof(pic), BUFFER_TYPE_PICDATA};
static LENTRY pps_entry = {&picture, pps, sizeof(pps), BUFFER_TYPE_PPS};
static LENTRY sps_entry = {&pps_entry, sps, sizeof(sps), BUFFER_TYPE_SPS};

static DECODE_UNIT unit(bool idr) {
  return (DECODE_UNIT){.frameNumber = idr ? 1 : 2,
      .frameType = idr ? FRAME_TYPE_IDR : FRAME_TYPE_PFRAME,
      .fullLength = (int)(idr ? sizeof(sps) + sizeof(pps) + sizeof(pic) : sizeof(pic)),
      .bufferList = idr ? &sps_entry : &picture, .presentationTimeUs = 1000};
}
static int submit(nl_video_renderer_t* renderer, bool idr) {
  DECODE_UNIT du = unit(idr);
  nl_video_frame_metadata_t frame = {.frame_number = du.frameNumber, .presentation_time_us = 1000};
  return nl_video_renderer_platform_submit_frame(renderer, &du, &frame);
}
static void init(nl_video_renderer_t* renderer) {
  // No worker/callback may be active across a fake-state reset.
  memset(&fake, 0, sizeof(fake));
  atomic_store(&fake.capacity, 1024);
  memset(renderer, 0, sizeof(*renderer));
  nl_surface_descriptor_t surface = {.surface_type = NL_SURFACE_ANDROID_NATIVE_WINDOW,
      .window_handle = (void*)(uintptr_t)1};
  nl_video_renderer_platform_attach_surface(renderer, &surface);
  assert(nl_video_renderer_platform_setup(renderer, VIDEO_FORMAT_H264, 1920, 1080, 60) == 0);
  atomic_store(&((nl_android_video_context_t*)renderer->platform_context)->running, true);
}

static void startup_retry(void) {
  nl_video_renderer_t renderer;
  init(&renderer);
  atomic_store(&fake.busy_left, 8); // CSD occupies input buffers for > 2 ms.
  assert(submit(&renderer, true) == DR_OK);
  assert(atomic_load(&fake.input_calls) == 9);
  assert(atomic_load(&fake.queued) == 1);
  assert(atomic_load(&fake.creates) == 1 && atomic_load(&fake.deletes) == 0);
  atomic_store(&fake.busy_left, 4);
  assert(submit(&renderer, false) == DR_OK); // Preserve this dependent P-frame.
  assert(atomic_load(&fake.queued) == 2 && atomic_load(&fake.deletes) == 0);
  nl_video_renderer_platform_cleanup(&renderer);
  puts("PASS startup CSD/input retries preserve codec and held IDR/P-frame");
}
static void backlog_not_failure(void) {
  nl_video_renderer_t renderer;
  init(&renderer);
  atomic_store(&fake.pending_frames, 15);
  assert(submit(&renderer, true) == DR_OK);
  assert(submit(&renderer, false) == DR_OK);
  assert(atomic_load(&fake.queued) == 2 && atomic_load(&fake.deletes) == 0);
  nl_video_renderer_platform_cleanup(&renderer);
  puts("PASS core backlog does not reset or drop decodable frames");
}

#if !defined(NOLAND_ANDROID_VIDEO_BASELINE)
static void wait_at_least(atomic_int* value, int minimum) {
  uint64_t deadline = LiGetMicroseconds() + 2000000U;
  while (atomic_load(value) < minimum && LiGetMicroseconds() < deadline) sleep_us(1000);
  assert(atomic_load(value) >= minimum);
}
static void deadline_keeps_codec(void) {
  nl_video_renderer_t renderer;
  init(&renderer);
  assert(submit(&renderer, true) == DR_OK);
  nl_android_video_context_t* ctx = renderer.platform_context;
  ctx->has_output = true;
  AMediaCodec* codec = ctx->codec;
  atomic_store(&fake.always_busy, 1);
  uint64_t before = LiGetMicroseconds();
  assert(submit(&renderer, false) == DR_NEED_IDR);
  uint64_t elapsed = LiGetMicroseconds() - before;
  assert(elapsed >= NL_ANDROID_INPUT_RETRY_US && elapsed < 300000U);
  assert(ctx->codec == codec && ctx->need_idr && atomic_load(&fake.deletes) == 0);
  atomic_store(&fake.always_busy, 0);
  assert(submit(&renderer, true) == DR_OK);
  assert(ctx->codec == codec && atomic_load(&fake.creates) == 1);
  nl_video_renderer_platform_cleanup(&renderer);
  puts("PASS bounded deadline requests IDR without codec recreation");
}
static void* release_output(void* unused) {
  (void)unused;
  sleep_us(20000);
  atomic_store(&fake.output_tokens, NL_ANDROID_MAX_IN_FLIGHT);
  return NULL;
}
static void inflight_retry(void) {
  nl_video_renderer_t renderer;
  init(&renderer);
  assert(submit(&renderer, true) == DR_OK);
  nl_android_video_context_t* ctx = renderer.platform_context;
  ctx->has_output = true;
  ctx->in_flight = NL_ANDROID_MAX_IN_FLIGHT;
  pthread_t release;
  assert(pthread_create(&release, NULL, release_output, NULL) == 0);
  assert(submit(&renderer, false) == DR_OK);
  pthread_join(release, NULL);
  assert(ctx->in_flight == 1 && atomic_load(&fake.queued) == 2 && atomic_load(&fake.deletes) == 0);
  nl_video_renderer_platform_cleanup(&renderer);
  puts("PASS in-flight cap drains output and retries same frame");
}
static void lifecycle_interrupt(bool detach) {
  nl_video_renderer_t renderer;
  init(&renderer);
  DECODE_UNIT du = unit(true);
  fake.core_unit = &du;
  atomic_store(&fake.always_busy, 1);
  atomic_store(&fake.core_ready, 1);
  nl_video_renderer_platform_start(&renderer);
  wait_at_least(&fake.input_calls, 3);
  uint64_t before = LiGetMicroseconds();
  if (detach) {
    nl_video_renderer_platform_detach_surface(&renderer);
    nl_surface_descriptor_t new_surface = {.surface_type = NL_SURFACE_ANDROID_NATIVE_WINDOW,
        .window_handle = (void*)(uintptr_t)2};
    nl_video_renderer_platform_attach_surface(&renderer, &new_surface);
    wait_at_least(&fake.completions, 1);
  } else {
    nl_video_renderer_platform_stop(&renderer);
  }
  assert(LiGetMicroseconds() - before < 100000U);
  assert(atomic_load(&fake.polls) == 1 && atomic_load(&fake.completions) == 1);
  assert(atomic_load(&fake.completion_status) == DR_NEED_IDR && atomic_load(&fake.queued) == 0);
  assert(atomic_load(&fake.creates) == 1); // Never send the old frame to the new window.
  nl_video_renderer_platform_cleanup(&renderer);
  puts(detach ? "PASS detach/re-attach cancels held frame by generation, exactly once" :
      "PASS stop/join interrupts retry promptly, exactly-once completion");
}
static void fatal_errors(void) {
  for (int error = 0; error < 5; ++error) {
    nl_video_renderer_t renderer;
    init(&renderer);
    const char* expected;
    if (error == 0) {atomic_store(&fake.input_error, -1234); expected = "dequeue_input_failed";}
    else if (error == 1) {atomic_store(&fake.output_error, -1235); expected = "dequeue_output_failed";}
    else if (error == 2) {atomic_store(&fake.queue_error, -1236); expected = "queue_input_failed";}
    else if (error == 3) {atomic_store(&fake.capacity, 1); expected = "input_buffer_too_small";}
    else {atomic_store(&fake.configure_error, -1237); expected = "configure_failed";}
    assert(submit(&renderer, true) == DR_NEED_IDR);
    nl_android_video_context_t* ctx = renderer.platform_context;
    assert(ctx->codec == NULL && ctx->need_idr && strcmp(ctx->error_reason, expected) == 0);
    assert(atomic_load(&fake.deletes) == 1 && atomic_load(&fake.queued) == 0);
    nl_video_renderer_platform_cleanup(&renderer);
  }
  puts("PASS fatal input/output/configure/queue/capacity failures have distinct reasons");
}
static void startup_watchdog(void) {
  nl_video_renderer_t renderer;
  init(&renderer);
  assert(submit(&renderer, true) == DR_OK);
  nl_android_video_context_t* ctx = renderer.platform_context;
  ctx->last_progress_us = LiGetMicroseconds() - 300000U;
  nl_video_renderer_platform_start(&renderer);
  sleep_us(20000);
  assert(atomic_load(&fake.deletes) == 0); // Not the former 100 ms startup timeout.
  pthread_mutex_lock(&ctx->mutex);
  ctx->last_progress_us = LiGetMicroseconds() - NL_ANDROID_STARTUP_STALL_US - 1U;
  pthread_mutex_unlock(&ctx->mutex);
  wait_at_least(&fake.idrs, 1);
  assert(atomic_load(&fake.deletes) == 1);
  nl_video_renderer_platform_cleanup(&renderer);
  puts("PASS startup grace, followed by bounded genuine no-output recovery");
}
#endif

int main(int argc, char** argv) {
  setbuf(stdout, NULL);
  if (argc > 1 && strcmp(argv[1], "backlog") == 0) {backlog_not_failure(); return 0;}
  startup_retry();
  backlog_not_failure();
#if !defined(NOLAND_ANDROID_VIDEO_BASELINE)
  deadline_keeps_codec();
  inflight_retry();
  lifecycle_interrupt(false);
  lifecycle_interrupt(true);
  fatal_errors();
  startup_watchdog();
#endif
  puts("ALL ANDROID VIDEO REGRESSION TESTS PASSED");
  return 0;
}
