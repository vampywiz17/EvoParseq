package app.evoparseq.tv

import android.content.Context
import android.media.AudioManager
import org.json.JSONArray
import org.json.JSONObject

/** One capability query per connection, outside the audio/video callbacks.
 * Rust matches these explicit speaker masks to AAudio's actual routed device. */
internal object AudioOutputs {
    fun inventory(context: Context): String {
        val devices = JSONArray()
        val manager = context.getSystemService(AudioManager::class.java)
        manager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).take(64).forEach { device ->
            devices.put(JSONObject().put("id", device.id)
                .put("masks", JSONArray(device.channelMasks.take(64))))
        }
        return JSONObject().put("devices", devices).toString()
    }
}
