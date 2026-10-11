package com.noland.connect.stream

import android.media.MediaCodec
import android.media.MediaFormat
import android.view.Surface
import java.nio.ByteBuffer

/** Surface-backed hardware decoder. Encoded access units remain outside the WebView. */
internal class MediaCodecVideoDecoder {
  private var codec: MediaCodec? = null

  @Synchronized
  fun configure(
    mimeType: String,
    width: Int,
    height: Int,
    surface: Surface,
    codecSpecificData: List<ByteArray>,
  ) {
    release()

    val format = MediaFormat.createVideoFormat(mimeType, width, height).apply {
      codecSpecificData.forEachIndexed { index, bytes ->
        setByteBuffer("csd-$index", ByteBuffer.wrap(bytes))
      }
      setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
    }

    codec = MediaCodec.createDecoderByType(mimeType).also { decoder ->
      decoder.configure(format, surface, null, 0)
      decoder.start()
    }
  }

  @Synchronized
  fun queueAccessUnit(data: ByteArray, presentationTimeUs: Long, flags: Int): Boolean {
    val decoder = codec ?: return false
    val inputIndex = decoder.dequeueInputBuffer(0)
    if (inputIndex < 0) return false

    val input = decoder.getInputBuffer(inputIndex) ?: return false
    input.clear()
    if (input.remaining() < data.size) return false
    input.put(data)
    decoder.queueInputBuffer(inputIndex, 0, data.size, presentationTimeUs, flags)
    drainOutput(decoder)
    return true
  }

  @Synchronized
  fun flush() {
    codec?.flush()
  }

  @Synchronized
  fun release() {
    val decoder = codec ?: return
    codec = null
    runCatching { decoder.stop() }
    decoder.release()
  }

  private fun drainOutput(decoder: MediaCodec) {
    val info = MediaCodec.BufferInfo()
    while (true) {
      val outputIndex = decoder.dequeueOutputBuffer(info, 0)
      if (outputIndex < 0) return
      decoder.releaseOutputBuffer(outputIndex, true)
    }
  }
}
