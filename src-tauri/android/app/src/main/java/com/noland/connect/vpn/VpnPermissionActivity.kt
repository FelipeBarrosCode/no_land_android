package com.noland.connect.vpn

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

internal data class VpnPermissionResult(
  val granted: Boolean,
  val error: String? = null,
)

/** Owns Android's VPN consent dialog and reports its result to the waiting native operation. */
class VpnPermissionActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    if (savedInstanceState == null) launchPermissionRequest()
  }

  @Deprecated("Deprecated by Android; retained for compatibility with the generated Tauri activity stack.")
  override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
    super.onActivityResult(requestCode, resultCode, data)
    if (requestCode != VPN_PERMISSION_REQUEST_CODE) return

    completePendingRequest(resultCode == RESULT_OK)
    finish()
  }

  private fun launchPermissionRequest() {
    val permissionIntent = VpnService.prepare(this)
    if (permissionIntent == null) {
      completePendingRequest(true)
      finish()
      return
    }

    @Suppress("DEPRECATION")
    startActivityForResult(permissionIntent, VPN_PERMISSION_REQUEST_CODE)
  }

  companion object {
    private const val VPN_PERMISSION_REQUEST_CODE = 4107
    private const val PERMISSION_TIMEOUT_SECONDS = 120L

    private val requestLock = Any()
    private var pendingRequest: PendingPermissionRequest? = null

    internal fun request(context: Context): VpnPermissionResult {
      val request = synchronized(requestLock) {
        pendingRequest ?: PendingPermissionRequest().also { pendingRequest = it }
      }

      try {
        context.startActivity(
          Intent(context, VpnPermissionActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP)
            addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
          },
        )
      } catch (error: Throwable) {
        clearPendingRequest(request)
        return VpnPermissionResult(
          granted = false,
          error = "Android could not open the VPN permission request: ${error.message ?: error.javaClass.simpleName}",
        )
      }

      val completed = try {
        request.completed.await(PERMISSION_TIMEOUT_SECONDS, TimeUnit.SECONDS)
      } catch (_: InterruptedException) {
        Thread.currentThread().interrupt()
        clearPendingRequest(request)
        return VpnPermissionResult(
          granted = false,
          error = "The VPN permission request was interrupted.",
        )
      }

      clearPendingRequest(request)
      if (!completed) {
        return VpnPermissionResult(
          granted = false,
          error = "The VPN permission request timed out. Try connecting again.",
        )
      }
      if (!request.granted) {
        return VpnPermissionResult(
          granted = false,
          error = "VPN permission was not granted. Approve the Android connection request to continue.",
        )
      }
      return VpnPermissionResult(granted = true)
    }

    private fun completePendingRequest(granted: Boolean) {
      val request = synchronized(requestLock) { pendingRequest } ?: return
      request.granted = granted
      request.completed.countDown()
    }

    private fun clearPendingRequest(request: PendingPermissionRequest) {
      synchronized(requestLock) {
        if (pendingRequest === request) pendingRequest = null
      }
    }
  }

  private class PendingPermissionRequest {
    val completed = CountDownLatch(1)

    @Volatile
    var granted = false
  }
}
