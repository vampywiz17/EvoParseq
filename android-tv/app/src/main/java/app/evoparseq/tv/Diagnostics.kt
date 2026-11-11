package app.evoparseq.tv

import android.content.Context
import android.content.Intent
import android.content.ClipData
import android.os.Build
import androidx.core.content.FileProvider
import org.json.JSONObject
import java.io.File
import java.util.zip.ZipEntry
import java.util.zip.ZipOutputStream

/** Dedicated provider avoids device-specific issues with directly declaring the base provider. */
class DiagnosticsFileProvider : FileProvider()

/** OS file/ZIP adapter. Never walk private storage or export settings/vaults. */
internal object Diagnostics {
    fun directory(context: Context) = File(context.noBackupFilesDir, "diagnostics")
    fun configure(context: Context, enabled: Boolean): Boolean = JSONObject(
        NativeCore.configureDiagnostics(directory(context).absolutePath, enabled)).optBoolean("ok")

    // Called only on the existing account worker. Ten MiB maximum input logs;
    // copy at most that much even if a broken writer grows a file unexpectedly.
    fun prepare(context: Context): File {
        check(JSONObject(NativeCore.flushDiagnostics()).optBoolean("ok"))
        val destination = File(context.cacheDir, "diagnostics-exports").apply { mkdirs() }
        destination.listFiles()?.filter { it.name.startsWith("EvoParseq-") && it.extension == "zip" }?.forEach { old ->
            // Never reuse an already shared URI for a future archive.
            runCatching { context.revokeUriPermission(FileProvider.getUriForFile(context,
                "${context.packageName}.diagnostics", old), Intent.FLAG_GRANT_READ_URI_PERMISSION) }
            old.delete()
        }
        val file = File.createTempFile("EvoParseq-", ".zip", destination)
        try {
            ZipOutputStream(file.outputStream().buffered()).use { zip ->
                listOf("client-events.jsonl", "client-events.previous.jsonl").forEach { name ->
                    val source = File(directory(context), name)
                    if (source.isFile) {
                        zip.putNextEntry(ZipEntry(name))
                        source.inputStream().use { input ->
                            val block = ByteArray(8192)
                            var remaining = 5 * 1024 * 1024
                            while (remaining > 0) {
                                val count = input.read(block, 0, minOf(block.size, remaining))
                                if (count < 0) break
                                zip.write(block, 0, count); remaining -= count
                            }
                        }
                        zip.closeEntry()
                    }
                }
                zip.putNextEntry(ZipEntry("device-info.txt"))
                val info = "EvoParseq TV ${AppUpdater.version(context)}\nAndroid ${Build.VERSION.RELEASE} / API ${Build.VERSION.SDK_INT}\n${Build.MANUFACTURER} ${Build.MODEL}\nABIs: ${Build.SUPPORTED_ABIS.joinToString()}\n"
                zip.write(info.take(4096).toByteArray(Charsets.UTF_8))
                zip.closeEntry()
            }
            return file
        } catch (error: Exception) { file.delete(); throw error }
    }
    fun share(context: Context, file: File) {
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.diagnostics", file)
        val intent = Intent(Intent.ACTION_SEND).setType("application/zip")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        intent.clipData = ClipData.newRawUri("Diagnostics", uri)
        context.startActivity(Intent.createChooser(intent, "Share diagnostics"))
    }
}
