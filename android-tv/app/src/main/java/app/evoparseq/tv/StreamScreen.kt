package app.evoparseq.tv

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.input.key.onPreviewKeyEvent
import kotlinx.coroutines.delay
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.viewinterop.AndroidView
import androidx.tv.material3.Button
import androidx.tv.material3.ButtonDefaults
import androidx.tv.material3.Text

/** SurfaceView is the codec's producer endpoint, never a Bitmap/TextureView. */
@Composable
internal fun StreamScreen(model: TvModel, host: Host, close: () -> Unit) {
    val context = LocalContext.current
    val activity = context as MainActivity
    val controllers = activity.controllerInput
    val desktop = controllers.desktop
    val first = remember { FocusRequester() }
    var focusGranted by remember { mutableStateOf<Boolean?>(null) }
    val timing = remember { SurfaceTiming() }
    val status = model.streamStatus
    val frameRateHint = status?.optJSONObject("overlay")?.optDouble("frame_rate_hint", 0.0)?.toFloat() ?: 0f
    val video = status?.optJSONObject("video")
    val width = video?.optInt("width") ?: 0
    val height = video?.optInt("height") ?: 0
    val exit: (String) -> Unit = { reason -> model.stopStream(reason); close() }
    var visible by remember { mutableStateOf(true) }
    var interaction by remember { mutableStateOf(0) }
    var consumeCenterUp by remember { mutableStateOf(false) }
    val root = remember { FocusRequester() }
    val error = status?.optString("error").orEmpty()
    val streaming = status?.optString("state") == "Streaming"
    BackHandler { if (!visible) { visible = true; interaction++ } else exit("Stream closed with Back.") }
    LaunchedEffect(visible, streaming, error, activity.controllerWindowFocused) {
        controllers.enabled = !visible && streaming && error.isEmpty() && activity.controllerWindowFocused
    }
    LaunchedEffect(visible) {
        NativeCore.setOverlayVisible(visible)
        if (visible) first.requestFocus() else root.requestFocus()
    }
    LaunchedEffect(visible, interaction, streaming, error) {
        if (visible && streaming && error.isEmpty()) {
            delay(status?.optJSONObject("overlay")?.optLong("auto_hide_ms", 5000) ?: 5000)
            visible = false
        }
    }
    LaunchedEffect(error) { if (error.isNotEmpty()) { visible = true; interaction++ } }
    DisposableEffect(Unit) {
        val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_GAME)
                .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE).build())
            .setOnAudioFocusChangeListener { change ->
                if (change == AudioManager.AUDIOFOCUS_LOSS || change == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT) model.stopStream("Android audio focus was lost.")
            }.build()
        val granted = audio.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        focusGranted = granted
        onDispose { controllers.enabled = false; desktop.mouseMode = false; model.stopStream("Streaming screen was closed."); if (granted) audio.abandonAudioFocusRequest(request) }
    }
    BoxWithConstraints(Modifier.fillMaxSize().background(Color.Black)
        .onGloballyPositioned { coordinates -> val p=coordinates.positionInWindow(); desktop.viewport(p.x,p.y,coordinates.size.width,coordinates.size.height) }
        .focusRequester(root).onPreviewKeyEvent { event ->
            if (visible) desktop.observeNavigation(event.nativeKeyEvent)
            if (desktop.remoteKey(event.nativeKeyEvent)) return@onPreviewKeyEvent true
            if (controllers.key(event.nativeKeyEvent)) return@onPreviewKeyEvent true
            when(desktop.key(event.nativeKeyEvent)) {
                1 -> return@onPreviewKeyEvent true
                2 -> { visible=true; interaction++; return@onPreviewKeyEvent true }
            }
            val key = event.nativeKeyEvent
            val center = key.keyCode == android.view.KeyEvent.KEYCODE_DPAD_CENTER || key.keyCode == android.view.KeyEvent.KEYCODE_ENTER
            when {
                key.keyCode == android.view.KeyEvent.KEYCODE_MENU && key.action == android.view.KeyEvent.ACTION_UP -> true
                center && consumeCenterUp && key.action == android.view.KeyEvent.ACTION_UP -> { consumeCenterUp = false; true }
                key.action == android.view.KeyEvent.ACTION_DOWN && key.repeatCount == 0 && key.keyCode == android.view.KeyEvent.KEYCODE_MENU -> { visible = !visible; interaction++; true }
                key.action == android.view.KeyEvent.ACTION_DOWN && !visible && center -> { consumeCenterUp = true; visible = true; interaction++; true }
                else -> { if (key.action == android.view.KeyEvent.ACTION_DOWN) interaction++; false }
            }
        }.focusable()) {
        val ratio = if (width > 0 && height > 0) width.toFloat() / height else maxWidth / maxHeight
        val videoSize = if (maxWidth / maxHeight > ratio)
            Modifier.height(maxHeight).width(maxHeight * ratio)
        else Modifier.width(maxWidth).height(maxWidth / ratio)
        AndroidView(modifier = videoSize.align(Alignment.Center), factory = { ctx ->
            SurfaceView(ctx).also { surface ->
                // MediaCodec owns buffer geometry. Only the View's layout
                // follows the visible aspect ratio; do not resize its producer
                // buffers in response to codec-format callbacks.
                surface.keepScreenOn = true
                surface.isFocusable = false
                var started = false
                surface.holder.addCallback(object : SurfaceHolder.Callback {
                    override fun surfaceCreated(holder: SurfaceHolder) = Unit
                    override fun surfaceChanged(holder: SurfaceHolder, format: Int, w: Int, h: Int) {
                        if (!started && w > 0 && h > 0) {
                            started = true
                            model.startStream(holder.surface, host, w, h)
                        }
                    }
                    override fun surfaceDestroyed(holder: SurfaceHolder) {
                        timing.detached()
                        model.stopStream("Android destroyed the video Surface.")
                    }
                })
            }
        }, update = { view -> timing.apply(view.holder.surface, frameRateHint) })
        if (visible) Column(Modifier.align(Alignment.TopStart).padding(16.dp)
            .widthIn(max=400.dp).heightIn(max=(maxHeight-32.dp).coerceAtLeast(0.dp))
            .background(Color(0xE6171D26),RoundedCornerShape(12.dp))
            .verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val compactScale = ButtonDefaults.scale(focusedScale=1f)
            val compactPadding = PaddingValues(horizontal=12.dp,vertical=6.dp)
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(onClick = { visible = false }, scale=compactScale,contentPadding=compactPadding,
                    modifier = Modifier.focusRequester(first)) { Text("Hide overlay",fontSize=14.sp) }
                Button(onClick = { exit("Disconnected from stream controls.") },scale=compactScale,
                    contentPadding=compactPadding) { Text("Disconnect",fontSize=14.sp) }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(onClick={desktop.mouseMode=!desktop.mouseMode},enabled=streaming,
                    scale=compactScale,contentPadding=compactPadding) {
                    Text("Remote mouse: " + if(desktop.mouseMode) "On" else "Off",fontSize=14.sp)
                }
                Button(onClick={desktop.captureMouse=!desktop.captureMouse},scale=compactScale,
                    contentPadding=compactPadding) {
                    Text("Mouse capture: " + if(desktop.captureMouse) "On" else "Off",fontSize=14.sp)
                }
            }
            Text(if (error.isNotEmpty()) error else "${host.name} | ${status?.optString("state") ?: "Connecting..."}",
                color = Color.White,fontSize=14.sp,lineHeight=18.sp)
            val metrics = status?.optJSONObject("overlay")
            Row(Modifier.fillMaxWidth(),horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(metrics?.optString("fps") ?: "N/A FPS",modifier=Modifier.weight(1f),
                    color=Color(0xFF71BCFF),fontSize=13.sp,maxLines=1,overflow=TextOverflow.Ellipsis)
                Text("RTT ${metrics?.optString("rtt") ?: "N/A ms"}",modifier=Modifier.weight(1f),
                    color=Color.White,fontSize=13.sp,maxLines=1,overflow=TextOverflow.Ellipsis)
                Text("RX ${metrics?.optString("bitrate") ?: "N/A Mbit/s"}",modifier=Modifier.weight(1f),
                    color=Color.White,fontSize=13.sp,maxLines=1,overflow=TextOverflow.Ellipsis)
            }
            if (metrics != null) Text(metrics.optString("format"),color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
            if (metrics?.has("video_request") == true) {
                val request = metrics.getString("video_request")
                val rate = metrics.optString("video_payload_rate")
                Text(if (rate.isEmpty()) request else "$request | $rate",
                    color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
            }
            val displayHz = activity.window.decorView.display?.refreshRate
            Text("Display ${displayHz?.let { String.format(java.util.Locale.ROOT, "%.2f", it) } ?: "N/A"} Hz | Catch-up skips ${video?.optLong("skipped", 0) ?: 0}",
                color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
            val network = status?.optJSONObject("network")
            Text("${network?.optString("transport", "WebRTC") ?: "Connecting"} | ${network?.optString("route", "Unverified") ?: "Unverified"}",
                color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
            if(desktop.mouseMode) Text("D-pad: move · OK: click · Hold OK: right click\nOK + D-pad: drag",
                color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
            if (focusGranted == false) Text("Android did not grant audio focus",color=Color(0xFFFFB4AB),fontSize=12.sp)
            Text("Back: controls · Auto-hide: 5 s\nKeyboard: Ctrl+Shift+M releases input",
                color=Color(0xFFA8BED7),fontSize=12.sp,lineHeight=16.sp)
        }
    }
}
