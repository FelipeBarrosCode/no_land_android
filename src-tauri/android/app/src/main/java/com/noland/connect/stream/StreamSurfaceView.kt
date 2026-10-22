package com.noland.connect.stream

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import kotlin.math.min
import kotlin.math.roundToInt

internal class StreamSurfaceView @JvmOverloads constructor(
  context: Context,
  attrs: AttributeSet? = null,
) : SurfaceView(context, attrs), SurfaceHolder.Callback {
  interface Listener {
    fun onSurfaceReady(surface: Surface, serial: Long, width: Int, height: Int)
    fun onSurfaceUnavailable(serial: Long)
  }

  private val touchHandler = Handler(Looper.getMainLooper())
  val touchController = StreamTouchController(
    clock = StreamTouchController.Clock { SystemClock.uptimeMillis() },
    scheduler = StreamTouchController.Scheduler { delay, task ->
      val runnable = Runnable { task() }
      touchHandler.postDelayed(runnable, delay)
      StreamTouchController.Cancellation { touchHandler.removeCallbacks(runnable) }
    },
    density = resources.displayMetrics.density,
  )

  var listener: Listener? = null

  /** Legacy raw-touch fallback only. Installing touchSink disables this path, never both. */
  var input: ((Int, Float, Float) -> Unit)? = null
    set(value) {
      cancelTouch()
      field = value
    }

  /** Install a session-bound sink; replacement releases all buttons to the previous sink first. */
  var touchSink: StreamTouchController.Sink?
    get() = touchController.sink
    set(value) {
      if (touchController.sink === value) return
      cancelTouch()
      touchController.sink = value
    }

  var touchMode: TouchMode
    get() = touchController.mode
    set(value) {
      if (touchController.mode == value) return
      cancelTouch()
      touchController.mode = value
    }

  /** Mouse suppression for menu/keyboard/virtual pad; keyboard gestures are gated separately. */
  var inputSuspended: Boolean
    get() = touchController.inputSuspended
    set(value) {
      if (touchController.inputSuspended == value) return
      cancelTouch()
      touchController.inputSuspended = value
    }

  var keyboardGesturesEnabled: Boolean
    get() = touchController.keyboardGesturesEnabled
    set(value) { touchController.keyboardGesturesEnabled = value }

  /**
   * Parent supplies max(44dp, leftSafeInset + 24dp), converted from root to surface-local x.
   * Parent must call cancelSurface() BEFORE claiming drawer/root gestures. Set keyboard
   * gestures false if the parent owns global three-finger recognition (including pad buttons).
   */
  var keyboardLeftEdgePx: Float
    get() = touchController.keyboardLeftEdgePx
    set(value) { touchController.keyboardLeftEdgePx = value }

  var videoWidth = 1920
  var videoHeight = 1080
  private var serial = 0L
  private var available = false
  private var legacyPointer = MotionEvent.INVALID_POINTER_ID

  init {
    holder.addCallback(this)
    holder.setKeepScreenOn(true)
    isFocusable = true
    isFocusableInTouchMode = true
  }

  // The Surface itself is aspect-fit inside a black FrameLayout. Its touch
  // coordinates therefore exclude the letterbox bars, as do the native pixels.
  override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
    val width = MeasureSpec.getSize(widthMeasureSpec)
    val height = MeasureSpec.getSize(heightMeasureSpec)
    val scale = min(width.toFloat() / videoWidth.coerceAtLeast(1), height.toFloat() / videoHeight.coerceAtLeast(1))
    setMeasuredDimension((videoWidth.coerceAtLeast(1) * scale).roundToInt(), (videoHeight.coerceAtLeast(1) * scale).roundToInt())
  }

  override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
    super.onSizeChanged(w, h, oldw, oldh)
    cancelTouch()
    touchController.setViewport(w, h, videoWidth, videoHeight)
  }

  override fun surfaceCreated(holder: SurfaceHolder) {
    serial += 1
    available = true
    requestFocus()
    // surfaceChanged provides authoritative, nonzero physical dimensions.
  }

  override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
    if (available && holder.surface.isValid && width > 0 && height > 0) {
      listener?.onSurfaceReady(holder.surface, serial, width, height)
    }
  }

  fun publishSurface() {
    val frame = holder.surfaceFrame
    if (available && holder.surface.isValid && frame.width() > 0 && frame.height() > 0) {
      listener?.onSurfaceReady(holder.surface, serial, frame.width(), frame.height())
    }
  }

  override fun surfaceDestroyed(holder: SurfaceHolder) = retireSurface()

  fun retireSurface() {
    cancelTouch()
    if (available) {
      available = false
      listener?.onSurfaceUnavailable(serial)
    }
  }

  fun cancelSurface() = cancelTouch()

  fun cancelTouch() {
    touchController.cancel()
    if (legacyPointer != MotionEvent.INVALID_POINTER_ID) input?.invoke(MotionEvent.ACTION_CANCEL, 0f, 0f)
    legacyPointer = MotionEvent.INVALID_POINTER_ID
  }

  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    if (!hasWindowFocus) cancelTouch()
    super.onWindowFocusChanged(hasWindowFocus)
  }

  override fun onDetachedFromWindow() {
    cancelTouch()
    super.onDetachedFromWindow()
  }

  override fun onTouchEvent(event: MotionEvent): Boolean {
    // Hardware mouse/stylus events must not enter the finger recognizer.
    if ((0 until event.pointerCount).any { event.getToolType(it) != MotionEvent.TOOL_TYPE_FINGER }) {
      cancelTouch()
      return super.onTouchEvent(event)
    }
    if (event.actionMasked == MotionEvent.ACTION_DOWN) requestFocus()
    if (touchSink == null) return legacyTouch(event)
    touchController.setViewport(width, height, videoWidth, videoHeight)
    val action = when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> StreamTouchController.Action.DOWN
      MotionEvent.ACTION_POINTER_DOWN -> StreamTouchController.Action.POINTER_DOWN
      MotionEvent.ACTION_MOVE -> StreamTouchController.Action.MOVE
      MotionEvent.ACTION_POINTER_UP -> StreamTouchController.Action.POINTER_UP
      MotionEvent.ACTION_UP -> StreamTouchController.Action.UP
      MotionEvent.ACTION_CANCEL -> StreamTouchController.Action.CANCEL
      else -> return super.onTouchEvent(event)
    }
    // POINTER_UP still contains the lifting pointer. Pass IDs, never a cached index;
    // the next MotionEvent may compact/reorder its indices.
    val pointers = (0 until event.pointerCount).map { index ->
      StreamTouchController.Pointer(event.getPointerId(index), event.getX(index), event.getY(index))
    }
    val changedId = if (action == StreamTouchController.Action.POINTER_DOWN ||
      action == StreamTouchController.Action.POINTER_UP || action == StreamTouchController.Action.UP) {
      event.getPointerId(event.actionIndex)
    } else null
    touchController.onTouch(action, pointers, changedId)
    return true
  }

  private fun legacyTouch(event: MotionEvent): Boolean {
    if (inputSuspended) {
      cancelTouch()
      return true
    }
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> {
        legacyPointer = event.getPointerId(0)
        input?.invoke(MotionEvent.ACTION_DOWN, event.x, event.y)
      }
      MotionEvent.ACTION_POINTER_DOWN -> cancelTouch()
      MotionEvent.ACTION_MOVE -> {
        val index = event.findPointerIndex(legacyPointer)
        if (index >= 0) input?.invoke(MotionEvent.ACTION_MOVE, event.getX(index), event.getY(index))
      }
      MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
        val index = event.actionIndex
        if (event.getPointerId(index) == legacyPointer) {
          input?.invoke(MotionEvent.ACTION_UP, event.getX(index), event.getY(index))
          legacyPointer = MotionEvent.INVALID_POINTER_ID
          performClick()
        }
      }
      MotionEvent.ACTION_CANCEL -> cancelTouch()
    }
    return true
  }

  override fun performClick(): Boolean {
    super.performClick()
    return true
  }
}
