package com.noland.connect.stream

import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.roundToInt

internal enum class TouchMode(val value: Int) {
  TRACKPAD(1), DIRECT(2), CLICK_TO_USE(3),
}

/**
 * Finger-only state machine; call on one thread. Coordinates are surface-local pixels.
 * Snapshots include the lifting pointer on POINTER_UP/UP, exactly like MotionEvent.
 * The bridge owns session routing and translates our IDs, not Android button masks.
 */
internal class StreamTouchController(
  private val clock: Clock,
  private val scheduler: Scheduler,
  val density: Float = 1f,
) {
  fun interface Clock { fun nowMillis(): Long }
  fun interface Cancellation { fun cancel() }
  fun interface Scheduler {
    fun schedule(delayMillis: Long, task: () -> Unit): Cancellation
  }
  interface Sink {
    fun absolute(x: Int, y: Int, width: Int, height: Int)
    fun relative(dx: Int, dy: Int)
    /** 1 = left, 2 = right, 3 = middle, NOT Android/Moonlight constants. */
    fun button(button: Int, pressed: Boolean)
    fun scroll(vertical: Int, horizontal: Int = 0, highResolution: Boolean)
    fun keyboardRequested()
  }
  enum class Action { DOWN, POINTER_DOWN, MOVE, POINTER_UP, UP, CANCEL }
  data class Pointer(val id: Int, val x: Float, val y: Float)

  var sink: Sink? = null
    set(value) {
      if (field === value) return
      cancel() // Release to the old session before changing the owner.
      field = value
    }
  var mode = TouchMode.TRACKPAD
    set(value) {
      if (field == value) return
      cancel()
      field = value
    }
  /** Mouse only: empty virtual-gamepad surface can still request a keyboard. */
  var inputSuspended = false
    set(value) {
      if (field == value) return
      cancel()
      field = value
    }
  /** False for menu/keyboard or when the parent owns global three-finger recognition. */
  var keyboardGesturesEnabled = true
    set(value) {
      if (field == value) return
      cancel()
      field = value
    }
  /** Surface-local x cutoff for the drawer strip; equality belongs to drawer. */
  var keyboardLeftEdgePx = 44f * density
    set(value) {
      require(value.isFinite() && value >= 0f)
      if (field == value) return
      cancel()
      field = value
    }

  private var width = 0
  private var height = 0
  private var videoWidth = 0
  private var videoHeight = 0
  private val pointers = linkedMapOf<Int, Pointer>()
  private val starts = mutableMapOf<Int, Pointer>()
  private val pressed = linkedSetOf<Int>()
  private var peak = 0
  private var origin = Pointer(0, 0f, 0f)
  private var location = origin
  private var moved = false
  private var individualMoved = false
  private var suppressed = true
  private var outsideEdge = true
  private var startedAt = 0L
  private var held = false
  private var holdTask: Cancellation? = null
  private var clickTask: Cancellation? = null
  private var holdToken = 0L
  private var clickToken = 0L
  private var pendingButton: Int? = null
  private val threshold = 5f * density

  init { require(density.isFinite() && density > 0f) }

  fun setViewport(width: Int, height: Int, videoWidth: Int = width, videoHeight: Int = height) {
    if (this.width == width && this.height == height &&
      this.videoWidth == videoWidth && this.videoHeight == videoHeight) return
    cancel()
    this.width = width.coerceAtLeast(0)
    this.height = height.coerceAtLeast(0)
    this.videoWidth = videoWidth.coerceAtLeast(0)
    this.videoHeight = videoHeight.coerceAtLeast(0)
  }

  /** Never clicks. Ignore the remaining physical sequence until a fresh DOWN. */
  fun cancel() {
    stopHold()
    finishPendingClick()
    releaseButtons()
    pointers.clear()
    starts.clear()
    peak = 0
    held = false
    moved = true
    suppressed = true
  }
  fun release() = cancel()

  fun onTouch(action: Action, snapshot: List<Pointer>, changedPointerId: Int? = null) {
    if (action == Action.CANCEL) {
      cancel()
      return
    }
    if (snapshot.any { !it.x.isFinite() || !it.y.isFinite() } ||
      snapshot.map { it.id }.toSet().size != snapshot.size) {
      cancel()
      return
    }
    if (action == Action.DOWN) {
      cancel() // Drain synthetic click before the next physical sequence.
      if (snapshot.size != 1) return
      val first = snapshot.single()
      pointers[first.id] = first
      starts[first.id] = first
      peak = 1
      origin = first
      location = first
      moved = false
      individualMoved = false
      outsideEdge = first.x > keyboardLeftEdgePx
      startedAt = clock.nowMillis()
      suppressed = inputSuspended || width <= 0 || height <= 0
      if (!suppressed) {
        if (mode != TouchMode.TRACKPAD) absolute(first)
        if (mode == TouchMode.DIRECT) press(LEFT)
        beginHold()
      }
      return
    }
    if (pointers.isEmpty()) return
    val ids = snapshot.map { it.id }.toSet()
    val newPointer = if (action == Action.POINTER_DOWN) snapshot.find { it.id == changedPointerId } else null
    val expected = pointers.keys.toMutableSet().apply { if (newPointer != null) add(newPointer.id) }
    if (ids != expected || (action == Action.POINTER_DOWN &&
        (newPointer == null || pointers.containsKey(newPointer.id))) ||
      ((action == Action.UP || action == Action.POINTER_UP) && changedPointerId !in pointers) ||
      (action == Action.UP && pointers.size != 1) ||
      (action == Action.POINTER_UP && pointers.size < 2)) {
      cancel()
      return
    }
    for (point in snapshot) {
      pointers[point.id] = point
      val start = starts[point.id]
      if (start != null && distance(point, start) >= threshold) individualMoved = true
    }
    when (action) {
      Action.POINTER_DOWN -> {
        val point = newPointer ?: return
        starts[point.id] = point
        outsideEdge = outsideEdge && point.x > keyboardLeftEdgePx
        peak = maxOf(peak, pointers.size)
        stopHold()
        releaseButtons()
        held = false
        if (peak >= 3) {
          finishPendingClick()
          suppressed = true
        } else if (pointers.size == 2) {
          origin = centroid()
          location = origin
        }
      }
      Action.MOVE -> move()
      Action.POINTER_UP, Action.UP -> {
        // UP can cross the tap threshold without a preceding MOVE.
        if (peak < 3 && (pointers.size == 2 || peak == 1)) {
          val point = if (pointers.size == 2) centroid() else pointers.values.first()
          if (distance(point, origin) >= threshold) moved = true
        }
        stopHold()
        val lastPoint = pointers[changedPointerId] ?: return
        pointers.remove(changedPointerId)
        if (pointers.isNotEmpty()) return // Never resume one finger after multitouch.
        val keyboard = peak == 3 && outsideEdge && !individualMoved &&
          clock.nowMillis() - startedAt in 0 until HOLD_MILLIS && keyboardGesturesEnabled
        if (peak >= 3) {
          cancel()
          if (keyboard) sink?.keyboardRequested()
        } else {
          if (!suppressed) {
            when {
              mode == TouchMode.DIRECT || held -> releaseButtons()
              !moved -> {
                if (mode == TouchMode.CLICK_TO_USE && peak == 1) absolute(lastPoint)
                click(if (peak == 2) RIGHT else LEFT)
              }
            }
          }
          pointers.clear()
          starts.clear()
          peak = 0
          suppressed = true
          held = false
        }
      }
      else -> Unit
    }
  }

  private fun move() {
    if (suppressed || inputSuspended || (peak > 1 && pointers.size < 2)) return
    val point = if (pointers.size == 2) centroid() else pointers.values.first()
    if (pointers.size == 2) {
      sink?.scroll(((point.y - location.y) * 10f).roundToInt(), highResolution = true)
    } else if (mode == TouchMode.TRACKPAD) {
      val dx = ((point.x - location.x) * 1280f / width).roundToInt()
      val dy = ((point.y - location.y) * 720f / height).roundToInt()
      if (dx != 0 || dy != 0) sink?.relative(dx, dy)
    } else {
      absolute(point)
    }
    location = point
    if (distance(point, origin) >= threshold) {
      moved = true
      stopHold()
    }
  }

  private fun centroid(): Pointer {
    val points = pointers.values.toList()
    return Pointer(0, (points[0].x + points[1].x) / 2f, (points[0].y + points[1].y) / 2f)
  }
  private fun distance(a: Pointer, b: Pointer): Float = hypot(a.x - b.x, a.y - b.y)

  private fun absolute(point: Pointer) {
    if (width <= 0 || height <= 0) return
    val aspect = if (videoWidth > 0 && videoHeight > 0) videoWidth.toFloat() / videoHeight else width.toFloat() / height
    val fittedWidth = min(width.toFloat(), height * aspect)
    val fittedHeight = fittedWidth / aspect
    val w = fittedWidth.roundToInt().coerceIn(1, Short.MAX_VALUE.toInt())
    val h = fittedHeight.roundToInt().coerceIn(1, Short.MAX_VALUE.toInt())
    sink?.absolute(
      (point.x - (width - fittedWidth) / 2f).roundToInt().coerceIn(0, w),
      (point.y - (height - fittedHeight) / 2f).roundToInt().coerceIn(0, h), w, h,
    )
  }

  private fun beginHold() {
    stopHold()
    val token = holdToken
    val deadline = startedAt + HOLD_MILLIS
    fun scheduleHold() {
      holdTask = scheduler.schedule((deadline - clock.nowMillis()).coerceAtLeast(0)) task@{
        if (token != holdToken || suppressed || inputSuspended || moved || peak != 1 || pointers.size != 1) return@task
        if (clock.nowMillis() < deadline) {
          scheduleHold()
        } else {
          holdTask = null
          held = true
          if (mode == TouchMode.DIRECT) {
            releaseButton(LEFT)
            press(RIGHT)
          } else {
            press(LEFT)
          }
        }
      }
    }
    scheduleHold()
  }

  private fun stopHold() {
    holdToken += 1
    holdTask?.cancel()
    holdTask = null
  }

  private fun click(button: Int) {
    finishPendingClick()
    pendingButton = button
    press(button)
    val token = clickToken
    clickTask = scheduler.schedule(TAP_RELEASE_MILLIS) {
      if (token == clickToken) finishPendingClick()
    }
  }

  private fun finishPendingClick() {
    clickToken += 1
    clickTask?.cancel()
    clickTask = null
    pendingButton?.let { releaseButton(it) }
    pendingButton = null
  }
  private fun press(button: Int) {
    if (pressed.add(button)) sink?.button(button, true)
  }
  private fun releaseButton(button: Int) {
    if (pressed.remove(button)) sink?.button(button, false)
  }
  private fun releaseButtons() {
    for (button in pressed.toList()) releaseButton(button)
  }

  companion object {
    const val LEFT = 1
    const val RIGHT = 2
    const val MIDDLE = 3
    const val HOLD_MILLIS = 650L
    const val TAP_RELEASE_MILLIS = 50L
  }
}
