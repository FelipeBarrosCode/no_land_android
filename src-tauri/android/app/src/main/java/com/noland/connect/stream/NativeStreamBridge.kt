package com.noland.connect.stream

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.view.Surface
import androidx.annotation.Keep
import java.lang.ref.WeakReference

/** The MainActivity already loads Rust. No codec callbacks or second library load. */
@Keep
internal object NativeStreamBridge {
  private const val TAG = "NolandStream"
  private val main = Handler(Looper.getMainLooper())
  @Volatile private var requestedGeneration = 0L
  @Volatile private var requestedWidth = 1920
  @Volatile private var requestedHeight = 1080
  private var active = WeakReference<StreamActivity>(null)

  external fun nativeClaimActivity(generation: Long): Long
  external fun nativeSurface(generation: Long, activity: Long, serial: Long, surface: Surface, width: Int, height: Int): Boolean
  external fun nativeSurfaceLost(generation: Long, activity: Long, serial: Long)
  external fun nativeFocus(generation: Long, activity: Long, focused: Boolean)
  external fun nativeStop(generation: Long, activity: Long)
  external fun nativeFailure(generation: Long)
  external fun nativeTouch(generation: Long, activity: Long, action: Int, x: Float, y: Float, width: Int, height: Int)
  external fun nativeKey(generation: Long, activity: Long, key: Int, pressed: Boolean, modifiers: Int)
  external fun nativePointer(generation: Long, activity: Long, kind: Int, x: Float, y: Float, width: Int, height: Int)
  external fun nativeButton(generation: Long, activity: Long, button: Int, pressed: Boolean)
  external fun nativeScroll(generation: Long, activity: Long, vertical: Int, horizontal: Int, highResolution: Boolean)
  external fun nativeText(generation: Long, activity: Long, text: String)
  external fun nativeController(generation: Long, activity: Long, number: Int, mask: Int, buttons: Int,
    leftTrigger: Int, rightTrigger: Int, leftX: Int, leftY: Int, rightX: Int, rightY: Int)
  external fun nativeSuspendInput(generation: Long, activity: Long, suspended: Boolean)
  external fun nativeDashboard(generation: Long, activity: Long): Boolean

  @JvmStatic
  fun updateStatistics(generation: Long, text: String) {
    // Never create an Activity or retain text across a presentation/session change.
    val target = active.get()?.takeIf { it.generation == generation } ?: return
    main.post {
      if (active.get() === target && target.generation == generation && requestedGeneration == generation) {
        target.updateStatistics(text)
      }
    }
  }

  /** Must be called on a worker: Rust waits for surfaceChanged after this returns. */
  @JvmStatic
  fun launch(context: Context, generation: Long, width: Int, height: Int): Boolean {
    check(Looper.myLooper() != Looper.getMainLooper()) { "Stream launch/wait must not run on the Android UI thread" }
    requestedWidth = width
    requestedHeight = height
    requestedGeneration = generation
    return main.post {
      if (requestedGeneration != generation) return@post
      try {
        context.startActivity(Intent(context, StreamActivity::class.java).apply {
          addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
          putExtra(StreamActivity.GENERATION, generation)
          putExtra(StreamActivity.VIDEO_WIDTH, width)
          putExtra(StreamActivity.VIDEO_HEIGHT, height)
        })
      } catch (error: Throwable) {
        Log.e(TAG, "Could not launch stream Activity generation=$generation", error)
        invoke("launch failure") { nativeFailure(generation) }
      }
    }
  }

  @JvmStatic
  fun present(context: Context, generation: Long, width: Int, height: Int): Boolean {
    if (requestedGeneration != generation) return false
    return main.post {
      if (requestedGeneration != generation) return@post
      try {
        context.startActivity(Intent(context, StreamActivity::class.java).apply {
          addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
          putExtra(StreamActivity.GENERATION, generation)
          putExtra(StreamActivity.VIDEO_WIDTH, requestedWidth)
          putExtra(StreamActivity.VIDEO_HEIGHT, requestedHeight)
        })
        active.get()?.takeIf { it.generation == generation }?.present()
      } catch (error: Throwable) {
        Log.e(TAG, "Could not present stream Activity generation=$generation", error)
        invoke("presentation failure") { nativeFailure(generation) }
      }
    }
  }

  @JvmStatic
  fun close(context: Context, generation: Long, width: Int, height: Int): Boolean {
    if (requestedGeneration == generation) requestedGeneration = 0L
    return main.post {
      active.get()?.takeIf { it.generation == generation }?.closeFromRust()
    }
  }

  fun register(activity: StreamActivity) {
    active = WeakReference(activity)
  }

  fun unregister(activity: StreamActivity) {
    if (active.get() === activity) active.clear()
  }

  /** Exceptions must retain their Java/native stack in logcat, not become silent no-ops. */
  fun invoke(operation: String, block: () -> Unit): Boolean = try {
    block()
    true
  } catch (error: Throwable) {
    Log.e(TAG, "JNI stream $operation failed", error)
    false
  }

  fun claim(generation: Long): Long = try {
    nativeClaimActivity(generation)
  } catch (error: Throwable) {
    Log.e(TAG, "JNI stream Activity registration failed", error)
    0L
  }

  fun surface(generation: Long, activity: Long, serial: Long, surface: Surface, width: Int, height: Int): Boolean = try {
    nativeSurface(generation, activity, serial, surface, width, height).also { accepted ->
      if (!accepted) Log.w(TAG, "Rejected stream surface generation=$generation activity=$activity serial=$serial")
    }
  } catch (error: Throwable) {
    Log.e(TAG, "JNI stream surface registration failed", error)
    false
  }
}
