package com.noland.connect.stream

import android.content.Context
import android.hardware.input.InputManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * Activity/bridge-independent physical input. Construct and call on the main thread.
 * Call start(), then announce() AGAIN when the native runtime accepts input: Sink has
 * no delivery acknowledgement, so an early announcement must not be considered delivered.
 * Announcements are ordinary controller snapshots; native owns controller arrival.
 *
 * Dispatch keys here BEFORE keyboard mapping. Dispatch generic motion and mouse touch
 * here BEFORE finger gestures/OSC filtering, including while suspended (to cache releases).
 * setSuspended(true) before menu/IME/background or native focus loss; resume native input
 * BEFORE setSuspended(false). Suspension releases output without disconnecting controllers.
 * Android cannot poll held keys/axes: resume reconciles devices and replays the latest known
 * snapshots, not hasKeys() (which reports capabilities). Events not dispatched cannot be polled.
 *
 * Mouse coordinates must be in the stream viewport's coordinate system. For Activity-local
 * events pass the viewport origin, or offset a copy of the event first; dimensions describe
 * that viewport, not decoded video pixels. The sink/native layer owns video/letterbox scaling.
 * For API 26+ the parent may requestPointerCapture() and route onCapturedPointerEvent here;
 * SOURCE_MOUSE_RELATIVE uses X/Y as deltas. This class never captures a View itself. On capture
 * loss call release(); on focus/menu loss use setSuspended(true), and release capture in parent.
 */
class StreamHardwareInput(context: Context, sink: Sink) : InputManager.InputDeviceListener, AutoCloseable {
  interface Sink {
    fun controller(number: Int, mask: Int, state: GamepadState)
    fun absolute(x: Int, y: Int, width: Int, height: Int)
    fun relative(dx: Int, dy: Int)
    /** Abstract IDs: 1 left, 2 right, 3 middle, 4 X1/back, 5 X2/forward. */
    fun button(button: Int, pressed: Boolean)
    /** High-resolution units are 1/120 of a wheel detent, in both directions. */
    fun scroll(vertical: Int, horizontal: Int, highRes: Boolean)
  }

  private val manager = context.applicationContext.getSystemService(Context.INPUT_SERVICE) as InputManager
  private val input = HardwareInputState(sink)
  private var started = false
  private var closed = false

  /** Idempotent; no registration or output is performed by the constructor. */
  fun start() {
    if (closed || started) return
    manager.registerInputDeviceListener(this, Handler(Looper.getMainLooper()))
    reconcileDevices()
    started = true
    input.start()
  }

  /** Always replays, even if nothing changed; call after native startup/reconnect. */
  fun announce() {
    if (closed || !started) return
    reconcileDevices()
    input.announce()
  }

  fun setVirtualState(enabled: Boolean, state: GamepadState) {
    if (!closed) input.setVirtualState(enabled, state)
  }

  fun setSuspended(suspended: Boolean) {
    if (closed) return
    if (!suspended && started) reconcileDevices()
    input.setSuspended(suspended)
  }

  /** Neutralizes and forgets held controls, but retains slots, registration and OSC enablement. */
  fun release() {
    if (!closed) input.release()
  }

  override fun close() {
    if (closed) return
    input.release()
    if (started) manager.unregisterInputDeviceListener(this)
    input.stop()
    started = false
    closed = true
  }

  override fun onInputDeviceAdded(deviceId: Int) {
    if (started && !closed) refreshDevice(deviceId)
  }

  override fun onInputDeviceChanged(deviceId: Int) {
    if (!started || closed) return
    // A changed mapping cannot safely retain keys/axes from the old mapping.
    input.resetDevice(deviceId)
    input.removeMouse(deviceId)
    refreshDevice(deviceId)
  }

  override fun onInputDeviceRemoved(deviceId: Int) {
    if (!started || closed) return
    input.disconnect(deviceId)
    input.removeMouse(deviceId)
    reconcileDevices() // A previously ignored 17th pad can now take the free slot.
  }

  private fun reconcileDevices() {
    val ids = InputDevice.getDeviceIds().toSet()
    input.deviceIds.filter { it !in ids }.forEach { input.disconnect(it) }
    input.mouseIds.filter { it !in ids }.forEach { input.removeMouse(it) }
    ids.sorted().forEach { refreshDevice(it) }
  }

  private fun refreshDevice(id: Int) {
    val device = InputDevice.getDevice(id)
    if (device != null && isGamepad(device)) input.connect(id) else input.disconnect(id)
  }

  private fun isGamepad(device: InputDevice): Boolean = !device.isVirtual &&
    (device.supportsSource(InputDevice.SOURCE_GAMEPAD) || device.supportsSource(InputDevice.SOURCE_JOYSTICK))

  fun handleKeyEvent(event: KeyEvent): Boolean {
    if (closed || !started) return false
    val device = event.device ?: return false
    if (!isGamepad(device) || !HardwareInputState.isMappedKey(event.keyCode)) return false
    if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return false
    input.connect(device.id)
    input.key(device.id, event.keyCode, event.action == KeyEvent.ACTION_DOWN && !event.isCanceled)
    return true // Also consume mapped keys when all 16 slots are occupied or suspended.
  }

  @JvmOverloads
  fun handleGenericMotionEvent(event: MotionEvent, viewWidth: Int, viewHeight: Int,
    originX: Float = 0f, originY: Float = 0f): Boolean {
    if (closed || !started) return false
    if (isMouse(event)) return mouse(event, viewWidth, viewHeight, originX, originY)
    val device = event.device ?: return false
    if (!isGamepad(device) || !event.isFromSource(InputDevice.SOURCE_JOYSTICK)) return false
    if (event.actionMasked != MotionEvent.ACTION_MOVE && event.actionMasked != MotionEvent.ACTION_CANCEL) return false
    input.connect(device.id)
    if (event.actionMasked == MotionEvent.ACTION_CANCEL) input.resetDevice(device.id)
    else input.axes(device.id, gamepadAxes(device, event))
    return true
  }

  /** Finger, stylus and touchpad events return false; physical mice bypass OSC finger handling. */
  @JvmOverloads
  fun handleTouchEvent(event: MotionEvent, viewWidth: Int, viewHeight: Int,
    originX: Float = 0f, originY: Float = 0f): Boolean =
    !closed && started && isMouse(event) && mouse(event, viewWidth, viewHeight, originX, originY)

  private fun isMouse(event: MotionEvent): Boolean = event.isFromSource(InputDevice.SOURCE_MOUSE) ||
    (Build.VERSION.SDK_INT >= 26 && event.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE))

  private fun mouse(event: MotionEvent, width: Int, height: Int, originX: Float, originY: Float): Boolean {
    when (event.actionMasked) {
      MotionEvent.ACTION_CANCEL -> input.removeMouse(event.deviceId)
      MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_MOVE,
      MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_BUTTON_PRESS,
      MotionEvent.ACTION_BUTTON_RELEASE, MotionEvent.ACTION_SCROLL -> {
        val relative = Build.VERSION.SDK_INT >= 26 && event.isFromSource(InputDevice.SOURCE_MOUSE_RELATIVE)
        if (event.actionMasked != MotionEvent.ACTION_SCROLL) {
          if (relative) input.relative(event.deviceId, event.x, event.y)
          else input.absolute(event.x - originX, event.y - originY, width, height)
        }
        var buttons = event.buttonState
        // actionButton is authoritative even on drivers with a stale buttonState at an edge.
        if (event.actionMasked == MotionEvent.ACTION_BUTTON_PRESS) buttons = buttons or event.actionButton
        if (event.actionMasked == MotionEvent.ACTION_BUTTON_RELEASE) buttons = buttons and event.actionButton.inv()
        if (event.actionMasked == MotionEvent.ACTION_DOWN && buttons == 0) buttons = MotionEvent.BUTTON_PRIMARY
        if (event.actionMasked == MotionEvent.ACTION_UP) buttons = 0
        input.mouseButtons(event.deviceId, buttons)
        if (event.actionMasked == MotionEvent.ACTION_SCROLL) input.scroll(event.deviceId,
          event.getAxisValue(MotionEvent.AXIS_VSCROLL), event.getAxisValue(MotionEvent.AXIS_HSCROLL))
      }
      // HOVER_EXIT is not button-up: dragging outside the viewport must not end a drag.
      MotionEvent.ACTION_HOVER_EXIT -> Unit
      else -> return false
    }
    return true
  }

  private fun gamepadAxes(device: InputDevice, event: MotionEvent): GamepadState {
    fun range(axis: Int) = device.getMotionRange(axis, event.source) ?: device.getMotionRange(axis)
    fun stick(axis: Int, invert: Boolean = false): Int {
      val r = range(axis) ?: return 0
      return HardwareInputState.stick(event.getAxisValue(axis), r.min, r.max, r.flat, invert)
    }
    fun trigger(vararg alternatives: Int): Int = alternatives.maxOf { axis ->
      val r = range(axis)
      if (r == null) 0 else HardwareInputState.trigger(event.getAxisValue(axis), r.min, r.max, r.flat)
    }
    val hatX = stick(MotionEvent.AXIS_HAT_X)
    val hatY = stick(MotionEvent.AXIS_HAT_Y)
    val hats = (if (hatX < 0) GamepadState.LEFT else if (hatX > 0) GamepadState.RIGHT else 0) or
      (if (hatY < 0) GamepadState.UP else if (hatY > 0) GamepadState.DOWN else 0)
    // RX/RY pads may use Z/RZ for triggers; never read those as their right stick.
    val hasRxRy = range(MotionEvent.AXIS_RX) != null && range(MotionEvent.AXIS_RY) != null
    val rightX = if (hasRxRy) MotionEvent.AXIS_RX else MotionEvent.AXIS_Z
    val rightY = if (hasRxRy) MotionEvent.AXIS_RY else MotionEvent.AXIS_RZ
    return GamepadState(hats,
      trigger(MotionEvent.AXIS_LTRIGGER, MotionEvent.AXIS_BRAKE),
      trigger(MotionEvent.AXIS_RTRIGGER, MotionEvent.AXIS_GAS),
      stick(MotionEvent.AXIS_X), stick(MotionEvent.AXIS_Y, true), stick(rightX), stick(rightY, true))
  }
}

/** Pure routing/state/math, shared by the Android adapter and device-free JVM regression tests. */
internal class HardwareInputState(private val sink: StreamHardwareInput.Sink) {
  private class Pad(val slot: Int) {
    val keys = mutableSetOf<Int>()
    var axes = GamepadState.ZERO
    fun state(): GamepadState = axes.copy(
      buttons = keys.fold(axes.buttons) { flags, key -> flags or keyFlag(key) },
      leftTrigger = max(axes.leftTrigger, if (KeyEvent.KEYCODE_BUTTON_L2 in keys) 255 else 0),
      rightTrigger = max(axes.rightTrigger, if (KeyEvent.KEYCODE_BUTTON_R2 in keys) 255 else 0))
    fun reset() { keys.clear(); axes = GamepadState.ZERO }
  }
  private data class Mouse(var buttons: Int = 0, var dx: Double = 0.0, var dy: Double = 0.0,
    var vertical: Double = 0.0, var horizontal: Double = 0.0)

  private val pads = linkedMapOf<Int, Pad>()
  private val mice = linkedMapOf<Int, Mouse>()
  val deviceIds: List<Int> get() = pads.keys.toList()
  val mouseIds: List<Int> get() = mice.keys.toList()
  private var virtualEnabled = false
  private var virtual = GamepadState.ZERO
  private var running = false
  private var suspended = false
  private var lastMask = 1
  private val sent = mutableMapOf<Int, Pair<Int, GamepadState>>()
  private var sentMouseButtons = 0
  private val mask: Int get() = pads.values.fold(1) { bits, pad -> bits or (1 shl pad.slot) }

  fun start() { running = true; announce() }
  fun stop() { running = false; pads.clear(); mice.clear(); sent.clear() }
  fun announce() { if (running) publishAll(force = true, neutral = suspended) }

  fun connect(id: Int) {
    if (id in pads) return
    val slot = (0..15).firstOrNull { slot -> pads.values.none { it.slot == slot } } ?: return
    pads[id] = Pad(slot)
    if (running && !suspended) publishAll(force = true)
  }

  fun disconnect(id: Int) {
    if (pads.remove(id) != null && running && !suspended) publishAll(force = true)
  }

  fun resetDevice(id: Int) {
    pads[id]?.reset() ?: return
    publish(id)
  }

  fun key(id: Int, key: Int, pressed: Boolean) {
    val pad = pads[id] ?: return
    if (pressed) pad.keys.add(key) else pad.keys.remove(key)
    publish(id)
  }

  fun axes(id: Int, state: GamepadState) {
    val pad = pads[id] ?: return
    pad.axes = safe(state)
    publish(id)
  }

  fun setVirtualState(enabled: Boolean, state: GamepadState) {
    virtualEnabled = enabled
    virtual = if (enabled) safe(state) else GamepadState.ZERO
    if (running && !suspended) emit(0, output(0), mask, false)
  }

  fun setSuspended(value: Boolean) {
    if (suspended == value) return
    suspended = value
    releaseMouse()
    if (running) publishAll(force = true, neutral = value)
  }

  fun release() {
    pads.values.forEach { it.reset() }
    virtual = GamepadState.ZERO
    releaseMouse()
    if (running && !suspended) publishAll(force = true)
  }

  private fun output(slot: Int): GamepadState {
    val physical = pads.values.firstOrNull { it.slot == slot }?.state() ?: GamepadState.ZERO
    return if (slot == 0 && virtualEnabled) merge(virtual, physical) else physical
  }

  private fun publish(id: Int) {
    if (!running || suspended) return
    val slot = pads[id]?.slot ?: return
    emit(slot, output(slot), mask, false)
  }

  private fun publishAll(force: Boolean, neutral: Boolean = false) {
    val active = mask
    // Send the removed slot too, with the reduced mask and a neutral snapshot.
    for (slot in 0..15) if ((active or lastMask) and (1 shl slot) != 0) {
      emit(slot, if (neutral || active and (1 shl slot) == 0) GamepadState.ZERO else output(slot), active, force)
    }
    lastMask = active
  }

  private fun emit(slot: Int, state: GamepadState, mask: Int, force: Boolean) {
    val next = mask to state
    if (force || sent[slot] != next) {
      sink.controller(slot, mask, state)
      sent[slot] = next
    }
  }

  fun absolute(x: Float, y: Float, width: Int, height: Int) {
    if (!running || suspended || !x.isFinite() || !y.isFinite() || width <= 0 || height <= 0) return
    sink.absolute(x.coerceIn(0f, (width - 1).toFloat()).roundToInt(),
      y.coerceIn(0f, (height - 1).toFloat()).roundToInt(), width, height)
  }

  fun relative(id: Int, x: Float, y: Float) {
    if (!running || suspended) return
    val mouse = mice.getOrPut(id) { Mouse() }
    mouse.dx = accumulate(mouse.dx, x)
    mouse.dy = accumulate(mouse.dy, y)
    val dx = mouse.dx.toInt(); val dy = mouse.dy.toInt()
    mouse.dx -= dx; mouse.dy -= dy
    if (dx != 0 || dy != 0) sink.relative(dx, dy)
  }

  fun mouseButtons(id: Int, buttons: Int) {
    if (!running || suspended) return
    mice.getOrPut(id) { Mouse() }.buttons = buttons and 31
    publishMouseButtons()
  }

  fun scroll(id: Int, vertical: Float, horizontal: Float) {
    if (!running || suspended) return
    val mouse = mice.getOrPut(id) { Mouse() }
    mouse.vertical = accumulate(mouse.vertical, vertical, 120.0)
    mouse.horizontal = accumulate(mouse.horizontal, horizontal, 120.0)
    val v = mouse.vertical.toInt(); val h = mouse.horizontal.toInt()
    mouse.vertical -= v; mouse.horizontal -= h
    if (v != 0 || h != 0) sink.scroll(v, h, true)
  }

  fun removeMouse(id: Int) { if (mice.remove(id) != null) publishMouseButtons() }
  private fun releaseMouse() { mice.clear(); publishMouseButtons() }
  private fun publishMouseButtons() {
    val buttons = mice.values.fold(0) { bits, mouse -> bits or mouse.buttons }
    // Android PRIMARY/SECONDARY/TERTIARY/BACK/FORWARD bits correspond to these abstract IDs.
    for (index in 0..4) {
      val bit = 1 shl index
      if ((buttons xor sentMouseButtons) and bit != 0) sink.button(index + 1, buttons and bit != 0)
    }
    sentMouseButtons = buttons
  }

  companion object {
    const val GUIDE = 0x0400 // Moonlight SPECIAL_FLAG; not present in the virtual pad's constants.
    private fun keyFlag(key: Int): Int = when (key) {
      KeyEvent.KEYCODE_BUTTON_A, KeyEvent.KEYCODE_DPAD_CENTER -> GamepadState.A
      KeyEvent.KEYCODE_BUTTON_B -> GamepadState.B
      KeyEvent.KEYCODE_BUTTON_X -> GamepadState.X
      KeyEvent.KEYCODE_BUTTON_Y -> GamepadState.Y
      KeyEvent.KEYCODE_BUTTON_L1 -> GamepadState.LB
      KeyEvent.KEYCODE_BUTTON_R1 -> GamepadState.RB
      KeyEvent.KEYCODE_BUTTON_THUMBL -> GamepadState.L3
      KeyEvent.KEYCODE_BUTTON_THUMBR -> GamepadState.R3
      KeyEvent.KEYCODE_BUTTON_SELECT, KeyEvent.KEYCODE_BACK -> GamepadState.SELECT
      KeyEvent.KEYCODE_BUTTON_START, KeyEvent.KEYCODE_MENU -> GamepadState.START
      KeyEvent.KEYCODE_BUTTON_MODE -> GUIDE
      KeyEvent.KEYCODE_DPAD_UP -> GamepadState.UP
      KeyEvent.KEYCODE_DPAD_DOWN -> GamepadState.DOWN
      KeyEvent.KEYCODE_DPAD_LEFT -> GamepadState.LEFT
      KeyEvent.KEYCODE_DPAD_RIGHT -> GamepadState.RIGHT
      else -> 0
    }
    fun isMappedKey(key: Int): Boolean = keyFlag(key) != 0 ||
      key == KeyEvent.KEYCODE_BUTTON_L2 || key == KeyEvent.KEYCODE_BUTTON_R2

    fun safe(state: GamepadState): GamepadState = state.copy(
      leftTrigger = state.leftTrigger.coerceIn(0, 255), rightTrigger = state.rightTrigger.coerceIn(0, 255),
      leftX = state.leftX.coerceIn(-32767, 32767), leftY = state.leftY.coerceIn(-32767, 32767),
      rightX = state.rightX.coerceIn(-32767, 32767), rightY = state.rightY.coerceIn(-32767, 32767))

    fun merge(virtual: GamepadState, physical: GamepadState): GamepadState {
      val v = safe(virtual); val p = safe(physical)
      fun axis(v: Int, p: Int) = if (abs(p) >= abs(v)) p else v
      return GamepadState(v.buttons or p.buttons, max(v.leftTrigger, p.leftTrigger),
        max(v.rightTrigger, p.rightTrigger), axis(v.leftX, p.leftX), axis(v.leftY, p.leftY),
        axis(v.rightX, p.rightX), axis(v.rightY, p.rightY))
    }

    /** Flat is in raw axis units. Subtract/re-scale the deadzone for a continuous response. */
    fun stick(value: Float, min: Float, max: Float, flat: Float, invert: Boolean = false): Int {
      if (!validRange(value, min, max, flat) || min >= 0f || max <= 0f) return 0
      val v = value.coerceIn(min, max)
      val extent = if (v < 0f) -min.toDouble() else max.toDouble()
      val dead = flat.toDouble().coerceAtLeast(0.0)
      if (abs(v.toDouble()) <= dead || extent <= dead) return 0
      val magnitude = ((abs(v.toDouble()) - dead) / (extent - dead)).coerceIn(0.0, 1.0)
      val sign = (if (v < 0f) -1 else 1) * (if (invert) -1 else 1)
      return (magnitude * 32767 * sign).roundToInt()
    }

    fun trigger(value: Float, min: Float, max: Float, flat: Float): Int {
      if (!validRange(value, min, max, flat)) return 0
      val span = max.toDouble() - min.toDouble()
      val dead = flat.toDouble().coerceAtLeast(0.0)
      val distance = value.coerceIn(min, max).toDouble() - min.toDouble()
      if (distance <= dead || span <= dead) return 0
      return (((distance - dead) / (span - dead)).coerceIn(0.0, 1.0) * 255).roundToInt()
    }

    private fun validRange(value: Float, min: Float, max: Float, flat: Float): Boolean =
      value.isFinite() && min.isFinite() && max.isFinite() && flat.isFinite() && max > min

    private fun accumulate(remainder: Double, value: Float, scale: Double = 1.0): Double =
      if (value.isFinite()) (remainder + value.toDouble() * scale).coerceIn(-32767.0, 32767.0) else remainder
  }
}
