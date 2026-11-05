package app.evoparseq.tv

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import java.io.File
import java.security.MessageDigest

/** Only package inspection and Android installer operations live in this adapter. */
internal object AppUpdater {
    @Suppress("DEPRECATION")
    fun installed(context: Context) = context.packageManager.getPackageInfo(context.packageName, PackageManager.GET_SIGNING_CERTIFICATES)
    fun clearPrepared(context: Context) {
        val prefs = context.getSharedPreferences("app-updates", Context.MODE_PRIVATE)
        val id = prefs.getInt("prepared", -1)
        if (id >= 0) runCatching { context.packageManager.packageInstaller.abandonSession(id) }
        prefs.edit().remove("prepared").commit()
    }
    fun version(context: Context): String = installed(context).versionName ?: "unknown"
    fun canInstall(context: Context) = context.packageManager.canRequestPackageInstalls()
    fun permission(activity: Activity) {
        activity.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}")))
    }
    @Suppress("DEPRECATION")
    fun prepare(context: Context, file: File, expectedVersion: String): Int {
        val pm = context.packageManager
        val candidate = pm.getPackageArchiveInfo(file.absolutePath, PackageManager.GET_SIGNING_CERTIFICATES)
            ?: error("The downloaded APK could not be inspected.")
        val current = installed(context)
        check(candidate.packageName == context.packageName) { "The APK belongs to a different application." }
        check(candidate.longVersionCode > current.longVersionCode) { "The APK does not have a newer Android version code." }
        check(candidate.versionName == expectedVersion.removePrefix("v")) { "The APK version does not match the release." }
        check((candidate.applicationInfo?.minSdkVersion ?: Int.MAX_VALUE) <= Build.VERSION.SDK_INT) { "This update requires a newer Android version." }
        fun signers(info: android.content.pm.PackageInfo): Set<String> =
            info.signingInfo?.apkContentsSigners?.map { signature ->
                MessageDigest.getInstance("SHA-256").digest(signature.toByteArray()).joinToString("") { "%02x".format(it.toInt() and 255) }
            }?.toSet() ?: emptySet()
        val old = signers(current)
        check(old.isNotEmpty() && signers(candidate) == old) {
            "The APK signing key differs from this installation. Updates cannot replace an older debug-signed installation."
        }
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(file.length())
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_REQUIRED)
        }
        val installer = pm.packageInstaller
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                session.openWrite("EvoParseq-TV.apk", 0, file.length()).use { output ->
                    file.inputStream().use { input -> input.copyTo(output) }
                    session.fsync(output)
                }
            }
        } catch (error: Exception) { installer.abandonSession(id); throw error }
        if (!context.getSharedPreferences("app-updates", Context.MODE_PRIVATE).edit().putInt("prepared", id).commit()) {
            installer.abandonSession(id)
            error("Could not save the prepared installation state.")
        }
        return id
    }
    fun install(activity: Activity, id: Int) {
        val intent = Intent(activity, UpdateInstallReceiver::class.java).setAction("${activity.packageName}.UPDATE_RESULT").putExtra("session", id)
        val mutable = if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
        val callback = PendingIntent.getBroadcast(activity, id, intent,
            PendingIntent.FLAG_UPDATE_CURRENT or mutable)
        activity.getSharedPreferences("app-updates", Context.MODE_PRIVATE).edit().remove("result").apply()
        activity.packageManager.packageInstaller.openSession(id).use { it.commit(callback.intentSender) }
        activity.getSharedPreferences("app-updates", Context.MODE_PRIVATE).edit().remove("prepared").apply()
    }
}

/** Explicit, non-exported system callback; package replacement may stop the app. */
class UpdateInstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != "${context.packageName}.UPDATE_RESULT") return
        val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
        val message = when (status) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirmation = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                    else intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
                try {
                    checkNotNull(confirmation)
                    context.startActivity(confirmation.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    "Confirm the update in the Android installer."
                } catch (_: Exception) { "Android could not open the installation confirmation. Please retry." }
            }
            PackageInstaller.STATUS_SUCCESS -> "Update installed."
            PackageInstaller.STATUS_FAILURE_ABORTED -> "Update cancelled. The installed app has not changed."
            else -> "Android could not install the update. Check storage, installation permission and APK compatibility."
        }
        context.getSharedPreferences("app-updates", Context.MODE_PRIVATE).edit().putString("result", message).apply()
    }
}
