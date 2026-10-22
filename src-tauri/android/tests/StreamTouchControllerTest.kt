package com.noland.connect.stream

import com.noland.connect.stream.StreamTouchController.Action
import com.noland.connect.stream.StreamTouchController.Pointer

private class Timeline : StreamTouchController.Clock, StreamTouchController.Scheduler {
  data class Job(val due: Long, val task: () -> Unit, var cancelled: Boolean = false, var ran: Boolean = false)
  var now = 0L
  val jobs = mutableListOf<Job>()
  override fun nowMillis() = now
  override fun schedule(delayMillis: Long, task: () -> Unit): StreamTouchController.Cancellation {
    val job = Job(now + delayMillis, task)
    jobs.add(job)
    return StreamTouchController.Cancellation { job.cancelled = true }
  }
  fun advance(millis: Long) {
    val end = now + millis
    while (true) {
      val job = jobs.filter { !it.cancelled && !it.ran && it.due <= end }.minByOrNull { it.due } ?: break
      now = job.due
      job.ran = true
      job.task()
    }
    now = end
  }
  // Simulate a callback already dequeued when its Cancellation was invoked.
  fun replayCancelled() { jobs.toList().filter { it.cancelled }.forEach { it.task() } }
}

private class RecordingSink : StreamTouchController.Sink {
  val events = mutableListOf<String>()
  val pressed = mutableSetOf<Int>()
  override fun absolute(x: Int, y: Int, width: Int, height: Int) { events.add("abs:$x,$y/$width,$height") }
  override fun relative(dx: Int, dy: Int) { events.add("rel:$dx,$dy") }
  override fun button(button: Int, pressed: Boolean) {
    check(if (pressed) this.pressed.add(button) else this.pressed.remove(button)) { "Unbalanced button $button/$pressed" }
    events.add("button:$button/$pressed")
  }
  override fun scroll(vertical: Int, horizontal: Int, highResolution: Boolean) {
    events.add("scroll:$vertical,$horizontal/$highResolution")
  }
  override fun keyboardRequested() { events.add("keyboard") }
  fun expect(vararg expected: String) {
    check(events == expected.toList()) { "Expected ${expected.toList()}, got $events" }
  }
}

private class Fixture(mode: TouchMode = TouchMode.TRACKPAD, density: Float = 1f) {
  val time = Timeline()
  val sink = RecordingSink()
  val controller = StreamTouchController(time, time, density).apply {
    setViewport(1280, 720)
    this.mode = mode
    this.sink = this@Fixture.sink
  }
  val fingers = linkedMapOf<Int, Pointer>()
  fun down(id: Int = 7, x: Float = 100f, y: Float = 100f) {
    val action = if (fingers.isEmpty()) Action.DOWN else Action.POINTER_DOWN
    fingers[id] = Pointer(id, x, y)
    controller.onTouch(action, fingers.values.toList().reversed(), id)
  }
  fun move(vararg points: Pointer) {
    points.forEach { fingers[it.id] = it }
    controller.onTouch(Action.MOVE, fingers.values.toList().reversed())
  }
  fun up(id: Int = 7, x: Float? = null, y: Float? = null) {
    val point = fingers.getValue(id)
    fingers[id] = point.copy(x = x ?: point.x, y = y ?: point.y)
    controller.onTouch(if (fingers.size == 1) Action.UP else Action.POINTER_UP, fingers.values.toList().reversed(), id)
    fingers.remove(id)
  }
  fun three(firstX: Float = 100f) {
    down(7, firstX)
    down(42, 200f)
    down(91, 300f)
  }
  fun liftThree() { up(42); up(7); up(91) }
  fun buttons() = sink.events.filter { it.startsWith("button:") }
}

private var passed = 0
private fun test(name: String, body: () -> Unit) {
  try { body(); passed++; println("PASS $name") }
  catch (error: Throwable) { throw AssertionError("FAIL $name: ${error.message}", error) }
}

fun main() {
  test("mode and abstract button values") {
    val f = Fixture()
    check(f.controller.mode == TouchMode.TRACKPAD)
    check(TouchMode.values().map { it.value } == listOf(1, 2, 3))
    check(listOf(StreamTouchController.LEFT, StreamTouchController.RIGHT, StreamTouchController.MIDDLE) == listOf(1, 2, 3))
  }
  test("trackpad tap has exactly 50ms synthetic release") {
    val f = Fixture(); f.down(); f.sink.expect(); f.up()
    f.sink.expect("button:1/true"); f.time.advance(49); f.sink.expect("button:1/true")
    f.time.advance(1); f.sink.expect("button:1/true", "button:1/false")
    f.time.advance(1000); f.sink.expect("button:1/true", "button:1/false")
  }
  test("relative movement scales independently to 1280x720") {
    val f = Fixture(); f.controller.setViewport(640, 180)
    f.down(); f.move(Pointer(7, 110f, 110f)); f.up(); f.time.advance(1000)
    f.sink.expect("rel:20,40")
  }
  test("stationary trackpad hold starts drag at exactly 650ms") {
    val f = Fixture(); f.down(); f.time.advance(649); f.sink.expect()
    f.time.advance(1); f.sink.expect("button:1/true")
    f.move(Pointer(7, 120f, 110f)); f.up(); f.time.advance(1000)
    f.sink.expect("button:1/true", "rel:20,10", "button:1/false")
  }
  test("5dp movement is inclusive and latched even after returning") {
    val f = Fixture(); f.down(); f.move(Pointer(7, 103f, 104f)); f.move(Pointer(7, 100f, 100f))
    f.time.advance(650); f.up(); f.time.replayCancelled()
    f.sink.expect("rel:3,4", "rel:-3,-4")
  }
  test("threshold follows density, not a fixed pixel distance") {
    val f = Fixture(density = 2f); f.down(); f.move(Pointer(7, 109f, 100f)); f.up(); f.time.advance(50)
    f.sink.expect("rel:9,0", "button:1/true", "button:1/false")
    val g = Fixture(density = 2f); g.down(); g.move(Pointer(7, 110f, 100f)); g.time.advance(650); g.up()
    g.sink.expect("rel:10,0")
  }
  test("final UP coordinate can disqualify a tap") {
    val f = Fixture(); f.down(); f.up(x = 106f); f.time.advance(1000); f.sink.expect()
  }
  test("click-to-use positions on down and tap, without immediate press") {
    val f = Fixture(TouchMode.CLICK_TO_USE); f.down(); f.sink.expect("abs:100,100/1280,720")
    f.up(x = 102f); f.time.advance(50)
    f.sink.expect("abs:100,100/1280,720", "abs:102,100/1280,720", "button:1/true", "button:1/false")
  }
  test("click-to-use slide only positions") {
    val f = Fixture(TouchMode.CLICK_TO_USE); f.down(); f.move(Pointer(7, 120f, 130f)); f.time.advance(700); f.up()
    f.sink.expect("abs:100,100/1280,720", "abs:120,130/1280,720")
  }
  test("click-to-use hold drags and never synthesizes a second tap") {
    val f = Fixture(TouchMode.CLICK_TO_USE); f.down(); f.time.advance(650); f.up(); f.time.advance(1000)
    f.sink.expect("abs:100,100/1280,720", "button:1/true", "button:1/false")
  }
  test("direct presses immediately and releases without synthetic delay") {
    val f = Fixture(TouchMode.DIRECT); f.down()
    f.sink.expect("abs:100,100/1280,720", "button:1/true"); f.up(); f.time.advance(1000)
    f.sink.expect("abs:100,100/1280,720", "button:1/true", "button:1/false")
  }
  test("direct stationary hold releases left before pressing right") {
    val f = Fixture(TouchMode.DIRECT); f.down(); f.time.advance(649)
    check(f.buttons() == listOf("button:1/true")); f.time.advance(1); f.up()
    check(f.buttons() == listOf("button:1/true", "button:1/false", "button:2/true", "button:2/false"))
  }
  test("direct movement prevents stationary right hold") {
    val f = Fixture(TouchMode.DIRECT); f.down(); f.move(Pointer(7, 105f, 100f)); f.time.advance(650); f.up()
    f.sink.expect("abs:100,100/1280,720", "button:1/true", "abs:105,100/1280,720", "button:1/false")
  }
  for (mode in TouchMode.values()) {
    test("two-finger centroid scroll in $mode; no one-finger resumption") {
      val f = Fixture(mode); f.down(); f.down(42, 200f, 200f)
      f.move(Pointer(7, 100f, 106f), Pointer(42, 200f, 206f))
      check(f.sink.events.last() == "scroll:60,0/true")
      val before = f.sink.events.toList()
      f.up(7); f.move(Pointer(42, 500f, 500f)); f.time.advance(650); f.up(42); f.time.advance(50)
      check(f.sink.events == before)
      check(f.sink.pressed.isEmpty())
    }
    test("two-finger tap right only outside direct mode: $mode") {
      val f = Fixture(mode); f.down(); f.down(42, 200f); f.up(7)
      val before = f.sink.events.toList(); f.up(42)
      if (mode == TouchMode.DIRECT) check(f.sink.events == before)
      else check(f.sink.events.last() == "button:2/true")
      f.time.advance(50)
      check(f.sink.pressed.isEmpty())
      check(f.buttons().count { it == "button:2/true" } == if (mode == TouchMode.DIRECT) 0 else 1)
    }
    test("three-finger tap cancels pointers and requests keyboard only after final up: $mode") {
      val f = Fixture(mode); f.three(); check("keyboard" !in f.sink.events)
      f.up(42); f.up(7); check("keyboard" !in f.sink.events); f.up(91)
      check(f.sink.events.last() == "keyboard"); check(f.sink.pressed.isEmpty())
      check(f.buttons().none { it == "button:2/true" })
      val before = f.sink.events.toList(); f.time.advance(2000); f.time.replayCancelled()
      check(f.sink.events == before)
    }
  }
  test("pinch with stationary centroid is not a scroll gesture") {
    val f = Fixture(); f.down(); f.down(42, 200f)
    f.move(Pointer(7, 110f, 100f), Pointer(42, 190f, 100f)); f.up(42); f.up(); f.time.advance(50)
    f.sink.expect("scroll:0,0/true", "button:2/true", "button:2/false")
  }
  test("all three fingers must start outside edge, including equality") {
    for (x in listOf(0f, 44f)) {
      val f = Fixture(); f.three(x); f.liftThree(); f.time.advance(1000); f.sink.expect()
    }
    val f = Fixture(); f.controller.keyboardLeftEdgePx = 120f; f.three(100f); f.liftThree(); f.sink.expect()
  }
  test("three-finger movement checks each pointer, not just centroid") {
    val f = Fixture(); f.three()
    f.move(Pointer(7, 110f, 100f), Pointer(42, 190f, 100f)); f.liftThree(); f.sink.expect()
  }
  test("three-finger long hold and four-finger sequence do not request keyboard") {
    val f = Fixture(); f.three(); f.time.advance(650); f.liftThree(); f.sink.expect()
    val g = Fixture(); g.three(); g.down(123, 400f); g.up(123); g.liftThree(); g.sink.expect()
  }
  test("empty virtual pad surface permits keyboard but suppresses all mouse") {
    val f = Fixture(TouchMode.DIRECT); f.controller.inputSuspended = true
    f.down(); f.time.advance(1000); f.move(Pointer(7, 500f, 500f)); f.up(); f.sink.expect()
    f.three(); f.liftThree(); f.sink.expect("keyboard")
  }
  test("menu/keyboard and root-owned recognition disable keyboard callback") {
    val f = Fixture(); f.controller.inputSuspended = true; f.controller.keyboardGesturesEnabled = false
    f.three(); f.liftThree(); f.sink.expect()
  }
  test("cancel after hold releases once and ignores tails and cancelled callbacks") {
    for (mode in TouchMode.values()) {
      val f = Fixture(mode); f.down(); f.time.advance(650); f.controller.cancel()
      val before = f.sink.events.toList(); check(f.sink.pressed.isEmpty())
      f.move(Pointer(7, 120f, 120f)); f.up(); f.time.advance(1000); f.time.replayCancelled(); f.controller.release()
      check(f.sink.events == before)
    }
  }
  test("ACTION_CANCEL before deadline never turns into hold or click") {
    val f = Fixture(); f.down(); f.controller.onTouch(Action.CANCEL, emptyList()); f.time.advance(1000)
    f.up(); f.time.replayCancelled(); f.sink.expect()
  }
  test("pending click drains synchronously to its old sink") {
    val f = Fixture(); f.down(); f.up(); val next = RecordingSink(); f.controller.sink = next
    f.sink.expect("button:1/true", "button:1/false")
    f.time.advance(1000); f.time.replayCancelled(); next.expect()
  }
  test("old hold cannot press into replacement session") {
    val f = Fixture(TouchMode.DIRECT); f.down(); val next = RecordingSink(); f.controller.sink = next
    f.time.advance(1000); f.time.replayCancelled(); f.up()
    f.sink.expect("abs:100,100/1280,720", "button:1/true", "button:1/false"); next.expect()
  }
  test("new physical sequence drains pending tap; old callback cannot release new drag") {
    val f = Fixture(); f.down(); f.up(); f.down(); f.time.advance(650)
    f.sink.expect("button:1/true", "button:1/false", "button:1/true")
    f.time.replayCancelled(); check(f.sink.pressed == setOf(1)); f.up(); check(f.sink.pressed.isEmpty())
  }
  test("mode, suspension, geometry and edge changes all cancel the sequence") {
    val changes: List<(StreamTouchController) -> Unit> = listOf(
      { it.mode = TouchMode.TRACKPAD }, { it.inputSuspended = true },
      { it.keyboardGesturesEnabled = false }, { it.setViewport(640, 360) }, { it.keyboardLeftEdgePx = 60f },
    )
    for (change in changes) {
      val f = Fixture(TouchMode.DIRECT); f.down(); change(f.controller)
      val before = f.sink.events.toList(); f.up(); f.time.advance(1000); f.time.replayCancelled()
      check(f.sink.events == before); check(f.sink.pressed.isEmpty())
    }
  }
  test("idempotent configuration does not cancel an active hold") {
    val f = Fixture(); f.down(); f.controller.sink = f.sink; f.controller.mode = TouchMode.TRACKPAD
    f.controller.inputSuspended = false; f.controller.setViewport(1280, 720); f.time.advance(650)
    f.sink.expect("button:1/true"); f.up()
  }
  test("absolute coordinates are aspect-fit and clamped") {
    val f = Fixture(TouchMode.DIRECT); f.controller.setViewport(1000, 1000, 1920, 1080)
    f.down(x = -20f, y = 100f); f.move(Pointer(7, 1200f, 1000f)); f.up()
    f.sink.expect("abs:0,0/1000,563", "button:1/true", "abs:1000,563/1000,563", "button:1/false")
    val g = Fixture(TouchMode.DIRECT); g.controller.setViewport(1000, 500, 1000, 1000); g.down(x = 100f)
    g.sink.expect("abs:0,100/500,500", "button:1/true"); g.controller.cancel()
  }
  test("invalid geometry emits no mouse input") {
    val f = Fixture(TouchMode.DIRECT); f.controller.setViewport(0, 0); f.down(); f.time.advance(1000); f.up(); f.sink.expect()
  }
  test("malformed pointer snapshots cancel safely") {
    for (snapshot in listOf(emptyList(), listOf(Pointer(99, 100f, 100f)),
      listOf(Pointer(7, 100f, 100f), Pointer(7, 100f, 100f)), listOf(Pointer(7, Float.NaN, 100f)))) {
      val f = Fixture(TouchMode.DIRECT); f.down(); f.controller.onTouch(Action.MOVE, snapshot)
      check(f.sink.pressed.isEmpty()); f.up(); f.time.advance(1000)
      check(f.buttons() == listOf("button:1/true", "button:1/false"))
    }
  }
  println("$passed gesture regression tests passed")
}
