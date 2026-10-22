#if defined(__ANDROID__)

#include "noland_audio_renderer.h"

#include <SLES/OpenSLES.h>
#include <SLES/OpenSLES_Android.h>
#if __has_include(<opus/opus_multistream.h>)
#include <opus/opus_multistream.h>
#else
#include <opus_multistream.h>
#endif
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define NL_ANDROID_AUDIO_MAX_SLOTS 64U
#define NL_ANDROID_AUDIO_MAX_BUFFER_MS 200U

typedef struct nl_android_audio_context {
  pthread_mutex_t mutex;
  SLObjectItf engine_object;
  SLEngineItf engine;
  SLObjectItf output_mix;
  SLObjectItf player;
  SLPlayItf play;
  SLAndroidSimpleBufferQueueItf queue;
  OpusMSDecoder* decoder;
  int16_t* decoded;
  int16_t* buffers;
  uint32_t slot_samples[NL_ANDROID_AUDIO_MAX_SLOTS];
  uint32_t capacity;
  uint32_t head;
  uint32_t count;
  uint32_t queued_samples;
  uint32_t target_samples;
  uint32_t maximum_samples;
  int samples_per_frame;
  int channels;
  bool running;
  bool playing;
  bool failed;
} nl_android_audio_context_t;

// Reap completed slots using OpenSL's queue count. No audio-thread callback
// accesses our memory, so stop/clear/destroy cannot deadlock on a callback lock.
static SLresult nl_android_audio_reap(nl_android_audio_context_t* ctx) {
  SLAndroidSimpleBufferQueueState state;
  SLresult result = (*ctx->queue)->GetState(ctx->queue, &state);
  if (result != SL_RESULT_SUCCESS) return result;
  if (state.count > ctx->count) return SL_RESULT_INTERNAL_ERROR;
  while (ctx->count > state.count) {
    ctx->queued_samples -= ctx->slot_samples[ctx->head];
    ctx->head = (ctx->head + 1U) % ctx->capacity;
    --ctx->count;
  }
  return SL_RESULT_SUCCESS;
}

static void nl_android_audio_destroy(nl_android_audio_context_t* ctx) {
  if (ctx == NULL) return;
  // Player destruction releases every borrowed PCM slot before freeing buffers.
  if (ctx->player != NULL) (*ctx->player)->Destroy(ctx->player);
  if (ctx->output_mix != NULL) (*ctx->output_mix)->Destroy(ctx->output_mix);
  if (ctx->engine_object != NULL) (*ctx->engine_object)->Destroy(ctx->engine_object);
  if (ctx->decoder != NULL) opus_multistream_decoder_destroy(ctx->decoder);
  free(ctx->decoded);
  free(ctx->buffers);
  pthread_mutex_destroy(&ctx->mutex);
  free(ctx);
}

static SLuint32 nl_android_audio_channel_mask(int channels) {
  switch (channels) {
    case 1: return SL_SPEAKER_FRONT_CENTER;
    case 2: return SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT;
    case 6: return SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT |
        SL_SPEAKER_FRONT_CENTER | SL_SPEAKER_LOW_FREQUENCY |
        SL_SPEAKER_BACK_LEFT | SL_SPEAKER_BACK_RIGHT;
    case 8: return SL_SPEAKER_FRONT_LEFT | SL_SPEAKER_FRONT_RIGHT |
        SL_SPEAKER_FRONT_CENTER | SL_SPEAKER_LOW_FREQUENCY |
        SL_SPEAKER_BACK_LEFT | SL_SPEAKER_BACK_RIGHT |
        SL_SPEAKER_SIDE_LEFT | SL_SPEAKER_SIDE_RIGHT;
    default: return 0;
  }
}

int nl_audio_renderer_init(nl_audio_renderer_t* renderer, int audio_configuration,
                           const POPUS_MULTISTREAM_CONFIGURATION config, int ar_flags) {
  (void)ar_flags;
  if (renderer == NULL || config == NULL ||
      nl_android_audio_channel_mask(config->channelCount) == 0 ||
      CHANNEL_COUNT_FROM_AUDIO_CONFIGURATION(audio_configuration) != config->channelCount ||
      (config->sampleRate != 8000 && config->sampleRate != 12000 && config->sampleRate != 16000 &&
       config->sampleRate != 24000 && config->sampleRate != 48000) ||
      config->samplesPerFrame <= 0 || config->samplesPerFrame > config->sampleRate * 120 / 1000) return -1;

  uint32_t target_ms = renderer->target_buffer_ms != 0 ? renderer->target_buffer_ms : 20U;
  uint32_t maximum_ms = renderer->maximum_buffer_ms != 0 ? renderer->maximum_buffer_ms : 80U;
  if (target_ms > NL_ANDROID_AUDIO_MAX_BUFFER_MS || maximum_ms > NL_ANDROID_AUDIO_MAX_BUFFER_MS) {
    fprintf(stderr, "[noland-audio] Requested buffer exceeds %u ms hard limit\n", NL_ANDROID_AUDIO_MAX_BUFFER_MS);
    return -1;
  }
  if (maximum_ms < target_ms) maximum_ms = target_ms;
  uint32_t frame_ms = ((uint32_t)config->samplesPerFrame * 1000U + (uint32_t)config->sampleRate - 1U) /
      (uint32_t)config->sampleRate;
  if (maximum_ms < frame_ms) maximum_ms = frame_ms;
  nl_audio_renderer_cleanup(renderer);

  nl_android_audio_context_t* ctx = calloc(1, sizeof(*ctx));
  if (ctx == NULL) return -1;
  if (pthread_mutex_init(&ctx->mutex, NULL) != 0) {
    free(ctx);
    return -1;
  }
  ctx->samples_per_frame = config->samplesPerFrame;
  ctx->channels = config->channelCount;
  ctx->target_samples = target_ms * (uint32_t)config->sampleRate / 1000U;
  ctx->maximum_samples = maximum_ms * (uint32_t)config->sampleRate / 1000U;
  ctx->capacity = ctx->maximum_samples / (uint32_t)ctx->samples_per_frame;
  if (ctx->capacity == 0) ctx->capacity = 1;
  if (ctx->capacity > NL_ANDROID_AUDIO_MAX_SLOTS) ctx->capacity = NL_ANDROID_AUDIO_MAX_SLOTS;
  if (ctx->target_samples > ctx->capacity * (uint32_t)ctx->samples_per_frame) {
    ctx->target_samples = ctx->capacity * (uint32_t)ctx->samples_per_frame;
  }
  int opus_error;
  ctx->decoder = opus_multistream_decoder_create(config->sampleRate, config->channelCount,
      config->streams, config->coupledStreams, config->mapping, &opus_error);
  if (ctx->decoder == NULL || opus_error != OPUS_OK) {
    fprintf(stderr, "[noland-audio] Opus decoder creation failed: %s\n", opus_strerror(opus_error));
    nl_android_audio_destroy(ctx);
    return -1;
  }
  size_t slot_bytes = (size_t)ctx->samples_per_frame * (size_t)ctx->channels * sizeof(int16_t);
  ctx->decoded = malloc(slot_bytes);
  ctx->buffers = malloc(slot_bytes * ctx->capacity);
  if (ctx->decoded == NULL || ctx->buffers == NULL) {
    nl_android_audio_destroy(ctx);
    return -1;
  }

  SLresult result = slCreateEngine(&ctx->engine_object, 0, NULL, 0, NULL, NULL);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->engine_object)->Realize(ctx->engine_object, SL_BOOLEAN_FALSE);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->engine_object)->GetInterface(ctx->engine_object, SL_IID_ENGINE, &ctx->engine);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->engine)->CreateOutputMix(ctx->engine, &ctx->output_mix, 0, NULL, NULL);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->output_mix)->Realize(ctx->output_mix, SL_BOOLEAN_FALSE);
  if (result != SL_RESULT_SUCCESS) goto fail;

  SLDataLocator_AndroidSimpleBufferQueue locator = {SL_DATALOCATOR_ANDROIDSIMPLEBUFFERQUEUE, ctx->capacity};
  SLDataFormat_PCM pcm = {
    SL_DATAFORMAT_PCM, (SLuint32)ctx->channels, (SLuint32)config->sampleRate * 1000U,
    SL_PCMSAMPLEFORMAT_FIXED_16, SL_PCMSAMPLEFORMAT_FIXED_16,
    nl_android_audio_channel_mask(ctx->channels), SL_BYTEORDER_LITTLEENDIAN,
  };
  SLDataSource source = {&locator, &pcm};
  SLDataLocator_OutputMix mix_locator = {SL_DATALOCATOR_OUTPUTMIX, ctx->output_mix};
  SLDataSink sink = {&mix_locator, NULL};
  const SLInterfaceID ids[] = {SL_IID_ANDROIDSIMPLEBUFFERQUEUE};
  const SLboolean required[] = {SL_BOOLEAN_TRUE};
  result = (*ctx->engine)->CreateAudioPlayer(ctx->engine, &ctx->player, &source, &sink, 1, ids, required);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->player)->Realize(ctx->player, SL_BOOLEAN_FALSE);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->player)->GetInterface(ctx->player, SL_IID_PLAY, &ctx->play);
  if (result != SL_RESULT_SUCCESS) goto fail;
  result = (*ctx->player)->GetInterface(ctx->player, SL_IID_ANDROIDSIMPLEBUFFERQUEUE, &ctx->queue);
  if (result != SL_RESULT_SUCCESS) goto fail;

  result = (*ctx->play)->SetPlayState(ctx->play, SL_PLAYSTATE_STOPPED);
  if (result != SL_RESULT_SUCCESS) goto fail;

  renderer->target_buffer_ms = target_ms;
  renderer->maximum_buffer_ms = maximum_ms;
  renderer->platform_context = ctx;
  return 0;

fail:
  fprintf(stderr, "[noland-audio] OpenSL ES initialization failed: %lu\n", (unsigned long)result);
  nl_android_audio_destroy(ctx);
  return -1;
}

void nl_audio_renderer_start(nl_audio_renderer_t* renderer) {
  nl_android_audio_context_t* ctx = renderer != NULL ? renderer->platform_context : NULL;
  if (ctx == NULL) return;
  pthread_mutex_lock(&ctx->mutex);
  // Playback starts after the target prebuffer contains real decoded samples.
  ctx->running = !ctx->failed;
  pthread_mutex_unlock(&ctx->mutex);
}

void nl_audio_renderer_stop(nl_audio_renderer_t* renderer) {
  nl_android_audio_context_t* ctx = renderer != NULL ? renderer->platform_context : NULL;
  if (ctx == NULL) return;
  pthread_mutex_lock(&ctx->mutex);
  ctx->running = false;
  // Serialized with decode/enqueue. No OpenSL callback needs this mutex.
  SLresult stopped = (*ctx->play)->SetPlayState(ctx->play, SL_PLAYSTATE_STOPPED);
  SLresult cleared = (*ctx->queue)->Clear(ctx->queue);
  ctx->playing = false;
  if (stopped == SL_RESULT_SUCCESS && cleared == SL_RESULT_SUCCESS) {
    ctx->head = ctx->count = ctx->queued_samples = 0;
    opus_multistream_decoder_ctl(ctx->decoder, OPUS_RESET_STATE);
  } else {
    ctx->failed = true;
    fprintf(stderr, "[noland-audio] OpenSL ES stop/clear failed: %lu/%lu\n",
        (unsigned long)stopped, (unsigned long)cleared);
  }
  pthread_mutex_unlock(&ctx->mutex);
}

void nl_audio_renderer_cleanup(nl_audio_renderer_t* renderer) {
  if (renderer == NULL || renderer->platform_context == NULL) return;
  nl_android_audio_context_t* ctx = renderer->platform_context;
  nl_audio_renderer_stop(renderer);
  nl_android_audio_destroy(ctx);
  renderer->platform_context = NULL;
}

void nl_audio_renderer_decode_and_play_sample(nl_audio_renderer_t* renderer,
                                              char* sample_data, int sample_length) {
  nl_android_audio_context_t* ctx = renderer != NULL ? renderer->platform_context : NULL;
  if (ctx == NULL || sample_length < 0 || (sample_length > 0 && sample_data == NULL)) return;
  pthread_mutex_lock(&ctx->mutex);
  if (!ctx->running || ctx->failed) goto done;
  // Decode even when dropping for overflow, preserving Opus prediction state.
  // A zero-length packet uses Opus PLC, not a synthetic silent buffer.
  int samples = opus_multistream_decode(ctx->decoder,
      sample_length > 0 ? (const unsigned char*)sample_data : NULL, sample_length,
      ctx->decoded, ctx->samples_per_frame, 0);
  if (samples <= 0) {
    fprintf(stderr, "[noland-audio] Opus decode failed: %s\n", opus_strerror(samples));
    goto done;
  }
  SLresult result = nl_android_audio_reap(ctx);
  if (result != SL_RESULT_SUCCESS) {
    ctx->failed = true;
    ctx->running = false;
    fprintf(stderr, "[noland-audio] OpenSL ES queue state failed: %lu\n", (unsigned long)result);
    goto done;
  }
  if (ctx->count == ctx->capacity ||
      (uint32_t)samples > ctx->maximum_samples - ctx->queued_samples) goto done;
  uint32_t slot = (ctx->head + ctx->count) % ctx->capacity;
  int16_t* buffer = ctx->buffers + (size_t)slot * (size_t)ctx->samples_per_frame * (size_t)ctx->channels;
  size_t bytes = (size_t)samples * (size_t)ctx->channels * sizeof(int16_t);
  memcpy(buffer, ctx->decoded, bytes);
  result = (*ctx->queue)->Enqueue(ctx->queue, buffer, (SLuint32)bytes);
  if (result != SL_RESULT_SUCCESS) {
    ctx->failed = true;
    ctx->running = false;
    fprintf(stderr, "[noland-audio] OpenSL ES enqueue failed: %lu\n", (unsigned long)result);
    goto done;
  }
  ctx->slot_samples[slot] = (uint32_t)samples;
  ctx->queued_samples += (uint32_t)samples;
  ++ctx->count;
  if (!ctx->playing && (ctx->queued_samples >= ctx->target_samples || ctx->count == ctx->capacity)) {
    result = (*ctx->play)->SetPlayState(ctx->play, SL_PLAYSTATE_PLAYING);
    if (result != SL_RESULT_SUCCESS) {
      ctx->failed = true;
      ctx->running = false;
      fprintf(stderr, "[noland-audio] OpenSL ES start failed: %lu\n", (unsigned long)result);
    } else {
      ctx->playing = true;
    }
  }
done:
  pthread_mutex_unlock(&ctx->mutex);
}

#endif
