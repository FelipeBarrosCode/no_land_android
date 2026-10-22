package com.noland.connect.stream

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.widget.FrameLayout
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Native, transparent gamepad. Each widget owns one pointer ID until that pointer ends;
 * moving a captured stick outside its bounds never transfers it to another widget.
 * Empty DOWNs return false. The Activity must suppress surface mouse input while enabled,
 * including sequences starting in empty space (StreamControlsOverlay handles those).
 * All methods and callbacks belong on the UI thread. No bridge or Activity dependency.
 */
class VirtualGamepadView @JvmOverloads constructor(
  context: Context,
  attrs: AttributeSet? = null,
) : FrameLayout(context, attrs) {
  var onStateChanged: ((GamepadState) -> Unit)? = null
  var state: GamepadState = GamepadState.ZERO
    private set

  private enum class Kind { LEFT_STICK, RIGHT_STICK, DPAD, BUTTON, LEFT_TRIGGER, RIGHT_TRIGGER }

  private val density = resources.displayMetrics.density
  private var safeLeft = 0
  private var safeTop = 0
  private var safeRight = 0
  private var safeBottom = 0
  private var tracking = false
  private var ready = false
  private val leftStick = Control("Left stick", Kind.LEFT_STICK)
  private val rightStick = Control("Right stick", Kind.RIGHT_STICK)
  private val dpad = Control("Directional pad", Kind.DPAD)
  private val buttons = listOf(
    Control("A", Kind.BUTTON, GamepadState.A), Control("B", Kind.BUTTON, GamepadState.B),
    Control("X", Kind.BUTTON, GamepadState.X), Control("Y", Kind.BUTTON, GamepadState.Y),
    Control("LB", Kind.BUTTON, GamepadState.LB), Control("RB", Kind.BUTTON, GamepadState.RB),
    Control("LT", Kind.LEFT_TRIGGER), Control("RT", Kind.RIGHT_TRIGGER),
    Control("L3", Kind.BUTTON, GamepadState.L3), Control("R3", Kind.BUTTON, GamepadState.R3),
    Control("Select", Kind.BUTTON, GamepadState.SELECT), Control("Start", Kind.BUTTON, GamepadState.START),
  )
  private val controls = listOf(leftStick, rightStick, dpad) + buttons

  init {
    setBackgroundColor(Color.TRANSPARENT)
    isMotionEventSplittingEnabled = false // This group explicitly routes by stable pointer ID.
    clipChildren = true
    controls.forEach { addView(it) }
    ready = true
  }

  fun setSafeInsets(left: Int, top: Int, right: Int, bottom: Int) {
    val l = max(0, left); val t = max(0, top)
    val r = max(0, right); val b = max(0, bottom)
    if (safeLeft == l && safeTop == t && safeRight == r && safeBottom == b) return
    releaseControls()
    safeLeft = l; safeTop = t; safeRight = r; safeBottom = b
    requestLayout()
  }

  /** Coordinates are local to this view; circular widgets retain the reference's square hit area. */
  fun isControlTouch(x: Float, y: Float): Boolean =
    isShown && isEnabled && alpha > 0.01f && controlAt(x, y) != null

  private fun controlAt(x: Float, y: Float): Control? = controls.asReversed().firstOrNull {
    x >= it.left && x < it.right && y >= it.top && y < it.bottom
  }

  override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
    setMeasuredDimension(MeasureSpec.getSize(widthMeasureSpec), MeasureSpec.getSize(heightMeasureSpec))
    // Exact widget sizes are established by onLayout, not FrameLayout's MATCH_PARENT default.
    controls.forEach { it.measure(MeasureSpec.makeMeasureSpec(0, MeasureSpec.EXACTLY),
      MeasureSpec.makeMeasureSpec(0, MeasureSpec.EXACTLY)) }
  }

  override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
    if (changed) releaseControls()
    val availableWidth = max(1, width - safeLeft - safeRight).toFloat()
    val availableHeight = max(1, height - safeTop - safeBottom).toFloat()
    // Keep the iOS geometry on handheld landscape displays. Scale uniformly on small
    // windows rather than overlapping shoulders/Select/Start or clipping the bottom row.
    val scale = min(1f, min(availableWidth / dp(400f), availableHeight / dp(340f)))
    val unit = density * scale
    val w = availableWidth / unit
    val h = availableHeight / unit
    val l = 12f; val r = w - 12f; val b = h - 12f
    val stickSize = min(104f, max(78f, max(250f, b) * 0.27f))
    val dpadSize = min(88f, stickSize)
    fun place(control: Control, x: Float, y: Float, width: Float, height: Float) {
      val x1 = safeLeft + (x * unit).roundToInt()
      val y1 = safeTop + (y * unit).roundToInt()
      val x2 = safeLeft + ((x + width) * unit).roundToInt()
      val y2 = safeTop + ((y + height) * unit).roundToInt()
      control.measure(MeasureSpec.makeMeasureSpec(max(1, x2 - x1), MeasureSpec.EXACTLY),
        MeasureSpec.makeMeasureSpec(max(1, y2 - y1), MeasureSpec.EXACTLY))
      control.layout(x1, y1, x2, y2)
    }
    place(leftStick, l + 12f, 70f, stickSize, stickSize)
    place(dpad, l + 20f, b - dpadSize, dpadSize, dpadSize)
    place(rightStick, r - stickSize - 20f, b - stickSize, stickSize, stickSize)
    val cx = r - 70f
    val offsets = arrayOf(0f to 46f, 46f to 0f, -46f to 0f, 0f to -46f)
    offsets.forEachIndexed { index, (x, y) -> place(buttons[index], cx + x - 22f, 132f + y - 22f, 44f, 44f) }
    place(buttons[4], l, 8f, 54f, 44f)
    place(buttons[6], l + 60f, 8f, 54f, 44f)
    place(buttons[5], r - 54f, 8f, 54f, 44f)
    place(buttons[7], r - 114f, 8f, 54f, 44f)
    place(buttons[8], l + 12f + stickSize + 8f, 70f + stickSize / 2f - 22f, 44f, 44f)
    place(buttons[9], r - stickSize - 20f - 52f, b - stickSize / 2f - 22f, 44f, 44f)
    val middle = (l + r) / 2f
    place(buttons[10], middle - 66f, 12f, 60f, 44f)
    place(buttons[11], middle + 6f, 12f, 60f, 44f)
  }

  override fun onInterceptTouchEvent(event: MotionEvent): Boolean = true

  override fun onTouchEvent(event: MotionEvent): Boolean = track(event, allowEmptyDown = false)

  /** Used only for an Activity-owned sequence that began in empty gamepad space. */
  internal fun onRootTouchEvent(event: MotionEvent): Boolean = track(event, allowEmptyDown = true)

  private fun track(event: MotionEvent, allowEmptyDown: Boolean): Boolean {
    if (!isShown || !isEnabled) {
      releaseControls()
      return false
    }
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> {
        releaseControlsIfNeeded()
        val control = controlAt(event.x, event.y)
        if (control == null && !allowEmptyDown) return false
        tracking = true
        parent?.requestDisallowInterceptTouchEvent(true)
        capture(event, 0)
      }
      MotionEvent.ACTION_POINTER_DOWN -> if (tracking) capture(event, event.actionIndex)
      MotionEvent.ACTION_MOVE -> if (tracking) {
        controls.filter { it.pointerId != MotionEvent.INVALID_POINTER_ID }.forEach { control ->
          val index = event.findPointerIndex(control.pointerId)
          if (index < 0) control.reset() else control.update(event.getX(index), event.getY(index))
        }
      }
      MotionEvent.ACTION_POINTER_UP -> if (tracking) {
        val id = event.getPointerId(event.actionIndex)
        controls.firstOrNull { it.pointerId == id }?.reset()
      }
      MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
        val consumed = tracking
        releaseControls()
        return consumed
      }
      else -> return tracking
    }
    publish()
    return tracking
  }

  private fun capture(event: MotionEvent, index: Int) {
    val control = controlAt(event.getX(index), event.getY(index)) ?: return
    if (control.pointerId != MotionEvent.INVALID_POINTER_ID) return
    control.pointerId = event.getPointerId(index)
    control.update(event.getX(index), event.getY(index))
  }

  private fun publish(force: Boolean = false) {
    var mask = 0
    var lt = 0
    var rt = 0
    controls.forEach { control ->
      if (control.pointerId != MotionEvent.INVALID_POINTER_ID) {
        when (control.kind) {
          Kind.DPAD -> mask = mask or control.position.directions
          Kind.BUTTON -> if (control.pressedInside) mask = mask or control.flag
          Kind.LEFT_TRIGGER -> if (control.pressedInside) lt = 255
          Kind.RIGHT_TRIGGER -> if (control.pressedInside) rt = 255
          else -> Unit
        }
      }
    }
    val next = GamepadState(mask, lt, rt,
      leftStick.position.axisX, leftStick.position.axisY,
      rightStick.position.axisX, rightStick.position.axisY)
    val changed = next != state
    state = next
    if (changed || force) onStateChanged?.invoke(next)
  }

  /** Clears pointer ownership as well as output; subsequent MOVE events cannot re-press a key. */
  fun releaseControls() {
    tracking = false
    controls.forEach { it.reset() }
    parent?.requestDisallowInterceptTouchEvent(false)
    publish(force = true)
  }

  fun reset() = releaseControls()

  private fun releaseControlsIfNeeded() {
    if (tracking || state != GamepadState.ZERO) releaseControls()
  }

  override fun onVisibilityChanged(changedView: View, visibility: Int) {
    super.onVisibilityChanged(changedView, visibility)
    if (ready && visibility != View.VISIBLE) releaseControls()
  }

  override fun onWindowVisibilityChanged(visibility: Int) {
    super.onWindowVisibilityChanged(visibility)
    if (ready && visibility != View.VISIBLE) releaseControls()
  }

  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    if (ready && !hasWindowFocus) releaseControls()
  }

  override fun setEnabled(enabled: Boolean) {
    super.setEnabled(enabled)
    if (ready && !enabled) releaseControls()
  }

  override fun onDetachedFromWindow() {
    releaseControls()
    super.onDetachedFromWindow()
  }

  private fun dp(value: Float): Float = value * density

  private inner class Control(val label: String, val kind: Kind, val flag: Int = 0) : View(context) {
    var pointerId = MotionEvent.INVALID_POINTER_ID
    var pressedInside = false
    var position = StickPosition(0f, 0f, 0f, 0f)
      private set
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val rect = RectF()
    private val analog get() = kind == Kind.LEFT_STICK || kind == Kind.RIGHT_STICK || kind == Kind.DPAD

    init {

      contentDescription = label
      tag = "stream.pad.$label"
      importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
      isClickable = !analog
      isFocusable = true
    }

    fun update(x: Float, y: Float) {
      pressedInside = x >= left && x < right && y >= top && y < bottom
      if (analog) position = StickPosition.at(x - left, y - top, width.toFloat())
      isPressed = analog || pressedInside
      invalidate()
    }

    fun reset() {
      pointerId = MotionEvent.INVALID_POINTER_ID
      pressedInside = false
      position = StickPosition(width / 2f, height / 2f, 0f, 0f)
      isPressed = false
      invalidate()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
      super.onSizeChanged(w, h, oldw, oldh)
      reset()
    }

    override fun performClick(): Boolean {
      super.performClick()
      if (!analog && this@VirtualGamepadView.isShown && this@VirtualGamepadView.isEnabled) {
        // Accessibility activation is an atomic pulse, independent of any held fingers.
        val original = state
        val pulse = when (kind) {
          Kind.LEFT_TRIGGER -> original.copy(leftTrigger = 255)
          Kind.RIGHT_TRIGGER -> original.copy(rightTrigger = 255)
          else -> original.copy(buttons = original.buttons or flag)
        }
        onStateChanged?.invoke(pulse)
        onStateChanged?.invoke(state)
      }
      return true
    }

    override fun onDraw(canvas: Canvas) {
      super.onDraw(canvas)
      val unit = min(width / (if (analog) 88f else if (label == "Select" || label == "Start") 60f else 44f), height / 44f)
      paint.style = Paint.Style.FILL
      if (analog) {
        val radius = min(width, height) / 2f
        paint.color = Color.argb(77, 26, 26, 26)
        canvas.drawCircle(width / 2f, height / 2f, radius - 1f, paint)
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = max(1f, 1.5f * unit)
        paint.color = Color.argb(153, 255, 255, 255)
        canvas.drawCircle(width / 2f, height / 2f, radius - paint.strokeWidth / 2f, paint)
        if (kind == Kind.DPAD) {
          canvas.drawLine(width * 0.25f, height / 2f, width * 0.75f, height / 2f, paint)
          canvas.drawLine(width / 2f, height * 0.25f, width / 2f, height * 0.75f, paint)
        }
        paint.style = Paint.Style.FILL
        paint.color = Color.argb(if (isPressed) 153 else 102, 255, 255, 255)
        canvas.drawCircle(position.knobX, position.knobY, width * 0.2f, paint)
      } else {
        paint.color = Color.argb(if (isPressed) 230 else 150, 38, 38, 38)
        rect.set(0f, 0f, width.toFloat(), height.toFloat())
        canvas.drawRoundRect(rect, 10f * unit, 10f * unit, paint)
        paint.color = Color.argb(if (isPressed) 255 else 190, 255, 255, 255)
        paint.textAlign = Paint.Align.CENTER
        paint.textSize = 14f * unit
        paint.isFakeBoldText = true
        canvas.drawText(label, width / 2f, height / 2f - (paint.ascent() + paint.descent()) / 2f, paint)
      }
    }
  }
}

/** Immutable Moonlight-compatible snapshot. Signed axes; digital triggers are 0/255. */
data class GamepadState(
  val buttons: Int = 0,
  val leftTrigger: Int = 0,
  val rightTrigger: Int = 0,
  val leftX: Int = 0,
  val leftY: Int = 0,
  val rightX: Int = 0,
  val rightY: Int = 0,
) {
  companion object {
    const val AXIS_MAX = 32766
    const val UP = 0x0001
    const val DOWN = 0x0002
    const val LEFT = 0x0004
    const val RIGHT = 0x0008
    const val START = 0x0010
    const val SELECT = 0x0020
    const val L3 = 0x0040
    const val R3 = 0x0080
    const val LB = 0x0100
    const val RB = 0x0200
    const val A = 0x1000
    const val B = 0x2000
    const val X = 0x4000
    const val Y = 0x8000
    val ZERO = GamepadState()
  }
}

/** Pure stick math: independent square clamping, positive Y up, and no client deadzone. */
internal data class StickPosition(val knobX: Float, val knobY: Float, val x: Float, val y: Float) {
  val axisX: Int get() = (x * GamepadState.AXIS_MAX).roundToInt()
  val axisY: Int get() = (y * GamepadState.AXIS_MAX).roundToInt()
  val directions: Int get() =
    (if (y > 0.35f) GamepadState.UP else 0) or
      (if (y < -0.35f) GamepadState.DOWN else 0) or
      (if (x > 0.35f) GamepadState.RIGHT else 0) or
      (if (x < -0.35f) GamepadState.LEFT else 0)

  companion object {
    fun at(x: Float, y: Float, size: Float): StickPosition {
      val center = size / 2f
      val visual = max(1f, size * 0.30f)
      val response = max(1f, size * 0.18f)
      val dx = (x - center).coerceIn(-visual, visual)
      val dy = (y - center).coerceIn(-visual, visual)
      return StickPosition(center + dx, center + dy,
        (dx / response).coerceIn(-1f, 1f), (-dy / response).coerceIn(-1f, 1f))
    }
  }
}
