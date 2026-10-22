package com.noland.connect.stream

import android.content.Context
import android.graphics.Rect
import android.os.Build
import android.text.Editable
import android.text.InputType
import android.text.Selection
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager

/**
 * Offscreen IME target, not a video/touch target. Add as a VISIBLE 1px child before
 * calling showKeyboard(), exclusively from the drawer/explicit keyboard gesture.
 * All methods and callbacks belong on the UI thread. No hardware mapping lives here:
 * Activity.dispatchKeyEvent must consume mapped physical keys before normal dispatch.
 *
 * The parent owns the 44dp Done toolbar, overlay suppression, and actual IME visibility
 * (WindowInsets.Type.ime() on API 30+, its existing inset fallback on API 24–29).
 * isRequested is authorization, NOT visibility. After a visible -> hidden transition,
 * call hideKeyboard(); do not treat an initial hidden inset during opening as dismissal.
 * Parent Back callbacks should call handleBack() before leaving the stream.
 *
 * Explicit finishComposingText commits preedit once. Dismissal, focus loss, connection
 * replacement/closure and detach discard unfinished preedit; already committed text
 * has been delivered synchronously, even inside an IME batch. No delayed text can leak
 * into a later stream/session. No listener is installed on the parent's insets.
 */
class StreamKeyboardView(context: Context) : View(context) {
  var onText: ((String) -> Unit)? = null
  /** Windows VK, pressed, modifiers (Shift=1, Ctrl=2, Alt=4, Meta=8). */
  var onKey: ((Int, Boolean, Int) -> Unit)? = null
  var onDismissed: (() -> Unit)? = null

  var isRequested: Boolean = false
    private set

  private val inputMethod get() = context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
  private var connection: EditorConnection? = null
  private var backDown = false
  private val showIme = Runnable {
    if (isRequested && isAttachedToWindow && hasWindowFocus() && hasFocus()) {
      if (Build.VERSION.SDK_INT >= 30) {
        windowInsetsController?.show(WindowInsets.Type.ime())
      } else {
        inputMethod.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
      }
    }
  }

  init {
    isFocusable = false
    isFocusableInTouchMode = false
    isClickable = false
    isLongClickable = false
    importantForAccessibility = IMPORTANT_FOR_ACCESSIBILITY_NO
    if (Build.VERSION.SDK_INT >= 26) importantForAutofill = IMPORTANT_FOR_AUTOFILL_NO
    // Keep the target attached/visible to the IME without obscuring or handling video.
    translationX = -10000f
    setWillNotDraw(true)
  }

  fun showKeyboard() {
    if (isRequested) return
    isRequested = true
    isFocusableInTouchMode = true
    isFocusable = true
    if (!requestFocus()) {
      hideKeyboard()
      return
    }
    if (isAttachedToWindow) {
      inputMethod.restartInput(this)
      post(showIme)
    }
  }

  fun hideKeyboard() {
    val notify = isRequested
    isRequested = false
    removeCallbacks(showIme)
    connection?.dispose()
    connection = null
    if (notify) {
      if (Build.VERSION.SDK_INT >= 30) windowInsetsController?.hide(WindowInsets.Type.ime())
      windowToken?.let { inputMethod.hideSoftInputFromWindow(it, 0) }
    }
    clearFocus()
    isFocusableInTouchMode = false
    isFocusable = false
    if (notify) onDismissed?.invoke()
  }

  fun handleBack(): Boolean {
    if (!isRequested) return false
    hideKeyboard()
    return true
  }

  override fun onCheckIsTextEditor(): Boolean = isRequested

  override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
    if (!isRequested || !hasFocus()) return null
    connection?.dispose()
    outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    outAttrs.imeOptions = EditorInfo.IME_ACTION_NONE or EditorInfo.IME_FLAG_NO_EXTRACT_UI or
      EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_ENTER_ACTION
    if (Build.VERSION.SDK_INT >= 26) {
      outAttrs.imeOptions = outAttrs.imeOptions or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
    }
    outAttrs.initialSelStart = 0
    outAttrs.initialSelEnd = 0
    outAttrs.initialCapsMode = 0
    return EditorConnection().also { connection = it }
  }

  override fun onTouchEvent(event: MotionEvent): Boolean = false

  override fun onKeyPreIme(keyCode: Int, event: KeyEvent): Boolean {
    if (keyCode == KeyEvent.KEYCODE_BACK) {
      if (event.action == KeyEvent.ACTION_DOWN && isRequested) {
        backDown = true
        return true
      }
      if (event.action == KeyEvent.ACTION_UP && backDown) {
        backDown = false
        if (!event.isCanceled) handleBack()
        return true
      }
    }
    return super.onKeyPreIme(keyCode, event)
  }

  override fun onFocusChanged(gainFocus: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
    super.onFocusChanged(gainFocus, direction, previouslyFocusedRect)
    if (!gainFocus) hideKeyboard()
  }

  override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
    super.onWindowFocusChanged(hasWindowFocus)
    if (!hasWindowFocus) hideKeyboard()
    else if (isRequested) post(showIme)
  }

  override fun onAttachedToWindow() {
    super.onAttachedToWindow()
    if (isRequested) {
      requestFocus()
      inputMethod.restartInput(this)
      post(showIme)
    }
  }

  override fun onDetachedFromWindow() {
    hideKeyboard()
    backDown = false
    super.onDetachedFromWindow()
  }

  private inner class EditorConnection : BaseInputConnection(this@StreamKeyboardView, true) {
    private val text = Editable.Factory.getInstance().newEditable("")
    private val commits = StreamKeyboardCommits()
    private var closed = false
    private var composing = false
    private var batchDepth = 0
    private var extraBackspaces = 0
    private var extraDeletes = 0
    private val live get() = !closed && isRequested && hasFocus() && connection === this

    init { Selection.setSelection(text, 0) }

    override fun getEditable(): Editable = text
    override fun getCursorCapsMode(reqModes: Int): Int = 0

    override fun beginBatchEdit(): Boolean {
      if (!live) return false
      batchDepth++
      return true
    }

    override fun endBatchEdit(): Boolean {
      if (!live || batchDepth == 0) return false
      batchDepth--
      updateSelection()
      return true
    }

    override fun setComposingText(value: CharSequence?, newCursorPosition: Int): Boolean {
      if (!live) return false
      composing = true
      super.setComposingText(normalize(value), newCursorPosition)
      updateSelection()
      return true
    }

    override fun setComposingRegion(start: Int, end: Int): Boolean {
      if (!live || start !in 0..text.length || end !in 0..text.length) return false
      composing = true
      super.setComposingRegion(
        StreamKeyboardCommits.floorBoundary(text, minOf(start, end)),
        StreamKeyboardCommits.ceilBoundary(text, maxOf(start, end)),
      )
      updateSelection()
      return true
    }

    override fun commitText(value: CharSequence?, newCursorPosition: Int): Boolean {
      if (!live) return false
      super.commitText(normalize(value), newCursorPosition)
      composing = false
      publish()
      return true
    }

    override fun finishComposingText(): Boolean {
      if (!live) return false
      super.finishComposingText()
      composing = false
      publish()
      return true
    }

    override fun setSelection(start: Int, end: Int): Boolean {
      if (!live || start !in 0..text.length || end !in 0..text.length) return false
      Selection.setSelection(text, StreamKeyboardCommits.floorBoundary(text, start),
        StreamKeyboardCommits.floorBoundary(text, end))
      updateSelection()
      return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean =
      delete(beforeLength, afterLength, false)

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean =
      delete(beforeLength, afterLength, true)

    private fun delete(before: Int, after: Int, codePoints: Boolean): Boolean {
      if (!live || before < 0 || after < 0) return false
      val start = minOf(Selection.getSelectionStart(text), Selection.getSelectionEnd(text)).coerceAtLeast(0)
      val end = maxOf(Selection.getSelectionStart(text), Selection.getSelectionEnd(text)).coerceAtLeast(0)
      val range = StreamKeyboardCommits.deletion(text.toString(), start, end, before, after, codePoints)
      // Delete in reverse order so neither selection nor composing spans get split.
      text.delete(end, range.end)
      text.delete(range.start, start)
      extraBackspaces += range.beforeContext
      extraDeletes += range.afterContext
      if (!composing) publish() else updateSelection()
      return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
      if (!live) return false
      val key = when (actionCode) {
        EditorInfo.IME_ACTION_NEXT, EditorInfo.IME_ACTION_PREVIOUS -> 0x09
        EditorInfo.IME_ACTION_UNSPECIFIED, EditorInfo.IME_ACTION_NONE, EditorInfo.IME_ACTION_DONE,
        EditorInfo.IME_ACTION_GO, EditorInfo.IME_ACTION_SEARCH, EditorInfo.IME_ACTION_SEND -> 0x0D
        else -> return false
      }
      finishComposingText()
      tap(key, if (actionCode == EditorInfo.IME_ACTION_PREVIOUS) 1 else 0)
      resetContext()
      updateSelection()
      return true
    }

    @Suppress("DEPRECATION") // Legacy IMEs on supported API 24+ can still send ACTION_MULTIPLE.
    override fun sendKeyEvent(event: KeyEvent): Boolean {
      if (!live) return false
      // Never redispatch through the Activity, or mirror physical keys it already sent.
      if (event.deviceId != KeyCharacterMap.VIRTUAL_KEYBOARD &&
        event.flags and KeyEvent.FLAG_SOFT_KEYBOARD == 0) return false
      if (event.keyCode == KeyEvent.KEYCODE_BACK) return onKeyPreIme(event.keyCode, event)
      if (event.action == KeyEvent.ACTION_MULTIPLE) {
        return event.characters?.let { commitText(it, 1) } ?: false
      }
      if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return false
      val key = when (event.keyCode) {
        KeyEvent.KEYCODE_DEL -> 0x08
        KeyEvent.KEYCODE_FORWARD_DEL -> 0x2E
        KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> 0x0D
        KeyEvent.KEYCODE_TAB -> 0x09
        else -> null
      }
      if (key != null) {
        if (event.action == KeyEvent.ACTION_DOWN) {
          when (key) {
            0x08 -> deleteSurroundingTextInCodePoints(1, 0)
            0x2E -> deleteSurroundingTextInCodePoints(0, 1)
            else -> {
              finishComposingText()
              val modifiers = (if (event.isShiftPressed) 1 else 0) or
                (if (event.isCtrlPressed) 2 else 0) or (if (event.isAltPressed) 4 else 0) or
                (if (event.isMetaPressed) 8 else 0)
              tap(key, modifiers)
              resetContext()
              updateSelection()
            }
          }
        }
        return true
      }
      val unicode = event.unicodeChar
      if (unicode <= 0 || unicode and KeyCharacterMap.COMBINING_ACCENT != 0) return false
      if (event.action == KeyEvent.ACTION_DOWN) commitText(String(Character.toChars(unicode)), 1)
      return true
    }

    private fun publish() {
      val change = commits.update(text.toString(), composing, extraBackspaces) ?: return
      val deletes = extraDeletes
      extraBackspaces = 0
      extraDeletes = 0
      repeat(change.backspaces) { tap(0x08) }
      repeat(deletes) { tap(0x2E) }
      // IMEs sometimes send Enter/Tab in commitText instead of as keys/actions.
      var from = 0
      for (index in change.text.indices) {
        val char = change.text[index]
        if (char == '\n' || char == '\t') {
          if (live && index > from) onText?.invoke(change.text.substring(from, index))
          tap(if (char == '\t') 0x09 else 0x0D)
          from = index + 1
        }
      }
      if (live && from < change.text.length) onText?.invoke(change.text.substring(from))
      // A host Enter/Tab may submit or change fields: never rewrite across that boundary.
      if (from > 0) resetContext()
      updateSelection()
    }

    private fun tap(key: Int, modifiers: Int = 0) {
      if (!live) return
      val sink = onKey ?: return
      try {
        sink(key, true, modifiers)
      } finally {
        // Pair with the original sink even if DOWN dismisses/rebinds the editor.
        sink(key, false, modifiers)
      }
    }

    private fun updateSelection() {
      if (!live) return
      val start = StreamKeyboardCommits.floorBoundary(text, Selection.getSelectionStart(text).coerceAtLeast(0))
      val end = StreamKeyboardCommits.floorBoundary(text, Selection.getSelectionEnd(text).coerceAtLeast(0))
      Selection.setSelection(text, start, end)
      if (batchDepth != 0) return
      inputMethod.updateSelection(this@StreamKeyboardView, start, end,
        getComposingSpanStart(text), getComposingSpanEnd(text))
    }

    private fun resetContext() {
      removeComposingSpans(text)
      text.clear()
      Selection.setSelection(text, 0)
      composing = false
      extraBackspaces = 0
      extraDeletes = 0
      commits.clear()
    }

    fun dispose() {
      closed = true
      resetContext()
      batchDepth = 0
    }

    override fun closeConnection() {
      dispose()
      if (connection === this) connection = null
      super.closeConnection()
    }

    private fun normalize(value: CharSequence?): String =
      value?.toString()?.replace("\r\n", "\n")?.replace('\r', '\n') ?: ""
  }
}

/** Pure UTF-16/remote-tail bookkeeping; composing updates must never reach the wire. */
internal class StreamKeyboardCommits {
  data class Change(val backspaces: Int, val text: String)
  data class Deletion(val start: Int, val end: Int, val beforeContext: Int, val afterContext: Int)

  private var committed = ""

  fun update(text: String, composing: Boolean, beforeContext: Int = 0): Change? {
    if (composing) return null
    var prefix = 0
    if (beforeContext == 0) {
      while (prefix < committed.length && prefix < text.length && committed[prefix] == text[prefix]) prefix++
      prefix = minOf(floorBoundary(committed, prefix), floorBoundary(text, prefix))
    }
    val change = Change(committed.codePointCount(prefix, committed.length) + beforeContext, text.substring(prefix))
    committed = text
    return change
  }

  fun clear() { committed = "" }

  companion object {
    fun floorBoundary(text: CharSequence, index: Int): Int {
      val at = index.coerceIn(0, text.length)
      return if (at > 0 && at < text.length && Character.isHighSurrogate(text[at - 1]) &&
        Character.isLowSurrogate(text[at])) at - 1 else at
    }

    fun ceilBoundary(text: CharSequence, index: Int): Int {
      val at = index.coerceIn(0, text.length)
      return if (floorBoundary(text, at) != at) at + 1 else at
    }

    fun deletion(text: String, start: Int, end: Int, before: Int, after: Int, codePoints: Boolean): Deletion {
      if (codePoints) {
        val availableBefore = text.codePointCount(0, start)
        val availableAfter = text.codePointCount(end, text.length)
        return Deletion(text.offsetByCodePoints(start, -minOf(before, availableBefore)),
          text.offsetByCodePoints(end, minOf(after, availableAfter)),
          (before - availableBefore).coerceAtLeast(0), (after - availableAfter).coerceAtLeast(0))
      }
      return Deletion(floorBoundary(text, start - minOf(before, start)),
        ceilBoundary(text, end + minOf(after, text.length - end)),
        (before - start).coerceAtLeast(0), (after - (text.length - end)).coerceAtLeast(0))
    }
  }
}
