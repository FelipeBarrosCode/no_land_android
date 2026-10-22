package com.noland.connect.stream

private var assertions = 0

private fun expect(expected: Any?, actual: Any?) {
  check(expected == actual) { "Expected <$expected>, got <$actual>" }
  assertions++
}

fun main() {
  val commits = StreamKeyboardCommits()
  fun change(backspaces: Int, text: String, next: String, before: Int = 0) =
    expect(StreamKeyboardCommits.Change(backspaces, text), commits.update(next, false, before))

  // Repeated preedit never sends, even when deletion removes the composing span.
  expect(null, commits.update("n", true))
  expect(null, commits.update("ni", true))
  expect(null, commits.update("", true))
  change(0, "你好", "你好")
  change(0, "", "你好") // finishComposingText after commitText is not a second commit.
  change(0, " 🌍", "你好 🌍")
  change(1, "", "你好 ") // Supplementary character is one Windows backspace.

  // An IME can mark existing committed text as composing and replace it.
  expect(null, commits.update("你好 world", true))
  change(0, "world", "你好 world")
  expect(null, commits.update("你好 world", true))
  change(0, "", "你好 world")
  expect(null, commits.update("你好 word", true))
  change(2, "d", "你好 word")

  // A shared high surrogate is not a shared code point.
  commits.clear()
  change(0, "😀", "😀")
  change(1, "😁", "😁")
  change(0, "😀", "😁😀") // Intentional repeated commits are still delivered.
  change(0, "", "😁😀")
  change(2, "", "")
  change(1, "", "", 1) // No local context: still delete on the remote host.

  commits.clear()
  change(0, "abc", "abc")
  change(4, "bc", "bc", 1) // Delete beyond retained context before restoring the tail.
  commits.clear() // Closing/replacing an editor never flushes canceled preedit.
  expect(null, commits.update("unfinished", true))
  commits.clear()
  change(0, "new session", "new session")

  fun deletion(text: String, start: Int, end: Int, before: Int, after: Int, points: Boolean,
               expected: StreamKeyboardCommits.Deletion) =
    expect(expected, StreamKeyboardCommits.deletion(text, start, end, before, after, points))

  val sample = "A😀B🌍C"
  // UTF-16 requests that intersect either half of a surrogate remove the whole pair.
  deletion(sample, 3, 3, 1, 0, false, StreamKeyboardCommits.Deletion(1, 3, 0, 0))
  deletion(sample, 1, 1, 0, 1, false, StreamKeyboardCommits.Deletion(1, 3, 0, 0))
  deletion(sample, 6, 6, 1, 0, true, StreamKeyboardCommits.Deletion(4, 6, 0, 0))
  deletion(sample, 1, 1, 0, 2, true, StreamKeyboardCommits.Deletion(1, 4, 0, 0))
  deletion(sample, 3, 4, 1, 1, true, StreamKeyboardCommits.Deletion(1, 6, 0, 0))
  deletion(sample, 3, 4, 0, 0, false, StreamKeyboardCommits.Deletion(3, 4, 0, 0))
  deletion("😀", 2, 2, 3, 2, false, StreamKeyboardCommits.Deletion(0, 2, 1, 2))
  deletion("😀", 2, 2, 3, 2, true, StreamKeyboardCommits.Deletion(0, 2, 2, 2))
  deletion("", 0, 0, 1, 0, true, StreamKeyboardCommits.Deletion(0, 0, 1, 0))
  expect(1, StreamKeyboardCommits.floorBoundary(sample, 2))
  expect(3, StreamKeyboardCommits.ceilBoundary(sample, 2))
  expect(4, StreamKeyboardCommits.floorBoundary(sample, 5))
  expect(6, StreamKeyboardCommits.ceilBoundary(sample, 5))
  expect(0, StreamKeyboardCommits.floorBoundary(sample, -1))
  expect(sample.length, StreamKeyboardCommits.ceilBoundary(sample, Int.MAX_VALUE))

  // Replay every generated change into a code-point host model, including edits at
  // the beginning/middle of its retained tail. No surrogate fragment reaches onText.
  val alphabet = listOf("", "a", "é", "中", "😀", "😁", "e\u0301", "\uD801\uDC37")
  for (left in alphabet) for (right in alphabet) {
    val tracker = StreamKeyboardCommits()
    val old = "prefix" + left + right
    val next = "prefix" + right + left
    tracker.update(old, false)
    expect(null, tracker.update("preedit", true))
    val patch = checkNotNull(tracker.update(next, false))
    val kept = old.offsetByCodePoints(old.length, -patch.backspaces)
    expect(next, old.substring(0, kept) + patch.text)
    expect(StreamKeyboardCommits.Change(0, ""), tracker.update(next, false))
  }
  println("StreamKeyboardCommits: $assertions assertions passed")
}
