package com.noland.connect.stream

import android.app.Activity
import android.media.MediaCodec
import android.os.Bundle
import android.view.Surface
import android.view.View
import android.view.WindowManager
import java.lang.ref.WeakReference

class StreamActivity : Activity(), StreamSurfaceView.Listener {
  private val decoder = MediaCodecVideoDecoder()
  private lateinit var streamView: StreamSurfaceView

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
    window.decorView.systemUiVisibility =
      View.SYSTEM_UI_FLAG_FULLSCREEN or
        View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or
        View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY

    streamView = StreamSurfaceView(this).also { it.listener = this }
    setContentView(streamView)
    activeActivity = WeakReference(this)
  }

  override fun onSurfaceReady(surface: Surface, width: Int, height: Int) {
    if (!NativeStreamBridge.attachSurface(surface, width, height)) {
      setResult(RESULT_CANCELED)
      finish()
    }
  }

  override fun onSurfaceSizeChanged(width: Int, height: Int) {
    NativeStreamBridge.resizeSurface(width, height)
  }

  override fun onSurfaceUnavailable() {
    decoder.release()
    NativeStreamBridge.detachSurface()
  }

  override fun onDestroy() {
    if (activeActivity?.get() === this) activeActivity = null
    decoder.release()
    NativeStreamBridge.detachSurface()
    super.onDestroy()
  }

  override fun onBackPressed() {
    NativeStreamBridge.stopStream()
    super.onBackPressed()
  }

  private fun configureDecoder(
    mimeType: String,
    width: Int,
    height: Int,
    codecSpecificData: Array<ByteArray>,
  ): Boolean = runCatching {
    decoder.configure(
      mimeType = mimeType,
      width = width,
      height = height,
      surface = streamView.holder.surface,
      codecSpecificData = codecSpecificData.asList(),
    )
  }.isSuccess

  private fun queueVideoAccessUnit(
    data: ByteArray,
    presentationTimeUs: Long,
    codecConfig: Boolean,
    endOfStream: Boolean,
  ): Boolean {
    var flags = 0
    if (codecConfig) flags = flags or MediaCodec.BUFFER_FLAG_CODEC_CONFIG
    if (endOfStream) flags = flags or MediaCodec.BUFFER_FLAG_END_OF_STREAM
    return decoder.queueAccessUnit(data, presentationTimeUs, flags)
  }

  private fun flushDecoder() {
    decoder.flush()
  }

  companion object {
    @Volatile
    private var activeActivity: WeakReference<StreamActivity>? = null

    @JvmStatic
    fun configureVideoDecoder(
      mimeType: String,
      width: Int,
      height: Int,
      codecSpecificData: Array<ByteArray>,
    ): Boolean = activeActivity?.get()?.configureDecoder(
      mimeType,
      width,
      height,
      codecSpecificData,
    ) ?: false

    @JvmStatic
    fun submitVideoAccessUnit(
      data: ByteArray,
      presentationTimeUs: Long,
      codecConfig: Boolean,
      endOfStream: Boolean,
    ): Boolean = activeActivity?.get()?.queueVideoAccessUnit(
      data,
      presentationTimeUs,
      codecConfig,
      endOfStream,
    ) ?: false

    @JvmStatic
    fun flushVideoDecoder() {
      activeActivity?.get()?.flushDecoder()
    }
  }
}
