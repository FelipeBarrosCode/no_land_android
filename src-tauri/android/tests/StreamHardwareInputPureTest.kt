package com.noland.connect.stream

import android.view.KeyEvent

private data class ControllerOutput(val number: Int, val mask: Int, val state: GamepadState)

private class HardwareRecordingSink : StreamHardwareInput.Sink {
  val controllers = mutableListOf<ControllerOutput>()
  val pointer = mutableListOf<String>()
  val held = mutableSetOf<Int>()
  override fun controller(number: Int, mask: Int, state: GamepadState) {
    check(number in 0..15 && mask in 1..65535 && mask and 1 == 1)
    check(state == HardwareInputState.safe(state))
    controllers.add(ControllerOutput(number, mask, state))
  }
  override fun absolute(x: Int, y: Int, width: Int, height: Int) { pointer.add("abs:$x,$y/$width,$height") }
  override fun relative(dx: Int, dy: Int) { pointer.add("rel:$dx,$dy") }
  override fun button(button: Int, pressed: Boolean) {
    check(button in 1..5)
    check(if (pressed) held.add(button) else held.remove(button)) { "Unbalanced mouse edge $button/$pressed" }
    pointer.add("button:$button/$pressed")
  }
  override fun scroll(vertical: Int, horizontal: Int, highRes: Boolean) {
    pointer.add("scroll:$vertical,$horizontal/$highRes")
  }
  fun last(slot: Int = 0): ControllerOutput = controllers.last { it.number == slot }
  fun clear() { controllers.clear(); pointer.clear() }
}

private class HardwareFixture(start: Boolean = true) {
  val sink = HardwareRecordingSink()
  val input = HardwareInputState(sink)
  init { if (start) input.start() }
  fun pad(id: Int = 100) { input.connect(id) }
  fun key(key: Int, down: Boolean = true, id: Int = 100) { input.key(id, key, down) }
  fun axes(state: GamepadState, id: Int = 100) { input.axes(id, state) }
  fun virtual(state: GamepadState, enabled: Boolean = true) { input.setVirtualState(enabled, state) }
}

private var hardwarePassed = 0
private fun hardwareTest(name: String, body: () -> Unit) {
  try { body(); hardwarePassed++; println("PASS $name") }
  catch (error: Throwable) { throw AssertionError("FAIL $name: ${error.message}", error) }
}

fun main() {
  hardwareTest("slot zero is always present with OSC off and without hardware") {
    val f = HardwareFixture()
    check(f.sink.controllers == listOf(ControllerOutput(0, 1, GamepadState.ZERO)))
    f.virtual(GamepadState(GamepadState.A), false)
    check(f.sink.last() == ControllerOutput(0, 1, GamepadState.ZERO))
  }
  hardwareTest("start and repeated announcements replay current state after an early native no-op") {
    val f = HardwareFixture(false)
    f.pad(); f.key(KeyEvent.KEYCODE_BUTTON_A); f.virtual(GamepadState(GamepadState.B))
    f.input.announce(); check(f.sink.controllers.isEmpty())
    f.input.start()
    val expected = ControllerOutput(0, 1, GamepadState(GamepadState.A or GamepadState.B))
    check(f.sink.last() == expected)
    f.input.announce(); f.input.announce()
    check(f.sink.controllers == List(3) { expected })
  }
  hardwareTest("slot zero ORs buttons, maximizes triggers, chooses each strongest axis with physical ties") {
    val f = HardwareFixture(); f.pad()
    f.virtual(GamepadState(GamepadState.A, 180, 10, 16000, -20000, -12000, 2000))
    f.axes(GamepadState(GamepadState.B, 40, 220, -26000, 20000, 8000, -2000))
    check(f.sink.last().state == GamepadState(GamepadState.A or GamepadState.B,
      180, 220, -26000, 20000, -12000, -2000))
  }
  hardwareTest("OSC disable and enable preserve physical buttons, triggers and axes") {
    val f = HardwareFixture(); f.pad(); f.key(KeyEvent.KEYCODE_BUTTON_B)
    val axes = GamepadState(leftTrigger = 201, leftX = -29000)
    f.axes(axes); f.virtual(GamepadState(GamepadState.A))
    f.virtual(GamepadState.ZERO, false)
    check(f.sink.last().state == axes.copy(buttons = GamepadState.B))
    f.virtual(GamepadState(GamepadState.X)); f.key(KeyEvent.KEYCODE_BUTTON_B, false)
    check(f.sink.last().state == axes.copy(buttons = GamepadState.X))
  }
  hardwareTest("all extended buttons use Moonlight flags, with no keyboard letters mapped") {
    val keys = listOf(KeyEvent.KEYCODE_BUTTON_A, KeyEvent.KEYCODE_BUTTON_B, KeyEvent.KEYCODE_BUTTON_X,
      KeyEvent.KEYCODE_BUTTON_Y, KeyEvent.KEYCODE_BUTTON_L1, KeyEvent.KEYCODE_BUTTON_R1,
      KeyEvent.KEYCODE_BUTTON_THUMBL, KeyEvent.KEYCODE_BUTTON_THUMBR, KeyEvent.KEYCODE_BUTTON_SELECT,
      KeyEvent.KEYCODE_BUTTON_START, KeyEvent.KEYCODE_BUTTON_MODE, KeyEvent.KEYCODE_DPAD_UP,
      KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT)
    val flags = listOf(0x1000, 0x2000, 0x4000, 0x8000, 0x100, 0x200, 0x40, 0x80,
      0x20, 0x10, 0x400, 1, 2, 4, 8)
    val f = HardwareFixture(); f.pad()
    keys.zip(flags).forEach { (key, flag) ->
      check(HardwareInputState.isMappedKey(key)); f.key(key)
      check(f.sink.last().state.buttons == flag); f.key(key, false)
      check(f.sink.last().state == GamepadState.ZERO)
    }
    check(!HardwareInputState.isMappedKey(KeyEvent.KEYCODE_A))
    check(!HardwareInputState.isMappedKey(KeyEvent.KEYCODE_SPACE))
  }
  hardwareTest("alias release, repeated keys and axis updates do not lose held keys") {
    val f = HardwareFixture(); f.pad()
    f.key(KeyEvent.KEYCODE_BUTTON_A); f.key(KeyEvent.KEYCODE_DPAD_CENTER)
    val count = f.sink.controllers.size
    f.key(KeyEvent.KEYCODE_BUTTON_A); check(f.sink.controllers.size == count)
    f.key(KeyEvent.KEYCODE_BUTTON_A, false)
    f.axes(GamepadState(GamepadState.UP, leftX = 123))
    check(f.sink.last().state.buttons == GamepadState.A or GamepadState.UP)
    f.key(KeyEvent.KEYCODE_DPAD_CENTER, false)
    check(f.sink.last().state.buttons == GamepadState.UP)
  }
  hardwareTest("hat and button Dpad ownership stay independent") {
    val f = HardwareFixture(); f.pad(); f.key(KeyEvent.KEYCODE_DPAD_UP)
    f.axes(GamepadState(GamepadState.UP or GamepadState.RIGHT))
    f.key(KeyEvent.KEYCODE_DPAD_UP, false)
    check(f.sink.last().state.buttons == GamepadState.UP or GamepadState.RIGHT)
    f.key(KeyEvent.KEYCODE_DPAD_LEFT); f.axes(GamepadState.ZERO)
    check(f.sink.last().state.buttons == GamepadState.LEFT)
  }
  hardwareTest("digital triggers and analog triggers merge independently") {
    val f = HardwareFixture(); f.pad(); f.axes(GamepadState(leftTrigger = 64, rightTrigger = 99))
    f.key(KeyEvent.KEYCODE_BUTTON_L2); f.key(KeyEvent.KEYCODE_BUTTON_R2)
    check(f.sink.last().state == GamepadState(leftTrigger = 255, rightTrigger = 255))
    f.key(KeyEvent.KEYCODE_BUTTON_L2, false)
    check(f.sink.last().state == GamepadState(leftTrigger = 64, rightTrigger = 255))
    f.key(KeyEvent.KEYCODE_BUTTON_R2, false)
    check(f.sink.last().state == GamepadState(leftTrigger = 64, rightTrigger = 99))
  }
  hardwareTest("physical slots zero through fifteen and full unsigned mask") {
    val f = HardwareFixture()
    (100..116).forEach { f.pad(it) }
    check(f.input.deviceIds.size == 16)
    f.key(KeyEvent.KEYCODE_BUTTON_Y, id = 115)
    check(f.sink.last(15) == ControllerOutput(15, 65535, GamepadState(GamepadState.Y)))
    val count = f.sink.controllers.size
    f.key(KeyEvent.KEYCODE_BUTTON_A, id = 116)
    check(f.sink.controllers.size == count)
  }
  hardwareTest("hot unplug clears held state, does not compact slots, and reuses the free slot") {
    val f = HardwareFixture(); f.pad(100); f.pad(101); f.pad(102)
    f.key(KeyEvent.KEYCODE_BUTTON_Y, id = 101); f.sink.clear()
    f.input.disconnect(101)
    check(f.sink.last(1) == ControllerOutput(1, 5, GamepadState.ZERO))
    check(f.sink.last(2).mask == 5)
    f.pad(103); f.key(KeyEvent.KEYCODE_BUTTON_B, id = 103)
    check(f.sink.last(1) == ControllerOutput(1, 7, GamepadState(GamepadState.B)))
  }
  hardwareTest("removing first physical retains virtual player one and second physical player two") {
    val f = HardwareFixture(); f.pad(100); f.pad(101)
    f.virtual(GamepadState(GamepadState.A)); f.key(KeyEvent.KEYCODE_BUTTON_B)
    f.key(KeyEvent.KEYCODE_BUTTON_Y, id = 101); f.input.disconnect(100)
    check(f.sink.last() == ControllerOutput(0, 3, GamepadState(GamepadState.A)))
    check(f.sink.last(1) == ControllerOutput(1, 3, GamepadState(GamepadState.Y)))
    f.input.disconnect(101); check(f.sink.last().mask == 1)
    f.pad(100); check(f.sink.last().state.buttons == GamepadState.A)
  }
  hardwareTest("suspend neutralizes without unplugging; no input events forwarded while suspended") {
    val f = HardwareFixture(); f.pad(100); f.pad(101)
    f.key(KeyEvent.KEYCODE_BUTTON_A); f.key(KeyEvent.KEYCODE_BUTTON_B, id = 101)
    f.input.mouseButtons(9, 31); f.sink.clear(); f.input.setSuspended(true)
    check(f.sink.controllers.size == 2)
    check(f.sink.controllers.all { it.mask == 3 && it.state == GamepadState.ZERO })
    check(f.sink.held.isEmpty())
    f.sink.clear(); f.key(KeyEvent.KEYCODE_BUTTON_A, false)
    f.axes(GamepadState(leftX = 1234), 101); f.virtual(GamepadState(GamepadState.X))
    f.input.absolute(2f, 3f, 100, 100); f.input.relative(9, 12f, 4f)
    f.input.mouseButtons(9, 31); f.input.scroll(9, 2f, 3f)
    check(f.sink.controllers.isEmpty() && f.sink.pointer.isEmpty())
    f.input.setSuspended(false)
    check(f.sink.last().state.buttons == GamepadState.X)
    check(f.sink.last(1).state == GamepadState(GamepadState.B, leftX = 1234))
    check(f.sink.held.isEmpty())
  }
  hardwareTest("unplug during suspension is silent and publishes reduced topology on resume") {
    val f = HardwareFixture(); f.pad(); f.pad(101); f.key(KeyEvent.KEYCODE_BUTTON_Y, id = 101)
    f.input.setSuspended(true); f.sink.clear(); f.input.disconnect(101)
    check(f.sink.controllers.isEmpty())
    f.input.setSuspended(false)
    check(f.sink.last(1) == ControllerOutput(1, 1, GamepadState.ZERO))
    check(f.sink.last() == ControllerOutput(0, 1, GamepadState.ZERO))
  }
  hardwareTest("suspended early start and announce send only neutrals, resume replays latest snapshots") {
    val f = HardwareFixture(false); f.pad(); f.key(KeyEvent.KEYCODE_BUTTON_A)
    f.input.setSuspended(true); f.input.start(); f.input.announce()
    check(f.sink.controllers.all { it.state == GamepadState.ZERO && it.mask == 1 })
    f.input.setSuspended(false); check(f.sink.last().state.buttons == GamepadState.A)
  }
  hardwareTest("release clears all ownership without changing mask or unregistering devices") {
    val f = HardwareFixture(); f.pad(); f.pad(101)
    f.key(KeyEvent.KEYCODE_BUTTON_A); f.virtual(GamepadState(GamepadState.B)); f.input.mouseButtons(9, 31)
    f.input.release(); f.input.release()
    check(f.sink.last().state == GamepadState.ZERO && f.sink.last(1).state == GamepadState.ZERO)
    check(f.sink.last().mask == 3 && f.input.deviceIds.size == 2 && f.sink.held.isEmpty())
    f.input.announce(); check(f.sink.last().state == GamepadState.ZERO)
    f.key(KeyEvent.KEYCODE_BUTTON_X); check(f.sink.last().state.buttons == GamepadState.X)
  }
  hardwareTest("mapping change resets keys and axes, stop suppresses subsequent events") {
    val f = HardwareFixture(); f.pad(); f.key(KeyEvent.KEYCODE_BUTTON_A); f.axes(GamepadState(leftX = 1000))
    f.input.resetDevice(100); check(f.sink.last().state == GamepadState.ZERO)
    f.input.release(); f.input.stop(); f.sink.clear()
    f.input.key(100, KeyEvent.KEYCODE_BUTTON_A, true); f.virtual(GamepadState(GamepadState.B))
    f.input.announce(); f.input.absolute(1f, 2f, 100, 100)
    check(f.sink.controllers.isEmpty() && f.sink.pointer.isEmpty())
  }
  hardwareTest("stick endpoints, inversion, asymmetric range and continuous MotionRange flat deadzone") {
    fun stick(v: Float, invert: Boolean = false) = HardwareInputState.stick(v, -1f, 1f, 0.2f, invert)
    check(stick(0f) == 0 && stick(0.2f) == 0 && stick(-0.2f) == 0)
    check(stick(1f) == 32767 && stick(-1f) == -32767)
    check(stick(1f, true) == -32767 && stick(-1f, true) == 32767)
    check(stick(0.6f) in 16383..16384)
    check(stick(2f) == 32767 && stick(-2f) == -32767)
    check(HardwareInputState.stick(-2f, -2f, 4f, 0f) == -32767)
    check(HardwareInputState.stick(4f, -2f, 4f, 0f) == 32767)
  }
  hardwareTest("trigger zero based and signed ranges, flat and finite hostile inputs") {
    check(HardwareInputState.trigger(0f, 0f, 1f, 0.1f) == 0)
    check(HardwareInputState.trigger(0.1f, 0f, 1f, 0.1f) == 0)
    check(HardwareInputState.trigger(1f, 0f, 1f, 0.1f) == 255)
    check(HardwareInputState.trigger(-1f, -1f, 1f, 0f) == 0)
    check(HardwareInputState.trigger(0f, -1f, 1f, 0f) == 128)
    check(HardwareInputState.trigger(1f, -1f, 1f, 0f) == 255)
    for (bad in listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY)) {
      check(HardwareInputState.stick(bad, -1f, 1f, 0f) == 0)
      check(HardwareInputState.stick(1f, bad, 1f, 0f) == 0)
      check(HardwareInputState.trigger(bad, 0f, 1f, 0f) == 0)
      check(HardwareInputState.trigger(1f, 0f, 1f, bad) == 0)
    }
    check(HardwareInputState.stick(1f, 0f, 1f, 0f) == 0)
    check(HardwareInputState.stick(1f, -1f, 1f, 2f) == 0)
    check(HardwareInputState.trigger(1f, 1f, 1f, 0f) == 0)
    check(HardwareInputState.trigger(1f, 0f, 1f, 2f) == 0)
  }
  hardwareTest("state clamping avoids integer abs overflow and preserves immutable input") {
    val hostile = GamepadState(0xFFFF, -1, 999, Int.MIN_VALUE, Int.MAX_VALUE, -99999, 99999)
    val merged = HardwareInputState.merge(hostile, GamepadState(leftX = 32767))
    check(merged == GamepadState(0xFFFF, 0, 255, 32767, 32767, -32767, 32767))
    check(hostile.leftX == Int.MIN_VALUE)
  }
  hardwareTest("absolute mouse clamps viewport-local coordinates, ignores invalid geometry") {
    val f = HardwareFixture(); f.input.absolute(-20f, 900f, 640, 360)
    check(f.sink.pointer == listOf("abs:0,359/640,360"))
    f.input.absolute(Float.NaN, 1f, 100, 100); f.input.absolute(1f, Float.POSITIVE_INFINITY, 100, 100)
    f.input.absolute(1f, 1f, 0, 10); f.input.absolute(1f, 1f, 10, -1)
    check(f.sink.pointer.size == 1)
  }
  hardwareTest("relative capture keeps subpixel residuals and rejects non-finite deltas") {
    val f = HardwareFixture()
    repeat(4) { f.input.relative(9, 0.25f, -0.25f) }
    check(f.sink.pointer == listOf("rel:1,-1"))
    f.input.relative(9, Float.NaN, Float.POSITIVE_INFINITY)
    check(f.sink.pointer.size == 1)
    f.input.relative(9, Float.MAX_VALUE, -Float.MAX_VALUE)
    check(f.sink.pointer.last() == "rel:32767,-32767")
  }
  hardwareTest("mouse buttons are balanced abstract IDs including X1 and X2") {
    val f = HardwareFixture(); f.input.mouseButtons(9, 31); f.input.mouseButtons(9, 31)
    check(f.sink.held == setOf(1, 2, 3, 4, 5)); check(f.sink.pointer.size == 5)
    f.input.mouseButtons(9, 0)
    check(f.sink.held.isEmpty() && f.sink.pointer.size == 10)
  }
  hardwareTest("two mice OR button ownership and unplug releases only the lost mouse") {
    val f = HardwareFixture(); f.input.mouseButtons(9, 1); f.input.mouseButtons(10, 3)
    check(f.sink.pointer == listOf("button:1/true", "button:2/true"))
    f.input.removeMouse(9); check(f.sink.held == setOf(1, 2))
    f.input.removeMouse(10); check(f.sink.held.isEmpty())
    check(f.sink.pointer.takeLast(2) == listOf("button:1/false", "button:2/false"))
  }
  hardwareTest("vertical/horizontal scroll preserves fractional detents at 120 units per detent") {
    val f = HardwareFixture(); f.input.scroll(9, 1f, -0.5f)
    check(f.sink.pointer == listOf("scroll:120,-60/true"))
    f.sink.clear(); repeat(4) { f.input.scroll(9, 0.003125f, -0.003125f) }
    check(f.sink.pointer == listOf("scroll:1,-1/true"))
    f.input.scroll(9, Float.NaN, Float.NEGATIVE_INFINITY); check(f.sink.pointer.size == 1)
  }
  hardwareTest("focus/capture release clears residuals and never resurrects held mouse buttons") {
    val f = HardwareFixture(); f.input.relative(9, 0.75f, 0.75f); f.input.scroll(9, 0.003125f, 0f)
    f.input.mouseButtons(9, 31); f.input.release(); f.sink.clear()
    f.input.relative(9, 0.25f, 0.25f); f.input.scroll(9, 0.003125f, 0f)
    check(f.sink.pointer.isEmpty() && f.sink.held.isEmpty())
    f.input.setSuspended(true); f.input.setSuspended(false)
    check(f.sink.held.isEmpty())
  }
  println("$hardwarePassed pure hardware input tests passed")
}
