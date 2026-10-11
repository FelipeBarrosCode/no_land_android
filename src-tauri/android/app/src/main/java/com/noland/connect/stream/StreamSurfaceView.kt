package com.noland.connect.stream

import android.content.Context
import android.util.AttributeSet
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView

internal class StreamSurfaceView @JvmOverloads constructor(
  context: Context,
  attrs: AttributeSet? = null,
) : SurfaceView(context, attrs), SurfaceHolder.Callback {
  interface Listener {
    fun onSurfaceReady(surface: Surface, width: Int, height: Int)
    fun onSurfaceSizeChanged(width: Int, height: Int)
    fun onSurfaceUnavailable()
  }

  var listener: Listener? = null

  init {
    holder.addCallback(this)
    holder.setKeepScreenOn(true)
    isFocusable = true
    isFocusableInTouchMode = true
  }

  override fun surfaceCreated(holder: SurfaceHolder) {
    requestFocus()
    val frame = holder.surfaceFrame
    listener?.onSurfaceReady(holder.surface, frame.width(), frame.height())
  }

  override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
    listener?.onSurfaceSizeChanged(width, height)
  }

  override fun surfaceDestroyed(holder: SurfaceHolder) {
    listener?.onSurfaceUnavailable()
  }
}
