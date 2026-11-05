package app.evoparseq.tv

import android.os.Handler
import android.os.Looper
import android.app.Application
import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import org.json.JSONObject
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

internal data class Host(val peer: String, val name: String)

/** Retained through Activity recreation. One bounded account request at a time. */
internal class TvModel(application: Application) : AndroidViewModel(application) {
    var signedIn by mutableStateOf(false)
        private set
    var busy by mutableStateOf(false)
        private set
    var hosts by mutableStateOf(emptyList<Host>())
        private set
    var message by mutableStateOf("")
        private set
    var inventory by mutableStateOf("")
        private set
    var settings by mutableStateOf<JSONObject?>(null)
        private set
    var streamStatus by mutableStateOf<JSONObject?>(null)
        private set
    var networkSummary by mutableStateOf(JSONObject().put("provider", "direct"))
        private set
    var localSetup by mutableStateOf<JSONObject?>(null)
        private set
    private var setupGeneration = 0
    private var streamGeneration = 0
    var lastStreamStopReason by mutableStateOf("")
        private set
    var updateRelease by mutableStateOf<JSONObject?>(null)
        private set
    var updateMessage by mutableStateOf("")
        private set
    var updateSession by mutableStateOf<Int?>(null)
        private set
    val installedVersion: String get() = AppUpdater.version(getApplication())

    fun checkUpdates(prerelease: Boolean) = request {
        AppUpdater.clearPrepared(getApplication())
        val reply = JSONObject(NativeCore.checkUpdate(installedVersion, prerelease))
        val candidate = reply.optJSONObject("release")
        return@request {
            updateSession = null
            updateRelease = candidate
            updateMessage = if (!reply.optBoolean("ok")) reply.optString("error")
                else if (candidate == null) "No newer compatible APK was found in this release channel."
                else "Update available. Download it to continue."
        }
    }
    fun downloadUpdate() = request {
        val context = getApplication<Application>()
        val file = java.io.File(context.cacheDir, "evoparseq-update.apk")
        val prepared = try {
            val reply = JSONObject(NativeCore.downloadUpdate(file.absolutePath))
            check(reply.optBoolean("ok")) { reply.optString("error") }
            AppUpdater.prepare(context, file, reply.getString("version"))
        } catch (error: Exception) {
            val detail = if (error is IllegalStateException) error.message ?: "The APK could not be prepared."
                else "Android could not prepare the update. Check available storage and retry."
            return@request { updateMessage = detail }
        } finally { file.delete() }
        return@request { updateSession = prepared; updateMessage = "APK verified. Choose Install update, then confirm in Android." }
    }
    fun installUpdate(activity: android.app.Activity) {
        try {
            if (!AppUpdater.canInstall(activity)) {
                AppUpdater.permission(activity)
                updateMessage = "Allow EvoParseq to install apps, then return and choose Install update again."
                return
            }
            val id = updateSession ?: return
            AppUpdater.install(activity, id)
            updateSession = null
            updateMessage = "Waiting for Android installation confirmation."
        } catch (_: Exception) { updateMessage = "Could not start the Android installer. Please retry." }
    }
    fun readUpdateResult() {
        val prefs = getApplication<Application>().getSharedPreferences("app-updates", Context.MODE_PRIVATE)
        prefs.getString("result", null)?.let { updateMessage = it; prefs.edit().remove("result").apply() }
    }
    private val preferences = application.getSharedPreferences("tv-requests", Context.MODE_PRIVATE)
    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val disposed = AtomicBoolean(false)

    private val vault = SessionVault(application)
    private val networkVault = SessionVault(application, "network")
    init {
        request {
            AppUpdater.clearPrepared(application)
            val previousCrash = recentAppFailure(application)
            val saved = preferences.getString("requests", "") ?: ""
            val reply = JSONObject(NativeCore.settings(saved, "", 0))
            var restored = false
            var feedback = reply.optString("error")
            try {
                val token = vault.load()
                if (token != null) {
                    val auth = try { JSONObject(NativeCore.restoreSession(token)) } finally { token.fill(0) }
                    restored = auth.optBoolean("ok")
                    if (!restored) { vault.clear(); feedback = auth.optString("error") }
                }
            } catch (_: Exception) {
                runCatching { vault.clear() }
                feedback = "Saved sign-in could not be restored. Sign in again."
            }
            var network = JSONObject().put("provider", "direct")
            try {
                val raw = networkVault.load()
                if (raw != null) {
                    val savedNetwork = try { JSONObject(NativeCore.restoreNetwork(raw)) } finally { raw.fill(0) }
                    if (savedNetwork.optBoolean("ok")) network = savedNetwork.getJSONObject("network")
                    else { networkVault.clear(); feedback = savedNetwork.optString("error") }
                }
            } catch (_: Exception) {
                runCatching { networkVault.clear() }
                feedback = "Saved relay settings could not be restored. Configure the relay again."
            }
            val restoredNetwork = network
            return@request { if (reply.optBoolean("ok")) settings = reply; signedIn = restored; networkSummary = restoredNetwork; message = feedback; if(previousCrash != null) lastStreamStopReason = previousCrash }
        }
    }

    private fun request(work: () -> (() -> Unit)) {
        if (busy || disposed.get()) return
        busy = true
        message = ""
        worker.execute {
            // Do not expose Java/JNI exception messages or raw HTTP response bodies.
            val complete = try { work() } catch (_: Exception) {
                { message = "The request could not be completed. Please try again." }
            } catch (_: LinkageError) {
                { message = "The native core is unavailable for this device." }
            }
            main.post {
                if (!disposed.get()) { complete(); busy = false }
            }
        }
    }

    fun signIn(email: String, password: String, code: String) = request {
        val reply = JSONObject(NativeCore.signIn(email, password, code))
        val ok = reply.optBoolean("ok")
        val feedback = if (ok) JSONObject(NativeCore.rememberSession(vault)).optString("error") else reply.optString("error")
        return@request { signedIn = ok; message = feedback }
    }

    fun refresh() = request {
        val reply = JSONObject(NativeCore.hosts())
        val next = reply.optJSONArray("hosts")
        val updated = if (next == null) null else (0 until next.length()).map {
            val host = next.getJSONObject(it)
            Host(host.getString("peer"), host.getString("name"))
        }
        val expired = reply.optBoolean("expired")
        if (expired) { runCatching { vault.clear() }; NativeCore.signOut() }
        val error = reply.optString("error")
        return@request { if (updated != null) hosts = updated; if (expired) { signedIn = false; hosts = emptyList() }; message = error }
    }

    fun signOut() = request {
        val forgotten = runCatching { vault.clear() }.isSuccess
        NativeCore.signOut()
        return@request {
            signedIn = false; hosts = emptyList()
            message = if (forgotten) "" else "Signed out, but local sign-in storage could not be fully cleared."
        }
    }

    fun inspectDecoders() = request {
        val text = DecoderInventory.describe()
        return@request { inventory = text }
    }

    fun readSettings() = request {
        val saved = preferences.getString("requests", "") ?: ""
        val reply = JSONObject(NativeCore.settings(saved, "", 0))
        val error = reply.optString("error")
        return@request { if (reply.optBoolean("ok")) settings = reply; message = error }
    }

    fun selectSetting(key: String, index: Int) {
        val current = settings?.getJSONObject("settings")?.toString() ?: return
        request {
            val reply = JSONObject(NativeCore.settings(current, key, index))
            var feedback = reply.optString("error")
            val saved = reply.optBoolean("ok") && preferences.edit()
                .putString("requests", reply.getJSONObject("settings").toString()).commit()
            if (reply.optBoolean("ok") && !saved) feedback = "Could not save TV settings."
            return@request {
                if (saved) settings = reply
                message = if (saved) "Saved on this TV. Applies to the next connection." else feedback
            }
        }
    }

    fun openLocalSetup() = request {
        val context = getApplication<Application>()
        val manager = context.getSystemService(Context.CONNECTIVITY_SERVICE) as android.net.ConnectivityManager
        val ip = manager.getLinkProperties(manager.activeNetwork)?.linkAddresses
            ?.map { it.address }?.filterIsInstance<java.net.Inet4Address>()
            ?.firstOrNull { it.isSiteLocalAddress }?.hostAddress
        val reply = if (ip == null) JSONObject().put("error", "No private LAN address. Check Wi-Fi or Ethernet.")
            else JSONObject(NativeCore.startLocalSetup(ip))
        return@request {
            if (reply.optBoolean("ok")) { localSetup = reply; pollSetup(++setupGeneration) }
            else message = reply.optString("error")
        }
    }
    private fun pollSetup(generation: Int) {
        if (generation != setupGeneration || disposed.get()) return
        worker.execute {
            val reply = runCatching { JSONObject(NativeCore.localSetupStatus()) }
                .getOrElse { JSONObject().put("closed", true).put("error", "Local setup unavailable") }
            main.post {
                if (generation == setupGeneration && !disposed.get()) {
                    localSetup = reply
                    if (!reply.optBoolean("closed")) main.postDelayed({ pollSetup(generation) }, 500)
                }
            }
        }
    }
    fun confirmLocalSetup() = request {
        val reply = JSONObject(NativeCore.confirmLocalSetup(networkVault))
        return@request {
            if (reply.optBoolean("ok")) {
                networkSummary = reply.getJSONObject("network"); ++setupGeneration; localSetup = null
                message = "Relay settings saved securely. Applies to the next connection."
            } else message = reply.optString("error")
        }
    }
    fun closeLocalSetup() {
        ++setupGeneration; localSetup = null
        worker.execute { runCatching { NativeCore.stopLocalSetup() } }
    }
    fun resetNetwork() = request {
        networkVault.clear()
        val reply = JSONObject(NativeCore.resetNetwork())
        return@request { networkSummary = reply.getJSONObject("network"); message = "Relay settings removed. Direct/STUN restored." }
    }

    fun startStream(surface: android.view.Surface, host: Host, width: Int, height: Int) {
        val current = settings?.getJSONObject("settings")?.toString() ?: return
        val generation = ++streamGeneration
        lastStreamStopReason = ""
        message = ""
        streamStatus = JSONObject().put("state", "Connecting…")
        worker.execute {
            val reply = runCatching {
                JSONObject(NativeCore.startStream(surface, host.peer, current, DecoderInventory.choices(), width, height))
            }.getOrElse { JSONObject().put("error", "Could not start hardware streaming on this device.") }
            main.post {
                if (generation == streamGeneration && !disposed.get()) {
                    streamStatus = if (reply.has("error")) reply.put("ended", true) else streamStatus
                    if (!reply.has("error")) pollStream(generation)
                }
            }
        }
    }

    private fun pollStream(generation: Int) {
        if (generation != streamGeneration || disposed.get()) return
        worker.execute {
            val status = runCatching { JSONObject(NativeCore.streamStatus()) }.getOrElse {
                JSONObject().put("error", "Stream status unavailable").put("ended", true)
            }
            main.post {
                if (generation == streamGeneration && !disposed.get()) {
                    streamStatus = status
                    if (!status.optBoolean("ended")) main.postDelayed({ pollStream(generation) }, 500)
                }
            }
        }
    }

    fun activityRecreated() {
        if (streamStatus != null) {
            lastStreamStopReason = "Android recreated the streaming Activity. Reconnect to start a new Surface."
            android.util.Log.i("EvoParseq", lastStreamStopReason)
        }
    }

    fun stopStream(reason: String = "Stream closed.") {
        if (lastStreamStopReason.isEmpty() && streamStatus != null) {
            lastStreamStopReason = reason
            message = reason
            android.util.Log.i("EvoParseq", "Stream stop requested: $reason")
        }
        ++streamGeneration
        // Serialized after any pending JNI start: a destroyed Surface cannot
        // leave a late session alive. This stop only signals, never joins UI.
        worker.execute { runCatching { NativeCore.stopStream() } }
    }

    override fun onCleared() {
        ++setupGeneration
        disposed.set(true)
        // Saved sign-in survives normal Activity/process shutdown; only explicit logout revokes it.
        worker.execute { runCatching { NativeCore.releaseSession() } }
        worker.shutdown()
    }
}
