# EvoParseq architecture and validation

EvoParseq is an independent native Rust Windows client for compatible, unmodified
Parsec hosts. It is not affiliated with Parsec/Unity or lowlat.

## Build and package

The default build is the complete native application:

```powershell
cargo test --locked --manifest-path src-native/Cargo.toml
cargo build --locked --release --manifest-path src-native/Cargo.toml
./scripts/package-native.ps1
```

The Windows SDK/MSVC toolchain and Rust are build-time requirements. The portable
application uses EvoParseq.exe and settings.json; ordinary use does not require
administrator rights. The configured static CRT and bundled Opus remain in place.

## DXGI VRR

Select **Video / Display timing / VRR**, disable Zero copy, save and reconnect.
This uses the existing normal DXGI renderer with all existing decoder backends;
it adds no frame generation, decoded-frame copies or jitter buffer. Unsupported
DXGI flags fall back to the existing timing path. Existing defaults are unchanged.

Enable Adaptive-Sync/FreeSync/G-SYNC in the physical monitor and graphics driver.
For initial testing use borderless fullscreen, the GPU driving that monitor, and
a continuous 60 FPS stream below the display's maximum refresh. Compare against
Low latency using the same scene, codec and network route. Confirm activation
with the driver's VRR indicator or a monitor OSD that reports live refresh;
Windows' configured refresh and Connection stats alone are not proof of VRR.
Low-framerate compensation may make the OSD show a multiple of stream FPS.

Connection stats reports **VRR path enabled** only for accepted DXGI flags and
explicitly leaves panel activation unverified. Event logging records
`video_vrr_configured` and any `video_vrr_fallback`. On a fixed-refresh or disabled
VRR display, allowing tearing may produce tearing. Zero copy retains its separate
Windows Presentation API timing; no host changes or custom host messages are used.

## Native components

- `native_core/account.rs`: account sign-in, optional two-factor authentication,
  computer discovery and session management. Remembered sessions are protected
  using Windows CurrentUser DPAPI in settings.json.
- `native_core/sign_in.rs`: window-API-independent sign-in/sign-out request
  lifecycle, busy/error state and remembered-sign-in preference. Password and
  optional 2FA code remain zeroizing request-owned values, without a retry cache.
  The common controller owns acceptance/persistence decisions, logout/expiration
  cleanup and close-time revocation policy. Its SessionStore implementation in
  `profile.rs` reuses CurrentUser DPAPI and the existing settings transaction;
  the native view only displays typed notices. Authentication APIs, revocation,
  storage format, notice ordering and polling cadence are unchanged.
- `settings_model.rs`: shared editable Settings state, numeric/URL form parsing,
  tab notices, option availability and save feedback. The native adapter reads
  controls and applies the model's availability; no validation rules depend on
  HWND or control IDs. `connection_settings::Manager` retains domain validation,
  DPAPI, atomic persistence and merging of concurrent native-profile changes.
  Disabled preferences and hidden provider fields remain available for later use;
  an unsupported 4:4:4 vendor is still cleared by the existing eligibility policy.
- `stats_view.rs`: presentation of the existing telemetry snapshots: cards,
  diagnostic rows, units, stale/unknown values, scope notes and compact Session
  metrics. Win32 retains text measurement, GDI drawing, scrolling and window
  lifetime. The existing 1 Hz refresh, rate calculation, resource queries and
  generation-scoped host-timing snapshots are unchanged; the presentation model
  performs no sampling, transport requests or video-frame work.
- `native_core/picker.rs`: window-API-independent computer list and discovery
  controller. It keeps one refresh in flight, distinguishes manual/background
  results, preserves selection by peer ID and avoids updates for identical lists.
  Cancelled replies cannot update a new account/view. The Win32 adapter retains
  its existing 30-second timer and 80/500 ms completion polling; discovery uses
  the same per-request HTTP worker, endpoint, validation and timeouts. Sign-in,
  two-factor authentication and protected-session storage remain unchanged.
- `native_core/signaling.rs` and `transport.rs`: authenticated session signaling
  and explicit BUD/WebRTC selection. Failure never silently changes transport.
- `native_core/session.rs`: session coordination uses desktop/runtime interfaces
  for fullscreen, input capture requests, cancellation, wake locks, clipboard,
  events and existing backend operations. Common event payloads live in
  `session_model.rs`; `window::Event` re-exports the same type, without conversion
  or a second queue. Local shortcut releases, clipboard deduplication and
  permission gating share controller tests without requiring a window.
- `native_core/session_platform.rs`: Windows adapter delegates to the existing
  Window, input/cursor handling, clipboard service, transport and WASAPI output.
  Its lifetime preserves end-session / stop / close teardown on success and
  error. The coordinator uses static dispatch; the existing event FIFO, worker
  wakeups, control/audio batch limits and 100 ms control-loop wait are unchanged.
  This separates desktop effects, not the Windows media backends themselves;
  decoding, GPU presentation, focus-release and relative-camera handling retain
  their existing implementation and no additional frame copy or worker is added.
- `input_windows.rs`: Win32 scan codes, hook messages, signed mouse coordinates,
  wheels, UTF-16 characters and Raw Input are translated into the existing
  common events. Keyboard publication retains its pressed-key lock/order and
  hook-time modifiers; callbacks write synchronously into the same bounded FIFO,
  with no intermediate event representation, queue or worker. `input.rs` retains
  platform-independent text decoding. Capture, focus/release order, cursor warp
  suppression, HID absolute synchronization and system-key default handling
  remain in the existing Window backend. The isolated immersive test covers
  immediate relative-motion reversal on both axes and ignores absolute reports.
- `attempt.rs`: native ICE/DTLS/SCTP and data channels for WebRTC.
  The worker-completion condition variable is retained; obsolete test receive
  wrappers and their unobserved receipt notifications have been removed.
- `crates/bud`: isolated BUD carrier, protocol and compressed-media handoff.
  Both transports use the same native video/audio workers and input handling.
- `crates/video`: GPU decoder backends and original-surface presentation.
  H.264 uses Media Foundation. Normal HEVC uses NVIDIA direct D3D11, AMD AMF (4:2:0), or Intel VPL (4:2:0/4:4:4), at 8 or 10 bits. Media Foundation is the compatibility fallback.
- `video_windows.rs`: decoder surfaces feed the existing D3D11 video processor
  directly, followed by DXGI presentation. No CPU pixel readback or intermediate
  decoded-frame export/copy is added. Original NV12/P010 presentation uses Windows
  Presentation API and explicit surface leases.
- `audio_stream.rs` and `audio_windows.rs`: bounded decoded PCM queues, reusable
  buffers and native WASAPI playback. Stereo Opus works on both transports;
  BUD can also request raw PCM and experimental 5.1/7.1 audio. Multichannel
  support has synthetic test coverage only.
- `window.rs`, `input_state.rs`, `native_core/session_ui.rs`: Win32/GDI UI,
  immersive capture, relative camera input and local controls. GPU video remains
  separate from UI painting. HID mouse/keyboard share one bounded BUD device
  lifecycle; ordinary Parsec input for a selected HID device is suppressed.
- `native_core/session_model.rs`: window-API-independent Session commands and
  shared host-video/display/pending state. The Win32 panel reads typed host
  preferences and uses the common saved/host/default selection rules. Panel field
  parsing, resolution labels and status text also live here; numeric limits reuse
  the Video profile's shared bounds. Only widget-index/native-message translation
  remains in the Win32 view. Existing
  window events and notifications still deliver commands and view updates;
  there is no additional queue, thread, frame processing or dependency. This
  is an initial UI separation, not a complete cross-platform application port.
- `native_core/session_video.rs`: host-settings control depends on the small
  `session_model::View` interface (state, client display size and typed status/
  display notifications), rather than the Win32 Window or Session panel. The
  Win32 adapter alone translates these notifications to its existing messages.
  Transport and input/media services retain their existing platform adapters.
- Settings, cursor, clipboard, diagnostics and optional event logging are native.
  Logging excludes continuous mouse/key activity and authentication secrets.

## Session behavior

The computer picker refreshes in the background without rebuilding unchanged
controls. Its draggable session icon opens resolution, bandwidth, FPS, pacing,
orientation, immersive mode and fullscreen controls. Video options are host
requests, not local frame duplication. Applied host settings and actual decoded
formats are reported separately. A host service and authorization are required
for the secure-attention request; portable hosts may not execute it.

UI, decoder, route and audio state are generation-scoped. Reconnect/cancellation
retires the previous attempt; original-surface presentation retains decoder
allocations until Windows releases them. Queue pressure may skip display updates
but must not discard reference dependencies or create unbounded latency.

Immersive mouse confinement follows the Windows cursor hotspot (the click
position); the cursor image can extend beyond the right/bottom edge. No cursor-
size inset is applied, so remote edge pixels remain reachable. Relative camera
input retains its existing Raw Input and one-pixel-anchor behavior. CI runs the
native capture lifecycle separately to prevent other UI tests stealing focus.
Destroying a stream window also terminates its own message loop, releasing the
UI thread's window reference instead of leaving it blocked in GetMessage.

## Validation scope

CI runs native unit/integration tests, GPU surface ownership/metadata tests,
BUD control/HID regressions and Win32 UI captures. Offline/synthetic tests do
not establish physical GPU capability, screen scanout, HDR or host compatibility.
Current live validation is summarized below and in the root README.

## Experimental 4:4:4 and WebRTC

The WebRTC account adapter uses the native Rust ICE/DTLS/SCTP
engine and its pinned channel/initialization contract, using native signaling.
It still needs live direct, TURN/UDP, TURN/TLS and TURN/TCP verification in this
independent build. BUD-only HID and raw/multichannel audio stay BUD-only.

Prefer 4:4:4 is available on NVIDIA and Intel client GPUs (experimental).
Intel 8-bit and combined 10-bit 4:4:4 playback are not live-validated. Intel now uses VPL AYUV/Y410 output, while NVIDIA uses direct D3D11.
It offers 8-bit HEVC after selected-GPU Main444/AYUV decoding and processor
input checks. With 10-bit HEVC also enabled, it checks Main10 4:4:4/Y410 decode,
Y410-to-RGB10 conversion and the existing Main/Main10 4:2:0 fallback rows. Existing pinned lowlat DXVA range-extension
parameters are reused in the GPU-only direct decoder. AYUV goes directly to the
existing conversion/presentation pass, with no CPU pixel readback or intermediate
video copy. This mode uses normal GPU rendering, not original-NV12 zero copy;
The two preferences can be combined; the host remains responsible for the
actual stream format. A 10-bit 4:4:4 surface uses the same single processor
write directly into the RGB10 swapchain, with no intermediate RGB texture.
Live 1920x1080 BUD/TURN-UDP playback of this combined mode was confirmed on
NVIDIA GeForce RTX 3060 on 2026-10-25: both request flags were declared
(`flags=27`), decoded output was 10-bit 4:4:4 and RGB output was 10-bit SDR,
with no decoder fallback/error recorded in that attempt. This validates that
configuration, not other GPUs/transports, physical panel depth or HDR. A rejected Y410/RGB10
conversion is an error, rather than a silent retry in RGB8. The host can still send
4:2:0; actual decoded chroma is shown separately in Connection stats. Live BUD
8-bit 4:4:4 decoding/presentation has been confirmed on NVIDIA GeForce RTX 3060.
The tested AMD Radeon 780M driver rejects the Main444 profile query with
`0x80070057`, so that configuration advertises 4:2:0 only. This is not a blanket
claim about all AMD hardware or APIs. AMD 4:4:4 remains disabled; Settings
clears and disables the checkbox for AMD/unknown adapters, including Automatic
when Windows selects one. NVIDIA and Intel still need actual decoder and
processor support; vendor eligibility alone does not advertise 4:4:4. The
connection capability check independently enforces this policy. Other NVIDIA
configurations and all Intel configurations remain unverified by live tests.

See stock settings comparison for existing
features and possible future additions; unimplemented options were not added.

For a requested Main 10 connection, a host encoder error `-15002` before the
first decoded frame displays a message asking the user to disable 10-bit HEVC
in Settings > Video and reconnect. The client does not automatically retry
in 8-bit or change saved preferences. Other host errors retain their numeric
status and the existing error handling.

In the native build, Connection stats is an independent top-level window so
normal activation can place it in front of or behind the stream. Session stays
owned by the stream. Closing the stream explicitly closes its stats window.

HEVC format changes are handled before compressed input reaches the old GPU
decoder. The client rebuilds the selected decode/presentation path, retains
current parameter sets and the arriving keyframe, and requests a new keyframe
only when needed. A size change alone does not disable vendor decoding or
zero-copy, and does not consume the decoder-error retry limit. Live validation
with remote hosts and the affected GPU paths is still required.

Full-chroma requests are retained on BUD per-stream encoder updates as well as
initialization and native keyframe requests. Capability logs distinguish
`chroma_444_requested` from the effective `chroma_444` declaration. When a
requested AYUV path is unavailable, `video_chroma_444_unavailable` records the
bounded GPU probe stage/result. Decoder state logs include the actual adapter,
decoded chroma and bit depth; no media data or continuous input is recorded.

The common video processor accepts AYUV decoder textures directly, alongside
NV12/P010. This fixes an initial 4:4:4 integration bug where the renderer's old
texture guard rejected otherwise accepted packed 4:4:4 input. No CPU repacking,
intermediate decoded texture or additional color conversion pass is introduced.
The regression test validates the format/array/geometry boundary without
claiming successful hardware 4:4:4 decoding. Actual GPU/driver output still needs
live confirmation. Failures record the vendor fallback reason for diagnosis.


## Host display selection

The Session menu queries the stock host's output list (application sub-ID 10,
reply 12) and current owner configuration (9, reply 11). Selecting a listed
physical or virtual output fetches both the current output list and fresh settings,
waits for both replies, revalidates the selected ID, changes `video[0].output`,
releases the previous output's forced size (`resolutionX/Y: 0`), and sends the
complete preserved configuration (11). The write is sent once;
read-only queries confirm the selected identity for up to five seconds. Other
video slots and unknown host settings are preserved. Refresh handles output
hotplug; a missing/invalid list leaves the stream running. Output IDs are neither
persisted as client preferences nor written to the event log.

This selects one existing stream, not simultaneous multiple stream windows or
creation of virtual displays. BUD and WebRTC reuse their existing reliable
control path and unchanged GPU decoding/presentation. No new dependency, image
copy, frame readback or per-frame display query is introduced. Live physical and
virtual multi-output switching still needs validation against a stock host.

HID mouse mode disables the selector in the UI and rejects requests in the
session controller. It keeps the existing primary-monitor-normalized absolute
reports, clamped to 0..32767; camera-relative reports retain their existing path.
HID keyboard alone does not disable switching. On the examined stock Windows
host, the list preserves DXGI adapter/output enumeration order. Both Microsoft
APIs document primary-first enumeration; the host serializes `hostOS: 1`.
The client uses this Windows-specific contract to request the first output when
necessary, preserving all other settings, and pauses HID mouse forwarding until
the reported output agrees. Unknown OS, missing/invalid list or failed
confirmation leaves the mouse paused with a visible explanation; refresh can
retry. Releases and keyboard forwarding remain available. Primary selection is
not inferred from display names, ID syntax or the client's own monitor layout.
See [reference evidence](NATIVE_REFERENCES.md#stock-windows-host-display-order).

This has synthetic/CI coverage; live multi-monitor validation is still needed.
Changing the host's primary monitor or capture configuration outside the client
requires refreshing the list or reconnecting: there is no host topology-change
subscription in the examined contract. The restriction governs this client's
absolute desktop reports and output selector, not physical input by someone at
the host or the operating system's cursor confinement during a game. No
relative-desktop or normal-injection fallback is introduced.


An announced BUD encoder rebuild (metadata or an encoder-generation change)
retires the old compressed queue, decoder and cached codec parameters before the
new keyframe. H.264/HEVC changes across this boundary can reuse the video worker
and presentation window; unannounced codec changes remain rejected. This does
not consume decoder-error recovery attempts or add decoded-frame copies. Initial
video requests wait for the complete host config even when the output-list reply
arrives first. Host configuration logs include display ordinals/counts rather
than private output identities to distinguish configuration confirmation from
successful media decoding.


Display selection returns the session's resolution preference to **Keep host**,
so an old fixed/client size is not automatically forced onto a different output.
The preference is updated in the session menu and saved profile; bandwidth, FPS,
pacing, orientation, immersive mode and other video slots are retained. A custom
size can be requested after selecting the display if that host output supports
it. Applying **Keep host** now explicitly sends the zero-size/no-override request
instead of silently retaining the last fixed request. This releases a client
constraint; live host behavior determines the resulting Windows display mode.
The client does not invent a display's original/maximum size from its name or
restore modes through a new host helper.

Live AirGPU testing identified a host-side limitation on its default 800x600
virtual output: NVENC rejected the capture device, the host returned to the
1920x1080 NVIDIA output on early attempts, and later selected H264 Encoder MFT
for the 800x600 output. A HEVC request could also fall back to H.264 on the host.
The client configuration reply confirms a requested output, not successful
capture from it. This VM result is not a general limitation of virtual displays;
switching between outputs backed by a compatible GPU still needs separate live
validation.

### About and updates

`updater.rs` owns information text, stable-version comparison and bounded public
GitHub metadata checks. `updater_ui.rs` provides dark, modeless Win32 pages,
owned by the launcher or session window. Opening About performs no network work;
opening Updates starts one metadata worker, polled only while that page exists.
No media path, core check, new dependency or installation worker is involved.
Downloads open validated project URLs in the browser; EXE replacement is manual.
See [migration](../MIGRATION.md) for the legacy-compatible transition package.

### AMF conformance-window sizing

The existing HEVC metadata reader retains both coded dimensions and dimensions
after the SPS conformance window. Only the AMF initialization uses visible
width/height, preventing the reported older-runtime failure for e.g. 1088 coded
rows with 1080 displayed rows. Coded limits and SPS checks remain; a changed
visible size also uses the existing format-rebuild/recovery mechanism. Renderer
crop coordinates, GPU surfaces, conversion and low-latency settings are unchanged.
Synthetic metadata and initialization-choice tests cover cropped/uncropped
streams and unchanged Intel/D3D11 size selection. Live older-AMF validation is
still required; the implementation is our existing adapter, not an imported
upstream decoder or dependency upgrade.

### Default vendor HEVC policy

Normal HEVC defaults to direct D3D11 on NVIDIA, AMF on AMD (4:2:0 only), and VPL on Intel. All accept 8/10-bit streams; Intel VPL also accepts 4:4:4 when its runtime and GPU support it. VPL Query and actual texture validation reject format changes, CPU output and cross-device surfaces. New NVIDIA P010 and Intel extended-format operation still need live validation. No extra decoded-frame copy or conversion pass is added.

The obsolete AMF Main10 preference/selector was removed; old JSON values are ignored. The existing Main10 component and visible-size initialization are retained. AMD 4:4:4 remains unavailable. Original-surface presentation supports 8-bit NV12 and optional 10-bit P010, still only 4:2:0.

Failures attempt Media Foundation. For a 4:4:4 compatibility failure, both transports request fresh 4:2:0 with the original bit depth, then use MF for the rest of that connection. The saved preference is unchanged. The request is bounded by existing recovery limits/deadlines; a host refusing the new format fails explicitly. No silent 10-to-8-bit downgrade is performed.

Logs distinguish video_decoder_selected, video_mf_fallback_attempted, video_chroma_fallback_requested and video_mf_fallback_unavailable. Settings are requests/preferences; Connection stats describes the actual stream and decoder.

### Settings mode selectors and AMF validation

The native Settings page groups automatic / direct-only / forced-TURN routing,
H.264 / HEVC 8-bit / HEVC 10-bit into one video
format selector, and standard / HID mouse / HID keyboard / HID both into one
input selector. Presentation, chroma and audio modes use non-editable choices.
Shared Rust models map these to the existing JSON flags, including dormant
HEVC/HID preferences; the native controls retain their keyboard/focus behavior.
No decoder/presentation/transport/input data path changes accompany this UI work.

The user live-confirmed AMF Main10 on Radeon 780M: HEVC Main10, 10-bit 4:2:0
D3D11 output, RGB10 SDR and zero local dropped frames in the supplied snapshot.
AMF-versus-MF performance remains unmeasured; AMF Main10 is now the default AMD route.
For the separate TCP QoS feasibility result and measurement limits, see
[Windows TCP/TLS QoS](windows-tcp-qos.md).

### Client-sized remote cursors

Remote cursor PNGs retain their host shape, but their canvas fits the client's
current Windows arrow cursor bitmap size, preserving aspect ratio and scaling
and clamping the hotspot with it. `GetIconInfo`/`GetObjectW` query the actual
system cursor bitmap rather than assuming 32 pixels or changing process DPI
awareness. The existing bounded cursor image path performs resizing only when
a new shape arrives or Windows cursor/display/DPI settings change. The current
source cursor is retained for those notifications; unchanged target geometry
reuses its HCURSOR. No video-frame, decoder, GPU, timer, input coordinates,
relative-mode or HID-control changes are involved.

An unsuccessful native cursor creation keeps the last usable cursor. Explicit
image resets still restore the normal system arrow. Relative/camera capture
remains authoritative for hiding the local cursor; a resize never reveals a
cursor hidden by that mode. Existing native cursor-retirement ownership remains.

With event logging enabled, `cursor_image_applied` contains source, client
reference, output dimensions and hotspot; `cursor_mode_changed` records only
stream ID and relative/hidden/suppressed transitions. No continuous positions, PNG pixels,
keys or video are recorded. These distinguish a local image size issue from a
host mode transition. A cursor already composited into host video cannot be
resized independently by this client-side cursor path; that case still needs
live confirmation and must not be hidden with an invented camera-mode rule.

Documented APIs: [GetIconInfo](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-geticoninfo),
[CreateIconIndirect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createiconindirect).

### Immersive keyboard and local shortcuts

Keyboard immersion uses the existing focus-scoped WH_KEYBOARD_LL hook for all
supported physical keys, including side-specific modifiers. One posted Win32
message path preserves modifier/chord/release order; normal and HID transports
reuse the same native input events. No additional input worker or application
queue is introduced, and video/audio workers are unchanged.

Shared Rust shortcut policy reserves the documented exact client combinations.
The native adapter consumes the local trigger's repeats and release and emits
one typed controller action. It does not reinterpret a remotely held key as a
local command on repeat. Extra Alt/Windows modifiers leave game combinations
remote. F8/F11 and Ctrl+comma follow the remote path during keyboard immersion;
local controls and ordinary Windows handling remain available after releasing
input or opening menus. Capture generations reject stale posted keys after
focus/capture changes. Detaching input clears host and cached input modifiers.

Ctrl+Alt+Del remains a Windows secure sequence; Ctrl+Alt+End uses the existing
host-service request. This does not add host patches or injected client input.
Hook/API behavior follows Microsoft's documented
[LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc).

Capture startup queries only eight sided modifiers, two lock keys and the seven
physical client-shortcut triggers (at most 17 asynchronous reads, rather than
the previous 264). Shortcut scans are translated with the current Windows
keyboard layout; existing held-key repeat suppression and lock state remain.
Other keys are learned from foreground events. Local shortcut routing also
avoids extra modifier queries for ordinary legacy keys, repeats and releases.
There is no full-keyboard polling loop, background sampler or per-frame work.
The original Defender report remains unresolved; this cleanup does not establish
the cause of its behavioral classification or guarantee future scan results.

### Original P010 presentation

HEVC Main10 4:2:0 can use the same original-surface presenter as 8-bit NV12.
The setup checks D3D11_FEATURE_DISPLAYABLE plus P010's optional
D3D11_FEATURE_FORMAT_SUPPORT2 / D3D11_FORMAT_SUPPORT2_DISPLAYABLE support.
The direct decoder also requires a Main10/P010 profile and an independent-texture
configuration before creating the DPB. It decodes into the displayable P010
textures themselves; the existing presentation leases protect those allocations.
There is no application pixel copy, CPU readback, RGB intermediate, conversion
pass, new queue/thread or per-frame capability probe. 4:4:4 remains excluded.
A missing capability, allocation/decode/presentation error uses the existing
logged MF fallback, retaining the actual Main10 bit depth. Original-surface
status reports P010 and 10-bit decoded output; final Windows composition,
scanout, display depth and HDR are not inferred. Normal rendering is unchanged.
See [Microsoft displayable surfaces](https://learn.microsoft.com/en-us/windows/win32/direct3d11/displayable-surfaces).

The maintainer live-confirmed HEVC Main10 4:2:0 P010 Zero copy on the tested AMD
and NVIDIA systems. This confirms those tested combinations, not every GPU or
driver; runtime capability checks and MF fallback remain necessary.
Under the current unified policy, Intel HEVC variants (including VPL 8-bit)
have not been live-tested: the available Intel test machine has HEVC disabled.
Earlier hardware observations do not validate the current extended VPL paths.

## Windows multi-display development build

The session overlay's **Add screen** menu requests a second or third host display
on the existing connection (BUD or WebRTC). The initial implementation exposes
the host's stream slots 1/2, corresponding to its second/third listed outputs.
Use extended host displays and normal Parsec mouse/keyboard input. Already-open
outputs are not opened twice. Two-screen playback has been observed in a live
Windows test; independent resolution changes and activating clicks need live
validation in this build. Host acceptance, including account restrictions, is
not guaranteed.

Each additional window uses the existing D3D11 decoder/presenter once for its
stream, with its own bounded reference/queue state. No decoded readback, screen
composition or additional decoded-image copy is introduced. One audio pipeline
and one connection remain active. Additional windows use the draggable session
icon for independent Resolution/Apply, Fullscreen, Release/Capture input and
Close screen; the main window owns session settings and Disconnect. Connection
stats is disabled on additional windows. Resolution writes refresh the complete
host configuration, patch only the selected stream's dimensions and confirm that
slot through the existing bounded readback transaction. Other stream settings
and unknown host fields are preserved; changes are serialized across windows.
Normal absolute mouse clicks send the target stream/position before button-down
on the same ordered input channel. Relative camera input and HID do not receive
an extra absolute warp. Closing an extra window sends only its stream-disable request. If
the host rejects it or no picture is decoded within ten seconds, that window
closes and the event log records its stream/status. Existing presentation-mode
costs and the project's strict Zero copy definition remain unchanged.

Android/Linux frontends do not enable this feature. The small vendored Rust
formatting changes accompanying the Windows backend validation are mechanical;
their media behavior is unchanged.


### Parsec virtual display prerequisite

The current Windows Add screen implementation opens host-reported outputs; it
does not provision Parsec VDD devices. Set the desired count in the stock host's
**Settings > Host > Virtual Displays**, then reconnect and refresh the display
list before opening another stream. **Fallback To Virtual Display** is a
separate single-output feature and is insufficient for multiple virtual screens.
The host's own driver and account/owner policy still apply. No guest-side VDD
count-setting contract has been established; see `NATIVE_REFERENCES.md` for the
specific stock-runtime evidence and its limits. Do not add a speculative VDD
count field to the existing remote video configuration.

### Multi-display cursor capture recovery

A visible, unsuppressed absolute cursor on another live stream invalidates an
old stream's hidden/relative cursor request. Clear that stale request without
changing focus or releasing held buttons/keys. Restore the pointer once through
the destination stream's current viewport and client-to-screen transform, with
the existing Disable cursor warp preference respected. Never reuse the old
display's coordinates. Both possible synthetic-move recipients (source capture
and destination) suppress the warp echo, including negative client coordinates.
The destination is held weakly and must remain live/visible; closed/minimized
windows do not receive a restoration. This lets cross-display window drags return from relative capture
when the host reports the visible cursor on the destination stream. Hidden or
relative updates retain the existing SDK/lowlat capture rule; suppressed cursors
never prove a visible endpoint. Closed/unknown streams do not trigger recovery.
The reconciliation is transition-only, uses the existing UI messages and logs
`cursor_capture_released_for_peer` with source/destination stream IDs. No media
path, timer, thread or dependency is added.
