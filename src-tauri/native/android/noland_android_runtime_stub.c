#include "../noland-moonlight/include/noland_moonlight.h"

#include <string.h>

struct nl_runtime {
  nl_stream_state_t state;
};

static struct nl_runtime android_runtime = {NL_STREAM_STATE_IDLE};

nl_result_t nl_runtime_create(nl_runtime_t** output) {
  if (output == NULL) {
    return NL_RESULT_INVALID_ARGUMENT;
  }
  android_runtime.state = NL_STREAM_STATE_IDLE;
  *output = &android_runtime;
  return NL_RESULT_OK;
}

void nl_runtime_destroy(nl_runtime_t* runtime) {
  (void)runtime;
}

const char* nl_runtime_version_string(void) {
  return "android-shell";
}

const char* nl_get_launch_query_parameters(void) {
  return "";
}

int32_t nl_runtime_smoke_test(void) {
  return 0;
}

size_t nl_sizeof_start_request(void) { return sizeof(nl_start_request_t); }
size_t nl_sizeof_event(void) { return sizeof(nl_event_t); }
size_t nl_sizeof_stats(void) { return sizeof(nl_stats_t); }
size_t nl_sizeof_performance_stats(void) { return sizeof(nl_performance_stats_t); }

nl_result_t nl_runtime_start(nl_runtime_t* runtime, const nl_start_request_t* request) {
  (void)runtime;
  (void)request;
  return NL_RESULT_INVALID_STATE;
}

nl_result_t nl_runtime_request_stop(nl_runtime_t* runtime) {
  if (runtime == NULL) {
    return NL_RESULT_INVALID_ARGUMENT;
  }
  runtime->state = NL_STREAM_STATE_IDLE;
  return NL_RESULT_OK;
}

nl_result_t nl_runtime_attach_surface(
    nl_runtime_t* runtime,
    const nl_surface_descriptor_t* surface) {
  (void)runtime;
  (void)surface;
  return NL_RESULT_INVALID_STATE;
}

nl_result_t nl_runtime_detach_surface(nl_runtime_t* runtime) {
  return runtime == NULL ? NL_RESULT_INVALID_ARGUMENT : NL_RESULT_OK;
}

nl_result_t nl_runtime_poll_event(nl_runtime_t* runtime, nl_event_t* output) {
  if (runtime == NULL || output == NULL) {
    return NL_RESULT_INVALID_ARGUMENT;
  }
  return NL_RESULT_QUEUE_EMPTY;
}

nl_result_t nl_runtime_read_stats(nl_runtime_t* runtime, nl_stats_t* output) {
  if (runtime == NULL || output == NULL) {
    return NL_RESULT_INVALID_ARGUMENT;
  }
  memset(output, 0, sizeof(*output));
  output->state = runtime->state;
  output->pending_core_video_frames = -1;
  return NL_RESULT_OK;
}

void nl_runtime_read_performance(
    nl_runtime_t* runtime,
    nl_performance_stats_t* output) {
  (void)runtime;
  if (output == NULL) {
    return;
  }
  memset(output, 0, sizeof(*output));
  output->incoming_fps = -1.0;
  output->decoded_fps = -1.0;
  output->submitted_fps = -1.0;
  output->host_processing_ms = -1.0;
  output->reassembly_ms = -1.0;
  output->decode_ms = -1.0;
  output->render_queue_ms = -1.0;
}

void nl_runtime_set_overlay_text(nl_runtime_t* runtime, const char* text) {
  (void)runtime;
  (void)text;
}

void nl_runtime_record_reconnect_result(
    nl_runtime_t* runtime,
    bool attempt_started,
    bool succeeded) {
  (void)runtime;
  (void)attempt_started;
  (void)succeeded;
}

#define ANDROID_INPUT_STUB(name, signature) \
  nl_result_t name signature { return NL_RESULT_INVALID_STATE; }

ANDROID_INPUT_STUB(nl_send_relative_mouse,
                   (nl_runtime_t* runtime, int16_t delta_x, int16_t delta_y))
ANDROID_INPUT_STUB(nl_send_absolute_mouse,
                   (nl_runtime_t* runtime, int16_t x, int16_t y,
                    int16_t reference_width, int16_t reference_height))
ANDROID_INPUT_STUB(nl_send_mouse_button,
                   (nl_runtime_t* runtime, uint8_t button, bool pressed))
ANDROID_INPUT_STUB(nl_send_vertical_scroll,
                   (nl_runtime_t* runtime, int16_t amount, bool high_resolution))
ANDROID_INPUT_STUB(nl_send_horizontal_scroll,
                   (nl_runtime_t* runtime, int16_t amount, bool high_resolution))
ANDROID_INPUT_STUB(nl_send_keyboard,
                   (nl_runtime_t* runtime, uint16_t virtual_key, bool pressed,
                    uint8_t modifiers))
ANDROID_INPUT_STUB(nl_send_controller_arrival,
                   (nl_runtime_t* runtime, uint8_t controller_number,
                    uint16_t active_gamepad_mask, uint8_t controller_type,
                    uint32_t supported_button_flags, uint16_t capabilities))
ANDROID_INPUT_STUB(nl_send_controller_state,
                   (nl_runtime_t* runtime, int16_t controller_number,
                    int16_t active_gamepad_mask, int32_t button_flags,
                    uint8_t left_trigger, uint8_t right_trigger,
                    int16_t left_stick_x, int16_t left_stick_y,
                    int16_t right_stick_x, int16_t right_stick_y))
ANDROID_INPUT_STUB(nl_send_utf8_text,
                   (nl_runtime_t* runtime, const char* text, uint32_t length))
ANDROID_INPUT_STUB(nl_send_touch,
                   (nl_runtime_t* runtime, uint8_t event_type, uint32_t pointer_id,
                    float x, float y, float pressure, uint16_t rotation))
ANDROID_INPUT_STUB(nl_send_controller,
                   (nl_runtime_t* runtime, uint16_t controller_number,
                    uint16_t active_gamepad_mask, uint32_t button_flags,
                    uint8_t left_trigger, uint8_t right_trigger,
                    int16_t left_stick_x, int16_t left_stick_y,
                    int16_t right_stick_x, int16_t right_stick_y))
ANDROID_INPUT_STUB(nl_release_all_input, (nl_runtime_t* runtime))
