package com.noland.connect.stream

import android.animation.Animator
import android.animation.AnimatorListenerAdapter
import android.animation.ValueAnimator
import android.content.Context
import android.content.SharedPreferences
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.VelocityTracker
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.WindowInsets
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Add MATCH_PARENT as the last child of the Activity's video FrameLayout. Do not put
 * the SurfaceView above normal window content with setZOrderOnTop(true).
 *
 * Activity dispatch contract (UI thread):
 * 1. Call onRootTouchEvent with EVERY unmodified Activity MotionEvent, before stream
 *    gestures or super.dispatchTouchEvent. A true result owns that whole sequence.
 * 2. Otherwise use normal super.dispatchTouchEvent so native menu/gamepad views work.
 * 3. Gate ONLY the surface's mouse/gesture sink on isSurfaceMouseSuspended, not normal
 *    View dispatch. Cancel the surface's existing mouse gesture whenever this becomes
 *    true (also onMenuChanged, onGamepadChanged, and before showing a keyboard).
 *    Gamepad-enabled empty areas must never become mouse gestures.
 * 4. Parent keyboard visibility detection calls setKeyboardVisible; pause/focus loss,
 *    stream rebinding, and teardown call reset/releaseControls before retiring input.
 *
 * Root events use Activity coordinates; isControlTouch uses overlay-local coordinates.
 * An edge DOWN is reserved immediately: a rejected/vertical/multifinger edge gesture
 * is drained, not replayed as a delayed mouse click. UI/control-started sequences are
 * never recruited into the edge recognizer. Empty-space gamepad sequences are routed
 * here so another finger can subsequently acquire a control without a mouse DOWN.
 *
 * SharedPreferences are application-wide, never per Activity/stream. Callbacks run on
 * the UI thread; publishState is also called on attachment, after preferences reload.
 */
class StreamControlsOverlay @JvmOverloads constructor(
  context: Context,
  attrs: AttributeSet? = null,
) : FrameLayout(context, attrs) {
  var onMenuChanged: ((Boolean) -> Unit)? = null
  var onModeChanged: ((Int) -> Unit)? = null
  var onGamepadChanged: ((Boolean, GamepadState) -> Unit)? = null
  var onKeyboardRequested: (() -> Unit)? = null
  var onDashboardRequested: (() -> Unit)? = null

  private val preferences = context.applicationContext.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE)
  var touchMode: Int = readMode()
    private set
  var gamepadEnabled: Boolean = readGamepad()
    private set
  var menuVisible: Boolean = false
    private set
  private var keyboardShown = false
  val keyboardVisible: Boolean get() = keyboardShown
  val gamepadState: GamepadState get() = gamepad.state
  var drawerProgress: Float = 0f
    private set

  /** Keep mouse input suspended even when the enabled pad is hidden or a touch is outside a widget. */
  val isSurfaceMouseSuspended: Boolean get() = gamepadEnabled || menuVisible || keyboardVisible ||
    rootRoute == RootRoute.EDGE || rootRoute == RootRoute.BLOCKED || rootRoute == RootRoute.CONTROLS

  private val density = resources.displayMetrics.density
  private val touchSlop = ViewConfiguration.get(context).scaledTouchSlop
  private var safeLeft = 0
  private var safeTop = 0
  private var safeRight = 0
  private var safeBottom = 0
  private var ready = false
  private val gamepad = VirtualGamepadView(context)
  private val scrim = View(context)
  private val drawer = ScrollView(context)
  private val menuStack = LinearLayout(context)
  private val hint = TextView(context)
  private val modeButtons = ArrayList<Button>()
  private val gamepadSwitch = Switch(context)
  private var drawerAnimator: ValueAnimator? = null
  private val location = IntArray(2)

  private enum class RootRoute { NONE, PASSTHROUGH, CONTROLS, EDGE, EMPTY_GAMEPAD, BLOCKED }
  private var rootRoute = RootRoute.NONE
  private var rootControlsAreGamepad = false
  private var gesturePointer = MotionEvent.INVALID_POINTER_ID
  private var downX = 0f
  private var downY = 0f
  private var panStart = 0f
  private var dragging = false
  private var dragCandidate = false
  private var edgeRejected = false
  private var velocity: VelocityTracker? = null
  private val preferenceListener = SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
    if (key == TOUCH_MODE_KEY || key == GAMEPAD_KEY) reloadPreferences()
  }

  init {
    setBackgroundColor(Color.TRANSPARENT)
    isMotionEventSplittingEnabled = false
    addView(gamepad, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    gamepad.onStateChanged = { onGamepadChanged?.invoke(gamepadEnabled, it) }
    scrim.apply {
      setBackgroundColor(Color.argb(128, 0, 0, 0))
      contentDescription = "Close stream menu"
      importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_YES
      setOnClickListener { setMenuOpen(false) }
    }
    addView(scrim, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    drawer.apply {
      setBackgroundColor(Color.argb(250, 14, 19, 31))
      isFillViewport = false
      clipToPadding = false
      contentDescription = "Stream controls"
    }
    menuStack.orientation = LinearLayout.VERTICAL
    drawer.addView(menuStack, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
    addView(drawer, LayoutParams(dp(340f), LayoutParams.MATCH_PARENT))
    addMenuItem(TextView(context).apply {
      text = "Stream controls"
      setTextColor(Color.WHITE)
      textSize = 23f
      setTypeface(typeface, Typeface.BOLD)
      isAccessibilityHeadingCompat()
    })
    listOf("Trackpad", "Direct touch", "Click-to-use").forEachIndexed { index, title ->
      val mode = index + 1
      val button = menuButton(title) { selectTouchMode(mode) }
      button.tag = "stream.mode.$mode"
      modeButtons.add(button)
      addMenuItem(button)
    }
    hint.apply { setTextColor(Color.LTGRAY); textSize = 13f }
    addMenuItem(hint)
    gamepadSwitch.apply {
      text = "On-screen controller"
      setTextColor(Color.WHITE)
      textSize = 14f
      minHeight = dp(44f)
      switchPadding = dp(8f)
      isChecked = gamepadEnabled
      tag = "stream.gamepad"
      setOnCheckedChangeListener { _, enabled -> enableGamepad(enabled) }
    }
    addMenuItem(gamepadSwitch)
    addMenuItem(menuButton("Keyboard") { requestKeyboard() })
    addMenuItem(menuButton("Return to Noland") {
      releaseControls()
      setMenuOpen(false, animated = false)
      onDashboardRequested?.invoke()
    })
    addMenuItem(menuButton("Resume stream") { setMenuOpen(false) })
    ready = true
    updateModeLabels()
    applySafePadding()
    render()
  }

  private fun View.isAccessibilityHeadingCompat() {
    if (Build.VERSION.SDK_INT >= 28) isAccessibilityHeading = true
  }

  private fun menuButton(title: String, action: () -> Unit): Button = Button(context).apply {
    text = title
    isAllCaps = false
    textSize = 15f
    setTextColor(Color.WHITE)
    setTypeface(typeface, Typeface.BOLD)
    minHeight = dp(44f)
    minimumWidth = 0
    setPadding(dp(10f), dp(8f), dp(10f), dp(8f))
    background = rounded(Color.argb(204, 38, 38, 38))
    setOnClickListener { action() }
  }

  private fun rounded(color: Int) = GradientDrawable().apply {
    setColor(color)
    cornerRadius = dp(10f).toFloat()
  }

  private fun addMenuItem(view: View) {
    menuStack.addView(view, LinearLayout.LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT).apply {
      if (menuStack.childCount > 0) topMargin = dp(12f)
    })
  }

  private fun readMode(): Int = try {
    preferences.getInt(TOUCH_MODE_KEY, TRACKPAD).takeIf { it in TRACKPAD..CLICK_TO_USE } ?: TRACKPAD
  } catch (_: ClassCastException) { TRACKPAD }

  private fun readGamepad(): Boolean = try {
    preferences.getBoolean(GAMEPAD_KEY, false)
  } catch (_: ClassCastException) { false }

  private fun reloadPreferences() {
    val mode = readMode()
    val enabled = readGamepad()
    if (mode != touchMode) {
      releaseControls()
      blockStreamSequence()
      touchMode = mode
      updateModeLabels()
      onModeChanged?.invoke(mode)
    }
    if (enabled != gamepadEnabled) applyGamepadEnabled(enabled)
  }

  fun selectTouchMode(mode: Int) {
    require(mode in TRACKPAD..CLICK_TO_USE) { "Touch mode must be 1, 2, or 3" }
    releaseControls()
    blockStreamSequence()
    val changed = touchMode != mode
    touchMode = mode
    preferences.edit().putInt(TOUCH_MODE_KEY, mode).apply()
    updateModeLabels()
    if (changed) onModeChanged?.invoke(mode)
  }

  fun enableGamepad(enabled: Boolean) {
    if (enabled == gamepadEnabled) return
    applyGamepadEnabled(enabled)
    preferences.edit().putBoolean(GAMEPAD_KEY, enabled).apply()
  }

  private fun applyGamepadEnabled(enabled: Boolean) {
    // Release under the old enabled flag before announcing the new connection state.
    releaseControls()
    blockStreamSequence()
    gamepadEnabled = enabled
    gamepadSwitch.isChecked = enabled
    render()
    onGamepadChanged?.invoke(enabled, gamepadState)
  }

  private fun updateModeLabels() {
    modeButtons.forEachIndexed { index, button ->
      button.isSelected = index + 1 == touchMode
      button.background = rounded(if (button.isSelected) Color.rgb(20, 92, 128) else Color.argb(204, 38, 38, 38))
    }
    hint.text = when (touchMode) {
      TRACKPAD -> "Slide to move the cursor. Tap to click, hold to drag. Two fingers scroll or right-click."
      DIRECT_TOUCH -> "Touch to press at that position, then move to drag. Hold still to right-click."
      else -> "Tap to move the cursor there and click. Slide to position without clicking; hold to drag. Two-finger tap right-clicks."
    }
  }

  /** Set true BEFORE requesting the IME; set false only when the parent knows it is dismissed. */
  fun setKeyboardVisible(visible: Boolean) {
    if (keyboardShown == visible) return
    keyboardShown = visible
    blockStreamSequence()
    releaseControls()
    if (visible) {
      clearGesture()
      setMenuOpen(false, animated = false)
    }
    render()
  }

  fun requestKeyboard() {
    setKeyboardVisible(true)
    onKeyboardRequested?.invoke()
  }

  fun toggleMenu() = setMenuOpen(!menuVisible)

  fun setMenuOpen(open: Boolean, animated: Boolean = true) {
    if (open && keyboardVisible) return
    cancelDrawerAnimation()
    clearGesture()
    releaseControls()
    blockStreamSequence()
    if (open) changeMenuVisibility(true)
    val target = if (open) 1f else 0f
    if (!animated || width == 0 || drawerProgress == target) {
      setProgress(target)
      changeMenuVisibility(open)
      return
    }
    drawerAnimator = ValueAnimator.ofFloat(drawerProgress, target).apply {
      duration = 200L
      addUpdateListener { setProgress(it.animatedValue as Float) }
      addListener(object : AnimatorListenerAdapter() {
        override fun onAnimationEnd(animation: Animator) {
          drawerAnimator = null
          setProgress(target)
          changeMenuVisibility(open)
        }
      })
      start()
    }
  }

  private fun changeMenuVisibility(visible: Boolean) {
    if (menuVisible == visible) { render(); return }
    menuVisible = visible
    render()
    onMenuChanged?.invoke(visible)
    if (visible) menuStack.sendAccessibilityEvent(android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED)
  }

  private fun cancelDrawerAnimation() {
    drawerAnimator?.let {
      it.removeAllListeners()
      it.removeAllUpdateListeners()
      it.cancel()
    }
    drawerAnimator = null
  }

  private fun setProgress(progress: Float) {
    drawerProgress = progress.coerceIn(0f, 1f)
    render()
  }

  private fun render() {
    if (!ready) return
    // Keep geometry measured while hidden so the first edge MOVE has a real drawer width.
    drawer.visibility = if (menuVisible) VISIBLE else INVISIBLE
    scrim.visibility = if (menuVisible) VISIBLE else INVISIBLE
    scrim.alpha = drawerProgress
    drawer.translationX = -drawer.width * (1f - drawerProgress)
    gamepad.visibility = if (gamepadEnabled && !menuVisible && !keyboardVisible) VISIBLE else INVISIBLE
  }

  /** Only menu/scrim and actual gamepad widgets are controls; empty space remains transparent. */
  fun isControlTouch(x: Float, y: Float): Boolean {
    if (!isShown || !isEnabled || alpha <= 0.01f || x < 0f || y < 0f || x >= width || y >= height) return false
    return menuVisible || (gamepadEnabled && !keyboardVisible && gamepad.isControlTouch(x, y))
  }

  /**
   * See class contract. True means return true directly from Activity.dispatchTouchEvent.
   * False means continue NORMAL view dispatch, never send the event directly to the surface.
   */
  fun onRootTouchEvent(event: MotionEvent): Boolean {
    getLocationOnScreen(location)
    val x = event.rawX - location[0]
    val y = event.rawY - location[1]
    if (event.actionMasked == MotionEvent.ACTION_DOWN) {
      clearGesture()
      rootControlsAreGamepad = false
      val inBounds = x >= 0f && x < width && y >= 0f && y < height
      val available = isShown && isEnabled && alpha > 0.01f
      rootRoute = when {
        !inBounds -> RootRoute.PASSTHROUGH
        available && isControlTouch(x, y) -> {
          rootControlsAreGamepad = !menuVisible
          RootRoute.CONTROLS
        }
        keyboardVisible -> RootRoute.BLOCKED
        available && !menuVisible && event.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER &&
          x <= max(dp(44f), safeLeft + dp(24f)) -> {
          prepareGesture(event, x, y, 0f)
          edgeRejected = false
          RootRoute.EDGE
        }
        gamepadEnabled && available -> RootRoute.EMPTY_GAMEPAD
        gamepadEnabled || menuVisible -> RootRoute.BLOCKED
        else -> RootRoute.PASSTHROUGH
      }
    }
    if (rootRoute == RootRoute.CONTROLS &&
      (if (rootControlsAreGamepad) gamepad.visibility != VISIBLE || !isShown || !isEnabled else !menuVisible || !isShown)) {
      rootRoute = RootRoute.BLOCKED
    }
    val consumed = when (rootRoute) {
      RootRoute.EDGE -> { trackEdge(event, x, y); true }
      RootRoute.EMPTY_GAMEPAD -> {
        val local = MotionEvent.obtain(event)
        try {
          local.offsetLocation(event.rawX - event.x - location[0], event.rawY - event.y - location[1])
          gamepad.onRootTouchEvent(local)
        } finally { local.recycle() }
        true
      }
      RootRoute.BLOCKED -> true
      else -> false
    }
    if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) {
      rootRoute = RootRoute.NONE
    }
    return consumed
  }

  private fun blockStreamSequence() {
    if (rootRoute == RootRoute.PASSTHROUGH || rootRoute == RootRoute.EMPTY_GAMEPAD ||
      (rootRoute == RootRoute.CONTROLS && rootControlsAreGamepad)) rootRoute = RootRoute.BLOCKED
  }

  private fun prepareGesture(event: MotionEvent, x: Float, y: Float, start: Float) {
    clearGesture()
    gesturePointer = event.getPointerId(0)
    downX = x; downY = y; panStart = start
    dragCandidate = true
    velocity = VelocityTracker.obtain().also { it.addMovement(event) }
  }

  private fun beginDrag() {
    cancelDrawerAnimation()
    dragging = true
    releaseControls()
    changeMenuVisibility(true)
  }

  private fun trackEdge(event: MotionEvent, x: Float, y: Float) {
    if (event.actionMasked == MotionEvent.ACTION_DOWN || edgeRejected) return
    velocity?.addMovement(event)
    when (event.actionMasked) {
      MotionEvent.ACTION_POINTER_DOWN -> rejectEdge()
      MotionEvent.ACTION_MOVE -> {
        if (event.pointerCount != 1 || event.getPointerId(0) != gesturePointer) { rejectEdge(); return }
        val dx = x - downX; val dy = y - downY
        if (!dragging && (dx < -touchSlop || (abs(dy) > touchSlop && abs(dy) >= abs(dx)))) {
          rejectEdge(); return
        }
        if (!dragging && dx > touchSlop && dx > abs(dy)) beginDrag()
        if (dragging) setProgress(dx / max(1, drawer.width))
      }
      MotionEvent.ACTION_UP -> {
        if (dragging) finishDrag(cancelled = false) else clearGesture()
      }
      MotionEvent.ACTION_CANCEL -> {
        if (dragging) finishDrag(cancelled = true) else clearGesture()
      }
    }
  }

  private fun rejectEdge() {
    val wasDragging = dragging
    edgeRejected = true
    if (wasDragging) setMenuOpen(false) else clearGesture()
  }

  private fun finishDrag(cancelled: Boolean) {
    velocity?.computeCurrentVelocity(1000)
    val speed = velocity?.getXVelocity(gesturePointer) ?: 0f
    val open = if (cancelled) panStart > 0f else if (abs(speed) > dp(300f)) speed > 0f else drawerProgress > 0.4f
    setMenuOpen(open)
  }

  private fun clearGesture() {
    velocity?.recycle()
    velocity = null
    gesturePointer = MotionEvent.INVALID_POINTER_ID
    dragging = false
    dragCandidate = false
  }

  /** A drawer drag may start on background/labels, never on a Button or Switch. */
  override fun onInterceptTouchEvent(event: MotionEvent): Boolean {
    if (dragging) return true
    when (event.actionMasked) {
      MotionEvent.ACTION_DOWN -> {
        clearGesture()
        val x = event.x - drawer.translationX
        if (menuVisible && event.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER &&
          x >= 0f && x < drawer.width && !interactiveAt(drawer, x, event.y)) {
          prepareGesture(event, event.x, event.y, drawerProgress)
        }
      }
      MotionEvent.ACTION_MOVE -> if (dragCandidate && event.pointerCount == 1) {
        val dx = event.x - downX; val dy = event.y - downY
        if (abs(dx) > touchSlop && abs(dx) > abs(dy)) {
          beginDrag()
          return true
        }
        velocity?.addMovement(event)
        if (abs(dy) > touchSlop) clearGesture()
      }
      MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> clearGesture()
    }
    return false
  }

  override fun onTouchEvent(event: MotionEvent): Boolean {
    if (!dragging) return false
    velocity?.addMovement(event)
    when (event.actionMasked) {
      MotionEvent.ACTION_MOVE -> {
        val index = event.findPointerIndex(gesturePointer)
        if (index < 0 || event.pointerCount != 1) finishDrag(cancelled = true)
        else setProgress(panStart + (event.getX(index) - downX) / max(1, drawer.width))
      }
      MotionEvent.ACTION_UP -> finishDrag(cancelled = false)
      MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN -> finishDrag(cancelled = true)
    }
    return true
  }

  private fun interactiveAt(view: View, x: Float, y: Float): Boolean {
    if (view is ViewGroup) {
      for (index in view.childCount - 1 downTo 0) {
        val child = view.getChildAt(index)
        if (child.visibility != VISIBLE) continue
        val cx = x + view.scrollX - child.left - child.translationX
        val cy = y + view.scrollY - child.top - child.translationY
        if (cx >= 0f && cx < child.width && cy >= 0f && cy < child.height && interactiveAt(child, cx, cy)) return true
      }
    }
    return view !== drawer && (view.isClickable || view.isLongClickable)
  }

  /** Fallback for parents that consume WindowInsets: values are physical pixels, not dp. */
  fun setSafeInsets(left: Int, top: Int, right: Int, bottom: Int) {
    val l = max(0, left); val t = max(0, top); val r = max(0, right); val b = max(0, bottom)
    if (safeLeft == l && safeTop == t && safeRight == r && safeBottom == b) return
    reset()
    safeLeft = l; safeTop = t; safeRight = r; safeBottom = b
    gamepad.setSafeInsets(l, t, r, b)
    applySafePadding()
  }

  private fun applySafePadding() {
    menuStack.setPadding(max(dp(16f), safeLeft), safeTop + dp(16f), dp(16f), safeBottom + dp(16f))
    requestLayout()
  }

  @Suppress("DEPRECATION")
  override fun onApplyWindowInsets(insets: WindowInsets): WindowInsets {
    if (Build.VERSION.SDK_INT >= 30) {
      val safe = insets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
      setSafeInsets(safe.left, safe.top, safe.right, safe.bottom)
    } else {
      val cutout = if (Build.VERSION.SDK_INT >= 28) insets.displayCutout else null
      setSafeInsets(max(insets.systemWindowInsetLeft, cutout?.safeInsetLeft ?: 0),
        max(insets.systemWindowInsetTop, cutout?.safeInsetTop ?: 0),
        max(insets.systemWindowInsetRight, cutout?.safeInsetRight ?: 0),
        max(insets.systemWindowInsetBottom, cutout?.safeInsetBottom ?: 0))
    }
    return insets // Siblings/parent still own IME and system-bar policy.
  }

  override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
    val width = MeasureSpec.getSize(widthMeasureSpec)
    val drawerWidth = min(dp(340f), (width * 0.8f).roundToInt())
    if (drawer.layoutParams.width != drawerWidth) drawer.layoutParams.width = drawerWidth
    super.onMeasure(widthMeasureSpec, heightMeasureSpec)
  }

  override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
    super.onSizeChanged(w, h, oldw, oldh)
    if (ready && (oldw != 0 || oldh != 0)) reset()
  }

  override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
    super.onLayout(changed, left, top, right, bottom)
    render()
  }

  /** Emits zero even when already neutral; useful before replacing a stream generation. */
  fun releaseControls() {
    gamepad.releaseControls()
  }

  /** Closes transient UI and drains any owned sequence; persisted mode/pad preference is unchanged. */
  fun reset() {
    blockStreamSequence()
    if (rootRoute == RootRoute.EDGE || rootRoute == RootRoute.CONTROLS) rootRoute = RootRoute.BLOCKED
    clearGesture()
    cancelDrawerAnimation()
    releaseControls()
    setProgress(0f)
    changeMenuVisibility(false)
  }

  fun publishState() {
    onModeChanged?.invoke(touchMode)
    onMenuChanged?.invoke(menuVisible)
    onGamepadChanged?.invoke(gamepadEnabled, gamepadState)
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    preferences.registerOnSharedPreferenceChangeListener(preferenceListener)
    reloadPreferences()
    requestApplyInsets()
    publishState()
  }

  override fun onVisibilityChanged(changedView: View, visibility: Int) {
    super.onVisibilityChanged(changedView, visibility)
    if (ready && visibility != VISIBLE) reset()
  }

  override fun onWindowVisibilityChanged(visibility: Int) {
    super.onWindowVisibilityChanged(visibility)
    if (ready && visibility != VISIBLE) reset()
  }

  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    if (ready && !hasWindowFocus) reset()
  }

  override fun setEnabled(enabled: Boolean) {
    super.setEnabled(enabled)
    if (ready && !enabled) reset()
  }

  override fun onDetachedFromWindow() {
    reset()
    preferences.unregisterOnSharedPreferenceChangeListener(preferenceListener)
    super.onDetachedFromWindow()
  }

  private fun dp(value: Float): Int = (value * density).roundToInt()

  companion object {
    const val TRACKPAD = 1
    const val DIRECT_TOUCH = 2
    const val CLICK_TO_USE = 3
    const val PREFERENCES_NAME = "noland.stream.controls"
    const val TOUCH_MODE_KEY = "noland.stream.touchMode"
    const val GAMEPAD_KEY = "noland.stream.gamepad"
  }
}
                                                                   