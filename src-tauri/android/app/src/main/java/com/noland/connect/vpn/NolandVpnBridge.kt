package com.noland.connect.vpn

import android.content.Context
import android.content.Intent
import android.net.VpnService
import androidx.annotation.Keep
import com.wireguard.android.backend.BackendException
import com.wireguard.android.backend.GoBackend
import com.wireguard.android.backend.Tunnel
import com.wireguard.config.Config
import org.json.JSONObject
import java.io.BufferedReader
import java.io.IOException
import java.io.StringReader
import java.security.MessageDigest

/** Static JSON boundary used by the native Android runtime to own one WireGuard tunnel. */
@Keep
object NolandVpnBridge {
  private const val TUNNEL_NAME = "noland"
  private const val PREFERENCES_NAME = "noland_wireguard_vpn"
  private const val KEY_CONFIGURATION = "configuration"
  private const val KEY_CONFIGURATION_REFERENCE = "configuration_reference"
  private const val KEY_LAUNCH_ID = "launch_id"
  private const val KEY_CONFIG_FINGERPRINT = "config_fingerprint"
  private const val DEFAULT_MTU = 1280

  private val lock = Any()
  private var goBackend: GoBackend? = null

  private data class StoredTunnel(
    val configuration: String,
    val configurationReference: String,
    val launchId: String,
    val configFingerprint: String,
  )

  private object NolandTunnel : Tunnel {
    @Volatile
    var state: Tunnel.State = Tunnel.State.DOWN
      private set

    override fun getName(): String = TUNNEL_NAME

    override fun onStateChange(newState: Tunnel.State) {
      state = newState
    }
  }

  @JvmStatic
  fun installAndStart(context: Context, requestJson: String): String = respond(::statusFailure) {
    val request = JSONObject(requestJson)
    val configurationReference = request.requiredString("configurationReference")
    val configurationText = request.requiredString("configuration")
    val launchId = request.requiredString("launchId")
    val requestedFingerprint = request.requiredString("configFingerprint")
    val computedFingerprint = fingerprint(configurationText)
    require(requestedFingerprint == computedFingerprint) {
      "The WireGuard configuration fingerprint does not match its contents."
    }
    val config = parseConfiguration(configurationText)
    val previous = load(context)
    val next = StoredTunnel(
      configuration = configurationText,
      configurationReference = configurationReference,
      launchId = launchId,
      configFingerprint = requestedFingerprint,
    )

    save(context, next)
    permissionRequired(context, configurationReference)?.let { return@respond it }

    try {
      backend(context).setState(NolandTunnel, Tunnel.State.UP, config)
    } catch (error: Throwable) {
      if (error.isPermissionFailure()) {
        return@respond permissionRequiredAfterBackendFailure(context, configurationReference)
      }
      restore(context, previous)
      throw error
    }
    statusResponse(true, "connected", configurationReference, null)
  }

  @JvmStatic
  fun reconnect(context: Context, requestJson: String): String = respond(::statusFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    val stored = matchingStoredTunnel(context, reference)
      ?: return@respond statusResponse(
        false,
        "not_configured",
        reference,
        "The requested protected tunnel is not configured.",
      )
    val config = parseConfiguration(stored.configuration)
    permissionRequired(context, reference)?.let { return@respond it }

    try {
      backend(context).setState(NolandTunnel, Tunnel.State.UP, config)
    } catch (error: Throwable) {
      if (error.isPermissionFailure()) {
        return@respond permissionRequiredAfterBackendFailure(context, reference)
      }
      throw error
    }
    statusResponse(true, "connected", reference, null)
  }

  @JvmStatic
  fun stop(context: Context, requestJson: String): String = respond(::statusFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    if (matchingStoredTunnel(context, reference) == null) {
      return@respond statusResponse(true, "not_configured", reference, null)
    }
    val backend = backend(context)
    if (backend.getState(NolandTunnel) == Tunnel.State.UP) {
      backend.setState(NolandTunnel, Tunnel.State.DOWN, null)
    }
    statusResponse(true, "disconnected", reference, null)
  }

  @JvmStatic
  fun remove(context: Context, requestJson: String): String = respond(::statusFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    if (matchingStoredTunnel(context, reference) != null) {
      val backend = backend(context)
      if (backend.getState(NolandTunnel) == Tunnel.State.UP) {
        backend.setState(NolandTunnel, Tunnel.State.DOWN, null)
      }
      clear(context)
    }
    statusResponse(true, "not_configured", null, null)
  }

  @JvmStatic
  fun status(context: Context, requestJson: String): String = respond(::statusFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    if (matchingStoredTunnel(context, reference) == null) {
      return@respond statusResponse(true, "not_configured", reference, null)
    }
    val state = backend(context).getState(NolandTunnel)
    statusResponse(
      true,
      if (state == Tunnel.State.UP) "connected" else "stored_inactive",
      reference,
      null,
    )
  }

  @JvmStatic
  fun runtime(context: Context, requestJson: String): String = respond(::runtimeFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    val stored = matchingStoredTunnel(context, reference)
      ?: return@respond runtimeResponse(error = "The requested protected tunnel is not configured.")
    val config = parseConfiguration(stored.configuration)
    runtimeResponse(context, stored, config, null)
  }

  @JvmStatic
  fun configuration(context: Context, requestJson: String): String = respond(::configurationFailure) {
    val reference = JSONObject(requestJson).requiredString("configurationReference")
    val stored = matchingStoredTunnel(context, reference)
      ?: return@respond configurationResponse(
        false,
        null,
        "The protected WireGuard configuration is missing.",
      )
    configurationResponse(true, stored.configuration, null)
  }

  @JvmStatic
  fun update(context: Context, requestJson: String): String = respond(::runtimeFailure) {
    val request = JSONObject(requestJson)
    val reference = request.requiredString("configurationReference")
    val configurationText = request.requiredString("configuration")
    val expectedFingerprint = request.requiredString("expectedConfigFingerprint")
    val requestedFingerprint = request.requiredString("configFingerprint")
    val transitionId = request.requiredString("transitionId")
    val previous = matchingStoredTunnel(context, reference)
      ?: return@respond runtimeResponse(error = "The requested protected tunnel is not configured.")

    require(previous.configFingerprint == expectedFingerprint) {
      "The tunnel changed before this transition could be applied."
    }
    require(fingerprint(configurationText) == requestedFingerprint) {
      "The updated WireGuard configuration fingerprint is invalid."
    }

    val backend = backend(context)
    if (backend.getState(NolandTunnel) != Tunnel.State.UP) {
      return@respond runtimeResponse(error = "The WireGuard tunnel is not active.")
    }

    val previousConfig = parseConfiguration(previous.configuration)
    val nextConfig = parseConfiguration(configurationText)
    val next = previous.copy(
      configuration = configurationText,
      configFingerprint = requestedFingerprint,
    )
    backend.setState(NolandTunnel, Tunnel.State.UP, nextConfig)
    try {
      save(context, next)
    } catch (commitError: Throwable) {
      try {
        backend.setState(NolandTunnel, Tunnel.State.UP, previousConfig)
      } catch (rollbackError: Throwable) {
        throw IOException(
          "Protected configuration commit failed (${commitError.userMessage()}); " +
            "runtime rollback also failed (${rollbackError.userMessage()}).",
          commitError,
        )
      }
      throw commitError
    }
    runtimeResponse(context, next, nextConfig, transitionId)
  }

  private fun respond(
    failure: (Throwable) -> JSONObject,
    operation: () -> JSONObject,
  ): String = synchronized(lock) {
    try {
      operation()
    } catch (error: Throwable) {
      failure(error)
    }.toString()
  }

  private fun backend(context: Context): GoBackend {
    goBackend?.let { return it }
    return GoBackend(context.applicationContext).also { goBackend = it }
  }

  private fun parseConfiguration(configuration: String): Config =
    Config.parse(BufferedReader(StringReader(configuration)))

  private fun preferences(context: Context) = context.applicationContext.getSharedPreferences(
    PREFERENCES_NAME,
    Context.MODE_PRIVATE,
  )

  private fun load(context: Context): StoredTunnel? {
    val preferences = preferences(context)
    val configuration = preferences.getString(KEY_CONFIGURATION, null) ?: return null
    val reference = preferences.getString(KEY_CONFIGURATION_REFERENCE, null) ?: return null
    val launchId = preferences.getString(KEY_LAUNCH_ID, null) ?: return null
    val configFingerprint = preferences.getString(KEY_CONFIG_FINGERPRINT, null) ?: return null
    return StoredTunnel(configuration, reference, launchId, configFingerprint)
  }

  private fun matchingStoredTunnel(context: Context, reference: String): StoredTunnel? =
    load(context)?.takeIf { it.configurationReference == reference }

  private fun save(context: Context, tunnel: StoredTunnel) {
    val committed = preferences(context).edit()
      .putString(KEY_CONFIGURATION, tunnel.configuration)
      .putString(KEY_CONFIGURATION_REFERENCE, tunnel.configurationReference)
      .putString(KEY_LAUNCH_ID, tunnel.launchId)
      .putString(KEY_CONFIG_FINGERPRINT, tunnel.configFingerprint)
      .commit()
    if (!committed) throw IOException("The protected WireGuard configuration could not be persisted.")
  }

  private fun restore(context: Context, previous: StoredTunnel?) {
    if (previous == null) clear(context) else save(context, previous)
  }

  private fun clear(context: Context) {
    if (!preferences(context).edit().clear().commit()) {
      throw IOException("The protected WireGuard configuration could not be removed.")
    }
  }

  private fun permissionRequired(context: Context, reference: String): JSONObject? {
    val intent = VpnService.prepare(context) ?: return null
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    context.startActivity(intent)
    return statusResponse(
      false,
      "permission_required",
      reference,
      "VPN permission is required; an authorization request was launched.",
    )
  }

  private fun permissionRequiredAfterBackendFailure(
    context: Context,
    reference: String,
  ): JSONObject = permissionRequired(context, reference) ?: statusResponse(
    false,
    "permission_required",
    reference,
    "VPN permission is required; retry after granting authorization.",
  )

  private fun runtimeResponse(
    context: Context,
    stored: StoredTunnel,
    config: Config,
    transitionId: String?,
  ): JSONObject {
    val backend = backend(context)
    if (backend.getState(NolandTunnel) != Tunnel.State.UP) {
      return runtimeResponse(error = "The WireGuard tunnel is not active.")
    }

    val peer = config.getPeers().firstOrNull { it.getEndpoint().isPresent }
      ?: return runtimeResponse(error = "The WireGuard endpoint and peer read-back are unavailable.")
    val endpoint = peer.getEndpoint().orElse(null)?.toString()
      ?: return runtimeResponse(error = "The WireGuard endpoint read-back is unavailable.")
    val statistics = backend.getStatistics(NolandTunnel)
    val latestHandshakeMillis = statistics.peers()
      .mapNotNull { statistics.peer(it)?.latestHandshakeEpochMillis()?.takeIf { value -> value > 0 } }
      .maxOrNull()
    val latestHandshakeAgeSecs = latestHandshakeMillis?.let {
      ((System.currentTimeMillis() - it).coerceAtLeast(0L)) / 1_000L
    }

    return runtimeResponse(
      ok = true,
      error = null,
      active = true,
      launchId = stored.launchId,
      configFingerprint = stored.configFingerprint,
      peerPublicKey = peer.getPublicKey().toBase64(),
      endpoint = endpoint,
      mtu = config.getInterface().getMtu().orElse(DEFAULT_MTU),
      latestHandshakeAgeSecs = latestHandshakeAgeSecs,
      rxBytes = statistics.totalRx(),
      txBytes = statistics.totalTx(),
      transitionId = transitionId,
    )
  }

  private fun runtimeResponse(
    ok: Boolean = false,
    error: String?,
    active: Boolean = false,
    launchId: String = "",
    configFingerprint: String = "",
    peerPublicKey: String = "",
    endpoint: String = "",
    mtu: Int = 0,
    latestHandshakeAgeSecs: Long? = null,
    rxBytes: Long = 0,
    txBytes: Long = 0,
    transitionId: String? = null,
  ): JSONObject = JSONObject()
    .put("ok", ok)
    .putNullable("error", error)
    .put("active", active)
    .put("launchId", launchId)
    .put("configFingerprint", configFingerprint)
    .put("peerPublicKey", peerPublicKey)
    .put("endpoint", endpoint)
    .put("mtu", mtu)
    .putNullable("latestHandshakeAgeSecs", latestHandshakeAgeSecs)
    .put("rxBytes", rxBytes)
    .put("txBytes", txBytes)
    .also { response ->
      if (transitionId != null) response.put("transitionId", transitionId)
    }

  private fun statusResponse(
    ok: Boolean,
    status: String,
    configurationReference: String?,
    error: String?,
  ): JSONObject = JSONObject()
    .put("ok", ok)
    .put("status", status)
    .putNullable("configurationReference", configurationReference)
    .putNullable("error", error)

  private fun configurationResponse(
    ok: Boolean,
    configuration: String?,
    error: String?,
  ): JSONObject = JSONObject()
    .put("ok", ok)
    .putNullable("configuration", configuration)
    .putNullable("error", error)

  private fun statusFailure(error: Throwable): JSONObject =
    statusResponse(false, "failed", null, error.userMessage())

  private fun runtimeFailure(error: Throwable): JSONObject =
    runtimeResponse(error = error.userMessage())

  private fun configurationFailure(error: Throwable): JSONObject =
    configurationResponse(false, null, error.userMessage())

  private fun JSONObject.requiredString(name: String): String {
    if (!has(name) || isNull(name)) throw IllegalArgumentException("Missing VPN request field: $name.")
    return getString(name).takeIf { it.isNotBlank() }
      ?: throw IllegalArgumentException("VPN request field $name must not be empty.")
  }

  private fun JSONObject.putNullable(name: String, value: Any?): JSONObject =
    put(name, value ?: JSONObject.NULL)

  private fun fingerprint(configuration: String): String = MessageDigest
    .getInstance("SHA-256")
    .digest(configuration.toByteArray(Charsets.UTF_8))
    .joinToString("") { byte -> "%02x".format(byte.toInt() and 0xff) }

  private fun Throwable.isPermissionFailure(): Boolean =
    this is BackendException && reason == BackendException.Reason.VPN_NOT_AUTHORIZED

  private fun Throwable.userMessage(): String = when (this) {
    is BackendException -> when (reason) {
      BackendException.Reason.VPN_NOT_AUTHORIZED -> "VPN permission is required."
      BackendException.Reason.TUNNEL_MISSING_CONFIG -> "The WireGuard tunnel configuration is missing."
      BackendException.Reason.UNABLE_TO_START_VPN -> "Android could not start the VPN service."
      BackendException.Reason.TUN_CREATION_ERROR -> "Android could not create the VPN interface."
      BackendException.Reason.GO_ACTIVATION_ERROR_CODE -> "The WireGuard userspace backend could not activate."
      BackendException.Reason.DNS_RESOLUTION_FAILURE -> "The WireGuard endpoint could not be resolved."
      else -> "The WireGuard backend failed: ${reason.name}."
    }
    else -> message?.takeIf { it.isNotBlank() } ?: javaClass.simpleName
  }
}
