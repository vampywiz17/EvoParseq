package app.evoparseq.tv

/** Control/lifecycle commands only. Decoded pixels never cross JNI. */
internal object NativeCore {
    init { System.loadLibrary("evoparseq_tv") }
    external fun checkUpdate(installed: String, prerelease: Boolean): String
    external fun downloadUpdate(destination: String): String
    external fun signIn(email: String, password: String, tfa: String): String
    external fun rememberSession(vault: SessionVault): String
    external fun restoreSession(token: ByteArray): String
    external fun startLocalSetup(ip: String): String
    external fun localSetupStatus(): String
    external fun confirmLocalSetup(vault: SessionVault): String
    external fun stopLocalSetup()
    external fun restoreNetwork(bytes: ByteArray): String
    external fun resetNetwork(): String
    external fun hosts(): String
    external fun signOut()
    external fun releaseSession()
    external fun settings(current: String, field: String, index: Int): String
    external fun startStream(surface: android.view.Surface, peer: String, preferences: String,
        decoders: String, width: Int, height: Int): String
    external fun stopStream()
    external fun streamStatus(): String
    external fun controllerAdapter(adapter: ControllerInput?)
    external fun controllerFocus(active: Boolean)
    external fun controllerButton(id: Int, code: Int, down: Boolean): Boolean
    external fun controllerRemoved(id: Int)
    external fun controllerAxes(id: Int, x: Float, y: Float, rx: Float, ry: Float, lt: Float, rt: Float,
        hx: Float, hy: Float, fx: Float, fy: Float, frx: Float, fry: Float, flt: Float, frt: Float)
    external fun remoteMouseMode(enabled: Boolean, longMs: Int, deviceId: Int)
    external fun remoteMouseKey(code: Int, down: Boolean, cancelled: Boolean, deviceId: Int, directionalSource: Boolean): Boolean
    external fun desktopKey(scan: Int, code: Int, meta: Int, down: Boolean, cancelled: Boolean, repeat: Int): Int
    external fun desktopPointer(x: Double,y: Double,relative: Boolean,moved: Boolean,w: Int,h: Int,buttons: Int,wx: Int,wy: Int)
    external fun desktopViewport(w: Int,h: Int)
    external fun desktopRelease()
    external fun setOverlayVisible(visible: Boolean)
}
