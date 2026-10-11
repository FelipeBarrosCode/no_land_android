package com.noland.connect.stream

import android.view.Surface

/** Narrow JNI boundary between the Android stream route and the Rust session owner. */
internal object NativeStreamBridge {
  private external fun nativeAttachSurface(surface: Surface, width: Int, height: Int): Int
  private external fun nativeResizeSurface(width: Int, height: Int): Int
  private external fun nativeDetachSurface()
  private external fun nativeStopStream()

  fun attachSurface(surface: Surface, width: Int, height: Int): Boolean =
    runCatching { nativeAttachSurface(surface, width, height) == 0 }.getOrDefault(false)

  fun resizeSurface(width: Int, height: Int): Boolean =
    runCatching { nativeResizeSurface(width, height) == 0 }.getOrDefault(false)

  fun detachSurface() {
    runCatching { nativeDetachSurface() }
  }

  fun stopStream() {
    runCatching { nativeStopStream() }
  }
}
