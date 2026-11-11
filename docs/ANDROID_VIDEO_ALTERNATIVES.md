# Android video alternatives (2026-11-05)

Scope: investigate 4:4:4, VRR and the Windows application's strict Zero copy mode.
This change adds audio features and diagnostics only. It does not add HID,
experimental video profiles, a GPU render pass or a decoded-frame copy.

## 4:4:4

The public `AVCProfileHigh444` constant exists since API 16, but this is a codec
profile identifier, not proof that a device decodes it in hardware. It also does
not establish that the Parsec host can send a compatible H.264 4:4:4 stream.
The public HEVC `HEVCProfileMain444` (8-bit) capability is added at API 37.
The current APK compiles against API 36 and supports API 29+, with the live
Android 14 box at API 34. There is therefore no portable existing HEVC 4:4:4
capability contract for this target to advertise. Do not infer it from Main10,
a vendor decoder name, hardware-acceleration flag or the presence of Surface output.

A future API-37 implementation needs a reported compatible profile, size/rate
support, a real host bitstream with validated SPS chroma/depth, successful native
Surface decoding and output-format validation. Neither an 8-bit Main444 constant
nor Main10 implies HEVC 10-bit 4:4:4. Do not add guessed profile integers, private
vendor flags or a CPU readback test to the live path. Current requests stay 4:2:0.

Source: [Android CodecProfileLevel reference](https://developer.android.com/reference/android/media/MediaCodecInfo.CodecProfileLevel).

## Refresh rate / VRR

Android's closest platform facility is adaptive refresh rate (ARR), introduced
in Android 15 with compatible framework, kernel, display and Hardware Composer
HAL support. It is not a universally available application toggle equivalent to
Windows DXGI/VRR. The tested Android 14 TV box cannot be assumed to support it,
nor does a TV's HDMI VRR capability establish that Android uses it.

Keep the existing documented Surface frame-rate hint and seamless-only policy.
It requests a suitable display cadence; it is not proof of variable physical
scanout. No additional timer, duplicate-frame producer or buffering is justified.
A future supported ARR target requires OS/device capability validation and live
timing tests before exposing a setting as equivalent to Windows VRR.

Sources: [AOSP ARR](https://source.android.com/docs/core/graphics/arr),
[Android app ARR guidance](https://developer.android.com/develop/ui/views/animations/adaptive-refresh-rate).

## Strict Zero copy

The current native MediaCodec decoder produces into the SurfaceView Surface.
The app does not retrieve decoded pixels, read them to CPU memory, create a
Bitmap, add a TextureView/GL intermediate or perform a separate conversion pass.
That is the appropriate Android output route for the project's minimal-copy goal.
It does not prove driver-side copies, color conversion or composition are absent.

SurfaceFlinger and Hardware Composer decide whether layers use hardware overlays
or GPU composition. Android provides no portable per-app contract that guarantees
original decoder surfaces reach scanout without conversion for every device and
window state. Therefore do not add a misleading Windows-equivalent Zero copy
checkbox. Preserve the existing path and state only what the app can establish.

Sources: [SurfaceFlinger and WindowManager](https://source.android.com/docs/core/graphics/surfaceflinger-windowmanager),
[SurfaceView](https://developer.android.com/reference/android/view/SurfaceView).

## Audio and diagnostic API boundaries

AAudio open/start/stop/close stays on the session/control thread. The callback
only renders the common bounded PCM queue and sets atomic error flags. Recovery
uses the existing shared retry policy. Surround is declared only after opening
an output with a verified exact AAudio speaker mask, matched to the actual routed
AudioDeviceInfo ID. Unknown positions fall back to stereo; channel count alone
is insufficient. API-32 mask symbols are optional runtime lookups, preserving
API-29 loading. Recovery does not renegotiate transport; a stereo endpoint uses
an explicit saturating fold-down with LFE omitted.

Diagnostic export uses the existing shared bounded logger, app-private no-backup
files and an explicit filename allowlist. ZIP work runs on the account executor;
SAF save and temporary read-only FileProvider sharing need no broad storage grant.
Some TV images have no document picker or sharing target; the UI reports that.
No new work is placed in the video render/decode path.

Sources: [AAudio reference](https://developer.android.com/ndk/reference/group/audio),
[Using newer NDK APIs](https://developer.android.com/ndk/guides/using-newer-apis),
[Storage Access Framework](https://developer.android.com/training/data-storage/shared/documents-files),
[Secure file sharing](https://developer.android.com/training/secure-file-sharing).
