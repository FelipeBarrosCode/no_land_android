package com.noland.connect.stream

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Rect
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Gravity
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Surface
import android.view.View
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.WindowManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.TextView
import android.window.OnBackInvokedCallback
import android.window.OnBackInvokedDispatcher
import androidx.annotation.Keep
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

/** Native media stays in AMediaCodec/OpenSL ES; these views own controls and input only. */
@Keep
class StreamActivity : Activity(), StreamSurfaceView.Listener {
  internal var generation = 0L
    private set
  private var incarnation = 0L
  private var closedFromRust = false
  private var dashboardLeaving = false
  private var unregisterBack: (() -> Unit)? = null
  private lateinit var content: StreamRoot
  private lateinit var streamView: StreamSurfaceView
  private lateinit var controls: StreamControlsOverlay
  private lateinit var keyboard: StreamKeyboardView
  private lateinit var keyboardToolbar: FrameLayout
  private lateinit var statistics: TextView
  private lateinit var hardware: StreamHardwareInput
  private val handler = Handler(Looper.getMainLooper())
  private val heldKeys = mutableSetOf<Int>()
  private var imeWasVisible = false
  private var safeLeft = 0
  private var safeTop = 0
  private var safeRight = 0
  private var keyboardSequence = false
  private var toolbarSequence = false
  private val rootLocation = IntArray(2)
  private val viewLocation = IntArray(2)

  // Recognize three fingers before the virtual pad consumes empty-space sequences.
  // This controller is mouse-suspended; the SurfaceView alone owns finger mouse input.
  private val keyboardGesture by lazy {
    StreamTouchController(
      StreamTouchController.Clock { SystemClock.uptimeMillis() },
      StreamTouchController.Scheduler { delay, task ->
        val runnable = Runnable { task() }
        handler.postDelayed(runnable, delay)
        StreamTouchController.Cancellation { handler.removeCallbacks(runnable) }
      }, resources.displayMetrics.density,
    ).apply {
      inputSuspended = true
      sink = object : StreamTouchController.Sink {
        override fun absolute(x: Int, y: Int, width: Int, height: Int) {}
        override fun relative(dx: Int, dy: Int) {}
        override fun button(button: Int, pressed: Boolean) {}
        override fun scroll(vertical: Int, horizontal: Int, highResolution: Boolean) {}
        override fun keyboardRequested() { controls.requestKeyboard() }
      }
    }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    // Keep the decoder Surface stable when the IME overlays the window.
    window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING)
    content = StreamRoot(this).apply { setBackgroundColor(Color.BLACK) }
    streamView = StreamSurfaceView(this).also { it.listener = this }
    content.addView(streamView, FrameLayout.LayoutParams(-1, -1, Gravity.CENTER))
    statistics = TextView(this).apply {
      textSize = 9f
      typeface = Typeface.MONOSPACE
      setTextColor(Color.WHITE)
      setPadding(dp(4), dp(3), dp(4), dp(3))
      background = GradientDrawable().apply { setColor(0x73000000); cornerRadius = dp(4).toFloat() }
      visibility = View.GONE
      importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
    }
    content.addView(statistics, FrameLayout.LayoutParams(-2, -2, Gravity.TOP or Gravity.RIGHT))
    keyboard = StreamKeyboardView(this)
    content.addView(keyboard, FrameLayout.LayoutParams(1, 1))
    controls = StreamControlsOverlay(this)
    content.addView(controls, FrameLayout.LayoutParams(-1, -1))
    keyboardToolbar = FrameLayout(this).apply {
      setBackgroundColor(Color.rgb(14, 19, 31))
      visibility = View.GONE
      addView(Button(this@StreamActivity).apply {
        text = "Done"
        isAllCaps = false
        contentDescription = "Hide stream keyboard"
        setOnClickListener { keyboard.hideKeyboard() }
      }, FrameLayout.LayoutParams(dp(96), -1, Gravity.RIGHT))
    }
    content.addView(keyboardToolbar, FrameLayout.LayoutParams(-1, dp(44), Gravity.BOTTOM))
    setContentView(content)
    hardware = StreamHardwareInput(this, object : StreamHardwareInput.Sink {
      override fun controller(number: Int, mask: Int, state: GamepadState) {
        native("controller") {
          NativeStreamBridge.nativeController(generation, incarnation, number, mask, state.buttons,
            state.leftTrigger, state.rightTrigger, state.leftX, state.leftY, state.rightX, state.rightY)
        }
      }
      override fun absolute(x: Int, y: Int, width: Int, height: Int) = pointer(0, x, y, width, height)
      override fun relative(dx: Int, dy: Int) = pointer(1, dx, dy, 0, 0)
      override fun button(button: Int, pressed: Boolean) = mouseButton(button, pressed)
      override fun scroll(vertical: Int, horizontal: Int, highRes: Boolean) = scrollInput(vertical, horizontal, highRes)
    })
    hardware.setSuspended(true)
    content.capturedMotion = { event -> hardware.handleGenericMotionEvent(event, streamView.width, streamView.height) }
    content.captureLost = { hardware.release() }
    installControlCallbacks()
    installInsets()
    if (Build.VERSION.SDK_INT >= 33) {
      val dispatcher = onBackInvokedDispatcher
      val callback = OnBackInvokedCallback { back() }
      dispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback)
      unregisterBack = { dispatcher.unregisterOnBackInvokedCallback(callback) }
    }
    bind(intent)
    hardware.start()
  }

  private fun installControlCallbacks() {
    controls.onModeChanged = { mode ->
      streamView.touchMode = TouchMode.entries.first { it.value == mode }
      streamView.cancelSurface()
    }
    controls.onMenuChanged = { visible ->
      streamView.cancelSurface()
      keyboardGesture.cancel()
      if (visible) {
        keyboard.hideKeyboard()
        heldKeys.clear()
        releaseCapture()
      }
      syncInput()
      if (!visible) hardware.announce()
    }
    controls.onGamepadChanged = { enabled, state ->
      streamView.inputSuspended = controls.isSurfaceMouseSuspended
      hardware.setVirtualState(enabled, state)
    }
    controls.onKeyboardRequested = {
      streamView.cancelSurface()
      keyboardGesture.cancel()
      releaseCapture()
      keyboardToolbar.visibility = View.VISIBLE
      syncInput()
      keyboard.showKeyboard()
      if (!keyboard.isRequested) keyboardDismissed()
    }
    controls.onDashboardRequested = { returnToDashboard() }
    keyboard.onText = { text -> native("text") { NativeStreamBridge.nativeText(generation, incarnation, text) } }
    keyboard.onKey = { key, pressed, modifiers -> sendKey(key, pressed, modifiers) }
    keyboard.onDismissed = { keyboardDismissed() }
  }

  private fun bind(intent: Intent) {
    val next = intent.getLongExtra(GENERATION, 0L)
    // A singleTask Activity may survive the dashboard handoff. Reclaim its
    // incarnation and republish the Surface even when the route token is unchanged.
    if (next == generation && incarnation != 0L && !dashboardLeaving && !closedFromRust) { present(); return }
    val claimed = NativeStreamBridge.claim(next)
    if (claimed == 0L) {
      if (incarnation == 0L) { closedFromRust = true; finish() }
      return // An old singleTask Intent must not retire the replacement route.
    }
    controls.reset()
    keyboard.hideKeyboard()
    hardware.release()
    streamView.touchSink = null
    generation = next
    incarnation = claimed
    closedFromRust = false
    dashboardLeaving = false
    NativeStreamBridge.register(this)
    streamView.videoWidth = intent.getIntExtra(VIDEO_WIDTH, 1920).coerceAtLeast(1)
    streamView.videoHeight = intent.getIntExtra(VIDEO_HEIGHT, 1080).coerceAtLeast(1)
    streamView.keyboardGesturesEnabled = false
    val token = generation
    val activity = incarnation
    streamView.touchSink = object : StreamTouchController.Sink {
      override fun absolute(x: Int, y: Int, width: Int, height: Int) {
        NativeStreamBridge.invoke("absolute pointer") {
          NativeStreamBridge.nativePointer(token, activity, 0, x.toFloat(), y.toFloat(), width, height)
        }
      }
      override fun relative(dx: Int, dy: Int) {
        NativeStreamBridge.invoke("relative pointer") {
          NativeStreamBridge.nativePointer(token, activity, 1, dx.toFloat(), dy.toFloat(), 0, 0)
        }
      }
      override fun button(button: Int, pressed: Boolean) {
        NativeStreamBridge.invoke("mouse button") { NativeStreamBridge.nativeButton(token, activity, button, pressed) }
      }
      override fun scroll(vertical: Int, horizontal: Int, highResolution: Boolean) {
        NativeStreamBridge.invoke("scroll") { NativeStreamBridge.nativeScroll(token, activity, vertical, horizontal, highResolution) }
      }
      override fun keyboardRequested() { controls.requestKeyboard() }
    }
    controls.publishState()
    streamView.requestLayout()
    streamView.publishSurface()
    present()
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    bind(intent)
  }

  @Suppress("DEPRECATION")
  internal fun present() {
    if (Build.VERSION.SDK_INT >= 30) {
      window.insetsController?.apply {
        hide(WindowInsets.Type.systemBars())
        systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
      }
    } else {
      window.decorView.systemUiVisibility = View.SYSTEM_UI_FLAG_FULLSCREEN or
        View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
    }
    if (::keyboard.isInitialized && !keyboard.isRequested && !controls.menuVisible) streamView.requestFocus()
    if (::hardware.isInitialized) { syncInput(); hardware.announce() }
  }

  internal fun updateStatistics(text: String) {
    if (closedFromRust || dashboardLeaving || isFinishing || isDestroyed) return
    statistics.text = text
    statistics.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
  }

  private fun syncInput() {
    if (!::hardware.isInitialized || incarnation == 0L) return
    val suspended = !hasWindowFocus() || dashboardLeaving || closedFromRust || controls.menuVisible || controls.keyboardVisible
    streamView.inputSuspended = suspended || controls.gamepadEnabled
    native("input suspension") { NativeStreamBridge.nativeSuspendInput(generation, incarnation, suspended) }
    hardware.setSuspended(suspended)
    if (suspended) releaseCapture()
  }

  private fun keyboardDismissed() {
    imeWasVisible = false
    controls.setKeyboardVisible(false)
    keyboardToolbar.visibility = View.GONE
    keyboardGesture.cancel()
    streamView.cancelSurface()
    syncInput()
    if (hasWindowFocus() && !controls.menuVisible && !dashboardLeaving) streamView.requestFocus()
  }

  private fun returnToDashboard() {
    streamView.cancelSurface()
    keyboard.hideKeyboard()
    controls.releaseControls()
    hardware.release()
    val accepted = try { NativeStreamBridge.nativeDashboard(generation, incarnation) } catch (_: Throwable) { false }
    if (!accepted) return
    dashboardLeaving = true
    heldKeys.clear()
    releaseCapture()
    statistics.text = ""
    statistics.visibility = View.GONE
    // MainActivity is singleTask, so Android may destroy this Activity. Rust retains
    // the session and recreates just its presentation when Return to stream is used.
    startActivity(Intent(this, Class.forName("com.noland.connect.MainActivity")).apply {
      addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
    })
  }

  override fun onSurfaceReady(surface: Surface, serial: Long, width: Int, height: Int) {
    if (incarnation == 0L || closedFromRust || dashboardLeaving) return
    if (NativeStreamBridge.surface(generation, incarnation, serial, surface, width, height)) {
      focus(hasWindowFocus())
      hardware.announce()
    }
  }

  override fun onSurfaceUnavailable(serial: Long) {
    native("surface lost") { NativeStreamBridge.nativeSurfaceLost(generation, incarnation, serial) }
  }

  private fun focus(focused: Boolean) {
    if (!::hardware.isInitialized) return
    if (!focused) {
      hardware.setSuspended(true)
      streamView.cancelSurface()
      controls.releaseControls()
      heldKeys.clear()
      releaseCapture()
    }
    native("focus") { NativeStreamBridge.nativeFocus(generation, incarnation, focused) }
    syncInput()
    if (focused) hardware.announce()
  }

  override fun onWindowFocusChanged(hasFocus: Boolean) {
    super.onWindowFocusChanged(hasFocus)
    focus(hasFocus)
    if (hasFocus && ::streamView.isInitialized) present()
  }

  override fun onPause() {
    focus(false)
    if (::keyboard.isInitialized) keyboard.hideKeyboard()
    keyboardGesture.cancel()
    super.onPause()
  }

  private fun back() {
    if (keyboard.handleBack()) return
    if (controls.menuVisible) { controls.setMenuOpen(false); return }
    native("stop") { NativeStreamBridge.nativeStop(generation, incarnation) }
    finish()
  }

  @Deprecated("Fallback for Android versions without predictive Back")
  override fun onBackPressed() = back()

  internal fun closeFromRust() {
    closedFromRust = true
    finish()
  }

  override fun onDestroy() {
    unregisterBack?.invoke()
    unregisterBack = null
    keyboardGesture.cancel()
    keyboard.hideKeyboard()
    controls.reset()
    hardware.close()
    focus(false)
    streamView.touchSink = null
    streamView.retireSurface()
    streamView.listener = null
    NativeStreamBridge.unregister(this)
    if (!closedFromRust && !dashboardLeaving && !isChangingConfigurations) {
      native("destroy stop") { NativeStreamBridge.nativeStop(generation, incarnation) }
    }
    super.onDestroy()
  }

  override fun dispatchTouchEvent(event: MotionEvent): Boolean {
    if (!::controls.isInitialized || !::hardware.isInitialized) return super.dispatchTouchEvent(event)
    if (event.actionMasked == MotionEvent.ACTION_DOWN) {
      keyboardToolbar.getLocationOnScreen(viewLocation)
      toolbarSequence = keyboardToolbar.visibility == View.VISIBLE &&
        event.rawX >= viewLocation[0] && event.rawX < viewLocation[0] + keyboardToolbar.width &&
        event.rawY >= viewLocation[1] && event.rawY < viewLocation[1] + keyboardToolbar.height
    }
    if (toolbarSequence) {
      // The IME scrim must not swallow the sibling Done toolbar's owned sequence.
      val handled = super.dispatchTouchEvent(event)
      if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) toolbarSequence = false
      return handled
    }
    if (!controls.menuVisible && !controls.keyboardVisible) {
      val (x, y) = viewportOrigin(event)
      if (hardware.handleTouchEvent(event, streamView.width, streamView.height, x, y)) return true
    }
    routeKeyboardGesture(event)
    if (controls.onRootTouchEvent(event)) { streamView.cancelSurface(); return true }
    streamView.inputSuspended = controls.isSurfaceMouseSuspended || !hasWindowFocus()
    return super.dispatchTouchEvent(event)
  }

  private fun routeKeyboardGesture(event: MotionEvent) {
    content.getLocationOnScreen(rootLocation)
    val local = MotionEvent.obtain(event)
    local.offsetLocation(event.rawX - event.x - rootLocation[0], event.rawY - event.y - rootLocation[1])
    try {
      if (event.actionMasked == MotionEvent.ACTION_DOWN) {
        keyboardGesture.cancel()
        keyboardSequence = !controls.menuVisible && !controls.keyboardVisible &&
          event.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER && !controls.isControlTouch(local.x, local.y)
      }
      if (event.actionMasked == MotionEvent.ACTION_POINTER_DOWN &&
        controls.isControlTouch(local.getX(event.actionIndex), local.getY(event.actionIndex))) {
        keyboardSequence = false
        keyboardGesture.cancel()
      }
      if (!keyboardSequence || controls.menuVisible || controls.keyboardVisible) return
      keyboardGesture.setViewport(content.width, content.height)
      keyboardGesture.keyboardLeftEdgePx = max(dp(44), safeLeft + dp(24)).toFloat()
      val action = when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> StreamTouchController.Action.DOWN
        MotionEvent.ACTION_POINTER_DOWN -> StreamTouchController.Action.POINTER_DOWN
        MotionEvent.ACTION_MOVE -> StreamTouchController.Action.MOVE
        MotionEvent.ACTION_POINTER_UP -> StreamTouchController.Action.POINTER_UP
        MotionEvent.ACTION_UP -> StreamTouchController.Action.UP
        MotionEvent.ACTION_CANCEL -> StreamTouchController.Action.CANCEL
        else -> return
      }
      val points = (0 until local.pointerCount).map {
        StreamTouchController.Pointer(local.getPointerId(it), local.getX(it), local.getY(it))
      }
      val changed = if (action == StreamTouchController.Action.POINTER_DOWN ||
        action == StreamTouchController.Action.POINTER_UP || action == StreamTouchController.Action.UP) local.getPointerId(local.actionIndex) else null
      keyboardGesture.onTouch(action, points, changed)
      if (action == StreamTouchController.Action.UP || action == StreamTouchController.Action.CANCEL) keyboardSequence = false
    } finally { local.recycle() }
  }

  override fun dispatchGenericMotionEvent(event: MotionEvent): Boolean {
    // Keep suspended stick/trigger releases current; only mouse motion belongs to local UI.
    if (::hardware.isInitialized && (event.isFromSource(InputDevice.SOURCE_JOYSTICK) ||
      (!controls.menuVisible && !controls.keyboardVisible))) {
      val (x, y) = viewportOrigin(event)
      if (hardware.handleGenericMotionEvent(event, streamView.width, streamView.height, x, y)) return true
    }
    return super.dispatchGenericMotionEvent(event)
  }

  private fun viewportOrigin(event: MotionEvent): Pair<Float, Float> {
    streamView.getLocationOnScreen(viewLocation)
    return viewLocation[0] - (event.rawX - event.x) to viewLocation[1] - (event.rawY - event.y)
  }

  override fun dispatchKeyEvent(event: KeyEvent): Boolean {
    val gamepadHandled = ::hardware.isInitialized && hardware.handleKeyEvent(event)
    if (::controls.isInitialized && (controls.menuVisible || controls.keyboardVisible) && gamepadHandled) {
      if (event.keyCode == KeyEvent.KEYCODE_BUTTON_B || event.keyCode == KeyEvent.KEYCODE_BACK ||
        event.keyCode == KeyEvent.KEYCODE_BUTTON_SELECT) {
        if (event.action == KeyEvent.ACTION_UP && !event.isCanceled) back()
        return true
      }
      if (event.keyCode == KeyEvent.KEYCODE_BUTTON_A) {
        val confirm = KeyEvent(event.downTime, event.eventTime, event.action, KeyEvent.KEYCODE_DPAD_CENTER,
          event.repeatCount, event.metaState, event.deviceId, event.scanCode, event.flags, event.source)
        return super.dispatchKeyEvent(confirm)
      }
      return super.dispatchKeyEvent(event)
    }
    if (gamepadHandled) return true
    if (event.keyCode == KeyEvent.KEYCODE_BACK) return super.dispatchKeyEvent(event)
    if (!::controls.isInitialized || controls.menuVisible || dashboardLeaving) return super.dispatchKeyEvent(event)
    val key = windowsVirtualKey(event.keyCode) ?: return super.dispatchKeyEvent(event)
    if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return super.dispatchKeyEvent(event)
    if (event.action == KeyEvent.ACTION_DOWN) {
      if (!heldKeys.add(key)) return true
    } else heldKeys.remove(key)
    val modifiers = (if (event.isShiftPressed) 1 else 0) or (if (event.isCtrlPressed) 2 else 0) or
      (if (event.isAltPressed) 4 else 0) or (if (event.isMetaPressed) 8 else 0)
    sendKey(key, event.action == KeyEvent.ACTION_DOWN && !event.isCanceled, modifiers)
    return true
  }

  private fun sendKey(key: Int, pressed: Boolean, modifiers: Int) {
    native("key") { NativeStreamBridge.nativeKey(generation, incarnation, key, pressed, modifiers) }
  }
  private fun pointer(kind: Int, x: Int, y: Int, width: Int, height: Int) {
    native("pointer") { NativeStreamBridge.nativePointer(generation, incarnation, kind, x.toFloat(), y.toFloat(), width, height) }
  }
  private fun mouseButton(button: Int, pressed: Boolean) {
    native("mouse button") { NativeStreamBridge.nativeButton(generation, incarnation, button, pressed) }
  }
  private fun scrollInput(vertical: Int, horizontal: Int, highResolution: Boolean) {
    native("scroll") { NativeStreamBridge.nativeScroll(generation, incarnation, vertical, horizontal, highResolution) }
  }
  private fun native(operation: String, block: () -> Unit) {
    if (incarnation != 0L) NativeStreamBridge.invoke(operation, block)
  }

  private fun releaseCapture() {
    if (Build.VERSION.SDK_INT >= 26 && ::content.isInitialized && content.hasPointerCapture()) content.releasePointerCapture()
  }

  @Suppress("DEPRECATION")
  private fun installInsets() {
    content.setOnApplyWindowInsetsListener { _, insets ->
      if (Build.VERSION.SDK_INT >= 30) {
        val safe = insets.getInsetsIgnoringVisibility(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
        safeLeft = safe.left; safeTop = safe.top; safeRight = safe.right
        val ime = insets.getInsets(WindowInsets.Type.ime())
        updateIme(insets.isVisible(WindowInsets.Type.ime()), ime.bottom)
      } else {
        safeLeft = insets.systemWindowInsetLeft
        safeTop = insets.systemWindowInsetTop
        safeRight = insets.systemWindowInsetRight
      }
      statistics.maxWidth = min(dp(245), (content.width * 0.4f).roundToInt()).coerceAtLeast(1)
      (statistics.layoutParams as FrameLayout.LayoutParams).apply {
        topMargin = safeTop + dp(6); rightMargin = safeRight + dp(6)
        statistics.layoutParams = this
      }
      insets
    }
    if (Build.VERSION.SDK_INT < 30) {
      content.viewTreeObserver.addOnGlobalLayoutListener {
        val visible = Rect()
        content.getWindowVisibleDisplayFrame(visible)
        val height = (content.rootView.height - visible.bottom).coerceAtLeast(0)
        updateIme(height > max(dp(100), content.height / 5), height)
      }
    }
    content.requestApplyInsets()
  }

  private fun updateIme(visible: Boolean, height: Int) {
    if (visible && keyboard.isRequested) imeWasVisible = true
    else if (!visible && imeWasVisible) keyboard.hideKeyboard()
    if (keyboard.isRequested) {
      (keyboardToolbar.layoutParams as FrameLayout.LayoutParams).apply {
        bottomMargin = if (visible) height else 0
        keyboardToolbar.layoutParams = this
      }
    }
  }

  private fun dp(value: Int) = (value * resources.displayMetrics.density).roundToInt()

  private class StreamRoot(context: Context) : FrameLayout(context) {
    var capturedMotion: ((MotionEvent) -> Boolean)? = null
    var captureLost: (() -> Unit)? = null
    override fun onCapturedPointerEvent(event: MotionEvent): Boolean = capturedMotion?.invoke(event) ?: false
    override fun onPointerCaptureChange(hasCapture: Boolean) {
      super.onPointerCaptureChange(hasCapture)
      if (!hasCapture) captureLost?.invoke()
    }
  }

  private fun windowsVirtualKey(key: Int): Int? = when (key) {
    in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> 0x41 + key - KeyEvent.KEYCODE_A
    in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 -> 0x30 + key - KeyEvent.KEYCODE_0
    in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> 0x70 + key - KeyEvent.KEYCODE_F1
    in KeyEvent.KEYCODE_NUMPAD_0..KeyEvent.KEYCODE_NUMPAD_9 -> 0x60 + key - KeyEvent.KEYCODE_NUMPAD_0
    KeyEvent.KEYCODE_DEL -> 0x08
    KeyEvent.KEYCODE_TAB -> 0x09
    KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> 0x0D
    KeyEvent.KEYCODE_ESCAPE -> 0x1B
    KeyEvent.KEYCODE_SPACE -> 0x20
    KeyEvent.KEYCODE_PAGE_UP -> 0x21
    KeyEvent.KEYCODE_PAGE_DOWN -> 0x22
    KeyEvent.KEYCODE_MOVE_HOME -> 0x24
    KeyEvent.KEYCODE_MOVE_END -> 0x23
    KeyEvent.KEYCODE_DPAD_LEFT -> 0x25
    KeyEvent.KEYCODE_DPAD_UP -> 0x26
    KeyEvent.KEYCODE_DPAD_RIGHT -> 0x27
    KeyEvent.KEYCODE_DPAD_DOWN -> 0x28
    KeyEvent.KEYCODE_INSERT -> 0x2D
    KeyEvent.KEYCODE_FORWARD_DEL -> 0x2E
    KeyEvent.KEYCODE_SHIFT_LEFT -> 0xA0
    KeyEvent.KEYCODE_SHIFT_RIGHT -> 0xA1
    KeyEvent.KEYCODE_CTRL_LEFT -> 0xA2
    KeyEvent.KEYCODE_CTRL_RIGHT -> 0xA3
    KeyEvent.KEYCODE_ALT_LEFT -> 0xA4
    KeyEvent.KEYCODE_ALT_RIGHT -> 0xA5
    KeyEvent.KEYCODE_META_LEFT -> 0x5B
    KeyEvent.KEYCODE_META_RIGHT -> 0x5C
    KeyEvent.KEYCODE_CAPS_LOCK -> 0x14
    KeyEvent.KEYCODE_SCROLL_LOCK -> 0x91
    KeyEvent.KEYCODE_NUM_LOCK -> 0x90
    KeyEvent.KEYCODE_SYSRQ -> 0x2C
    KeyEvent.KEYCODE_BREAK -> 0x13
    KeyEvent.KEYCODE_NUMPAD_DIVIDE -> 0x6F
    KeyEvent.KEYCODE_NUMPAD_MULTIPLY -> 0x6A
    KeyEvent.KEYCODE_NUMPAD_SUBTRACT -> 0x6D
    KeyEvent.KEYCODE_NUMPAD_ADD -> 0x6B
    KeyEvent.KEYCODE_NUMPAD_DOT -> 0x6E
    KeyEvent.KEYCODE_SEMICOLON -> 0xBA
    KeyEvent.KEYCODE_EQUALS -> 0xBB
    KeyEvent.KEYCODE_COMMA -> 0xBC
    KeyEvent.KEYCODE_MINUS -> 0xBD
    KeyEvent.KEYCODE_PERIOD -> 0xBE
    KeyEvent.KEYCODE_SLASH -> 0xBF
    KeyEvent.KEYCODE_GRAVE -> 0xC0
    KeyEvent.KEYCODE_LEFT_BRACKET -> 0xDB
    KeyEvent.KEYCODE_BACKSLASH -> 0xDC
    KeyEvent.KEYCODE_RIGHT_BRACKET -> 0xDD
    KeyEvent.KEYCODE_APOSTROPHE -> 0xDE
    else -> null
  }

  companion object {
    internal const val GENERATION = "com.noland.connect.stream.GENERATION"
    internal const val VIDEO_WIDTH = "com.noland.connect.stream.VIDEO_WIDTH"
    internal const val VIDEO_HEIGHT = "com.noland.connect.stream.VIDEO_HEIGHT"
  }
}
