package com.noland.connect.feedback

import android.app.Activity
import android.app.Application
import android.content.Context
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import androidx.annotation.Keep
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Keystore holds a non-exportable AES key, never bearer strings. No file-key fallback. */
@Keep
internal object FeedbackNativeBridge {
  private const val ALIAS = "com.noland.connect.feedback.aes-gcm.v1"
  private const val FORMAT: Byte = 1
  private var installed = false
  private val main = Handler(Looper.getMainLooper())
  private var resumed = 0

  private external fun nativeLifecycle(foreground: Boolean)

  private fun file(context: Context) = AtomicFile(File(context.noBackupFilesDir, "noland-feedback-v1/credentials.enc"))
  private fun accountFile(context: Context, service: String, account: String): AtomicFile {
    check(service.startsWith("noland-connect.state-secrets.v1.") && service.length <= 256)
    check(account.length in 1..256)
    val digest = java.security.MessageDigest.getInstance("SHA-256")
      .digest("$service\u0000$account".toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it.toInt() and 255) }
    return AtomicFile(File(context.noBackupFilesDir, "noland-state-keystore-v1/$digest.enc"))
  }
  private fun store() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
  private fun aad(context: Context, binding: String = "noland-feedback-v1") = "${context.packageName}:$binding".toByteArray(Charsets.UTF_8)
  private fun key(existingBlob: Boolean, alias: String = ALIAS): SecretKey {
    val keystore = store()
    (keystore.getKey(alias, null) as? SecretKey)?.let { return it }
    check(!existingBlob) { "The feedback encryption key is missing" }
    return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
      init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setKeySize(256)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setRandomizedEncryptionRequired(true)
        .setUserAuthenticationRequired(false)
        .build())
      generateKey()
    }
  }
  private fun decrypt(context: Context, bytes: ByteArray, binding: String = "noland-feedback-v1", alias: String = ALIAS): String {
    check(bytes.size in 29..2097181 && bytes[0] == FORMAT) { "Invalid protected blob" }
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    cipher.init(Cipher.DECRYPT_MODE, key(true, alias), GCMParameterSpec(128, bytes.copyOfRange(1, 13)))
    cipher.updateAAD(aad(context, binding))
    return cipher.doFinal(bytes.copyOfRange(13, bytes.size)).toString(Charsets.UTF_8)
  }

  private fun write(context: Context, target: AtomicFile, plaintext: String,
                    binding: String = "noland-feedback-v1", alias: String = ALIAS) {
    val bytes = plaintext.toByteArray(Charsets.UTF_8)
    check(bytes.size <= 2097152) { "Protected value exceeds bounds" }
    val cipher = Cipher.getInstance("AES/GCM/NoPadding")
    val anyExistingBlob = target.exists() || target.baseFile.parentFile?.listFiles()?.any { it.name.endsWith(".enc") } == true
    cipher.init(Cipher.ENCRYPT_MODE, key(anyExistingBlob, alias))
    cipher.updateAAD(aad(context, binding))
    check(cipher.iv.size == 12) { "Invalid Keystore nonce length" }
    val encrypted = byteArrayOf(FORMAT) + cipher.iv + cipher.doFinal(bytes)
    check(target.baseFile.parentFile!!.mkdirs() || target.baseFile.parentFile!!.isDirectory)
    val stream = target.startWrite()
    try {
      stream.write(encrypted)
      stream.fd.sync()
      target.finishWrite(stream)
      val parent = android.system.Os.open(target.baseFile.parentFile!!.absolutePath,
        android.system.OsConstants.O_RDONLY or android.system.OsConstants.O_DIRECTORY, 0)
      try { android.system.Os.fsync(parent) } finally { android.system.Os.close(parent) }
    } catch (error: Throwable) {
      target.failWrite(stream)
      throw error
    }
    check(decrypt(context, target.readFully(), binding, alias) == plaintext) { "Protected write verification failed" }
  }

  @JvmStatic
  @Synchronized
  fun call(context: Context, encoded: String): String = try {
    val request = JSONObject(encoded)
    val target = file(context)
    val response = JSONObject().put("ok", true)
    when (request.getString("action")) {
      "probe" -> {
        // No identity, files, or key are created by local status on a fresh installation.
        store()
        Cipher.getInstance("AES/GCM/NoPadding")
        if (target.exists()) decrypt(context, target.readFully())
        check(installed) { "Feedback lifecycle callbacks are not installed" }
      }
      "read" -> if (target.exists()) response.put("value", decrypt(context, target.readFully()))
      "write" -> {
        val plaintext = request.getString("value")
        check(plaintext.toByteArray(Charsets.UTF_8).size <= 48000) { "Feedback credentials exceed bounds" }
        write(context, target, plaintext)
      }
      "stateRead", "stateWrite", "stateDelete" -> {
        val service = request.getString("service")
        val account = request.getString("account")
        val accountTarget = accountFile(context, service, account)
        val binding = "$service\u0000$account"
        val alias = "${context.packageName}.state-secrets.aes-gcm.v1"
        when (request.getString("action")) {
          "stateRead" -> if (accountTarget.exists()) response.put("value", decrypt(context, accountTarget.readFully(), binding, alias))
          "stateWrite" -> write(context, accountTarget, request.getString("value"), binding, alias)
          "stateDelete" -> accountTarget.delete()
        }
      }
      "installLifecycle" -> install(context.applicationContext as Application)
      else -> error("Unsupported feedback operation")
    }
    response.toString()
  } catch (_: Throwable) {
    // Never echo plaintext, request JSON, tokens, or exception details to logs/WebView.
    JSONObject().put("ok", false).put("error", "Android Keystore feedback storage is unavailable; enrollment and delivery are disabled").toString()
  }

  private fun install(application: Application) {
    if (installed) return
    application.registerActivityLifecycleCallbacks(object : Application.ActivityLifecycleCallbacks {
      override fun onActivityResumed(activity: Activity) {
        resumed += 1
        runCatching { nativeLifecycle(true) }
      }
      override fun onActivityPaused(activity: Activity) {
        resumed = (resumed - 1).coerceAtLeast(0)
        // Checkpoint before the process becomes killable; no continuous background promise.
        runCatching { nativeLifecycle(false) }
      }
      override fun onActivityCreated(activity: Activity, state: Bundle?) {}
      override fun onActivityStarted(activity: Activity) {}
      override fun onActivityStopped(activity: Activity) {
        main.postDelayed({ if (resumed == 0) runCatching { nativeLifecycle(false) } }, 500)
      }
      override fun onActivitySaveInstanceState(activity: Activity, outState: Bundle) {}
      override fun onActivityDestroyed(activity: Activity) {}
    })
    // Verify the JNI callback is linked before reporting the lifecycle adapter as supported.
    nativeLifecycle(true)
    installed = true
  }
}
