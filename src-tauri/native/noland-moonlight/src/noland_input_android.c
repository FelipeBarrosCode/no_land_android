#if defined(__ANDROID__)

#include "noland_moonlight.h"

// Android's Rust/Kotlin bridge sends input through nl_send_* directly. These
// desktop lifecycle hooks deliberately install no SDL/JNI handlers or capture.
int nl_desktop_input_install(const nl_surface_descriptor_t* surface) {
  return surface != NULL && surface->surface_type == NL_SURFACE_ANDROID_NATIVE_WINDOW &&
      surface->window_handle != NULL ? 0 : -1;
}

void nl_desktop_input_uninstall(void) {
}

int nl_desktop_input_set_capture_active(bool active, int mode) {
  (void)active;
  (void)mode;
  return 0;
}

#endif
