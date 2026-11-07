// Shared desktop form descriptors. Values/actions are supplied by platform adapters.
#[derive(Clone, Copy)]
pub enum Kind {
    Edit,
    Choice,
    Check,
    Info,
}
#[derive(Clone, Copy)]
pub struct SettingRow {
    pub tab: usize,
    pub group: u8,
    pub label: &'static str,
    pub help: &'static str,
    pub y: i32,
    pub help_height: i32,
    pub field_height: i32,
    pub kind: Kind,
    pub control_text: &'static str,
}
pub const STUN_SERVERS:SettingRow=SettingRow{tab:0,group:0,label:"STUN servers",help:"Discover a direct route. One stun: URL per line.\nLeave blank to use the default discovery server.",y:242,help_height:48,field_height:76,kind:Kind::Edit,control_text:""};
pub const CONNECTION_ROUTE:SettingRow=SettingRow{tab:0,group:0,label:"Connection route",help:"Choose automatic routing, direct-only or TURN relay.\nDirect-only is intended for LAN or VPN connections.",y:344,help_height:40,field_height:32,kind:Kind::Choice,control_text:""};
pub const TURN_PROVIDER: SettingRow = SettingRow {
    tab: 0,
    group: 0,
    label: "TURN provider",
    help: "An optional relay when a direct connection is unavailable.",
    y: 418,
    help_height: 40,
    field_height: 32,
    kind: Kind::Choice,
    control_text: "",
};
pub const TURN_SERVERS:SettingRow=SettingRow{tab:0,group:1,label:"TURN servers",help:"coturn, eturnal, ExpressTURN and other providers.\nUDP, TCP or TLS; e.g. turns:host:443. One URL per line.",y:502,help_height:48,field_height:76,kind:Kind::Edit,control_text:""};
pub const USERNAME: SettingRow = SettingRow {
    tab: 0,
    group: 1,
    label: "Username",
    help: "The username supplied by your TURN provider.",
    y: 608,
    help_height: 48,
    field_height: 36,
    kind: Kind::Edit,
    control_text: "",
};
pub const PASSWORD: SettingRow = SettingRow {
    tab: 0,
    group: 1,
    label: "Password",
    help: "Leave blank to keep your saved password.",
    y: 688,
    help_height: 48,
    field_height: 36,
    kind: Kind::Edit,
    control_text: "",
};
pub const TURN_KEY_ID: SettingRow = SettingRow {
    tab: 0,
    group: 2,
    label: "TURN Key ID",
    help: "The key ID from your Cloudflare TURN configuration.",
    y: 502,
    help_height: 48,
    field_height: 36,
    kind: Kind::Edit,
    control_text: "",
};
pub const API_TOKEN: SettingRow = SettingRow {
    tab: 0,
    group: 2,
    label: "API token",
    help: "Leave blank to keep your saved token.",
    y: 582,
    help_height: 48,
    field_height: 36,
    kind: Kind::Edit,
    control_text: "",
};
pub const CREDENTIAL_LIFETIME: SettingRow = SettingRow {
    tab: 0,
    group: 2,
    label: "Credential lifetime",
    help: "How long generated TURN credentials remain valid (seconds).",
    y: 678,
    help_height: 48,
    field_height: 36,
    kind: Kind::Edit,
    control_text: "",
};
pub const CREDENTIAL_CACHE: SettingRow = SettingRow {
    tab: 0,
    group: 2,
    label: "Credential cache",
    help: "Reuse valid credentials until you close EvoParseq.",
    y: 758,
    help_height: 24,
    field_height: 32,
    kind: Kind::Check,
    control_text: "Reuse unexpired credentials",
};
pub const VIDEO_GPU: SettingRow = SettingRow {
    tab: 1,
    group: 0,
    label: "Video GPU",
    help:
        "GPU for stream decoding and rendering.\nAutomatic uses Windows' default. Next connection.",
    y: 242,
    help_height: 40,
    field_height: 32,
    kind: Kind::Choice,
    control_text: "",
};
pub const VIDEO_FORMAT:SettingRow=SettingRow{tab:1,group:0,label:"Video format",help:"HEVC and 10-bit require compatible host encoding.\nThe host may select H.264; reconnect after changing.",y:344,help_height:48,field_height:32,kind:Kind::Choice,control_text:""};
pub const DISPLAY_TIMING:SettingRow=SettingRow{tab:1,group:0,label:"Display timing",help:"Experimental: normal GPU rendering; reconnect to apply.\nVRR needs an enabled compatible monitor and driver.\nZero copy keeps Windows-managed timing.",y:626,help_height:60,field_height:32,kind:Kind::Choice,control_text:""};
pub const CHROMA:SettingRow=SettingRow{tab:1,group:0,label:"Chroma",help:"NVIDIA tested; Intel not live-validated. AMD unavailable.\nHEVC 8/10-bit; requires compatible host and GPU.\nNormal GPU rendering; host may send 4:2:0 instead.",y:438,help_height:60,field_height:32,kind:Kind::Choice,control_text:""};
pub const SESSION_MENU: SettingRow = SettingRow {
    tab: 1,
    group: 0,
    label: "Session menu",
    help: "Ctrl+Shift+M opens the menu even when its button is hidden.\nApplies after saving.",
    y: 834,
    help_height: 48,
    field_height: 32,
    kind: Kind::Check,
    control_text: "Hide session button",
};
pub const AI_UPSCALING_EXPERIMENTAL: SettingRow = SettingRow {
    tab: 1,
    group: 0,
    label: "AI upscaling (experimental)",
    help: "",
    y: 924,
    help_height: 64,
    field_height: 32,
    kind: Kind::Choice,
    control_text: "",
};
pub const TRANSPORT:SettingRow=SettingRow{tab:0,group:0,label:"Transport",help:"WebRTC or BUD: direct UDP or TURN over UDP/TLS/TCP.\nUses the TURN URLs above. Reconnect after changing.",y:844,help_height:40,field_height:32,kind:Kind::Choice,control_text:""};
pub const VIDEO_PRESENTATION:SettingRow=SettingRow{tab:1,group:0,label:"Video presentation",help:"HEVC 4:2:0: NV12 (8-bit) or P010 (10-bit).\nRequires driver support; otherwise MF. Reconnect after changing.",y:532,help_height:40,field_height:32,kind:Kind::Choice,control_text:""};
pub const TCP_TLS_QOS:SettingRow=SettingRow{tab:0,group:0,label:"TCP/TLS QoS",help:"Windows outgoing TURN traffic only; network support varies.\nUnsupported or refused: continue unmarked.\nReconnect after changing.",y:844 + 94,help_height:60,field_height:32,kind:Kind::Choice,control_text:""};
pub const EVENT_LOG:SettingRow=SettingRow{tab:0,group:0,label:"Event log",help:"Log connection changes and decoder errors.\nSaved beside settings.json; no credentials or media payloads.\nApplies immediately after saving.",y:938 + 94,help_height:60,field_height:32,kind:Kind::Check,control_text:"Enable event logging"};
pub const PLAYBACK_BUFFER: SettingRow = SettingRow {
    tab: 2,
    group: 0,
    label: "Playback buffer",
    help:
        "Custom limits apply only when enabled.\nOff uses the core's limits (normally 75 / 150 ms).",
    y: 242,
    help_height: 48,
    field_height: 32,
    kind: Kind::Choice,
    control_text: "",
};
pub const MINIMUM_BUFFER_MS:SettingRow=SettingRow{tab:2,group:0,label:"Minimum buffer (ms)",help:"PCM reserve before starting or refilling playback.\nSmaller values can increase audible gaps.",y:344,help_height:48,field_height:36,kind:Kind::Edit,control_text:""};
pub const MAXIMUM_BUFFER_MS:SettingRow=SettingRow{tab:2,group:0,label:"Maximum buffer (ms)",help:"Local PCM plus audio queued on the Windows device.\n0 <= minimum <= maximum <= 500; maximum above zero.",y:438,help_height:48,field_height:36,kind:Kind::Edit,control_text:""};
pub const AUDIO_FORMAT:SettingRow=SettingRow{tab:2,group:0,label:"Audio format",help:"RAW: BUD only; host must also enable RAW.\n48 kHz stereo PCM uses about 1.54 Mbps before overhead.\nThe actual codec is shown in Connection stats.",y:532,help_height:60,field_height:32,kind:Kind::Choice,control_text:""};
pub const SPEAKER_LAYOUT:SettingRow=SettingRow{tab:2,group:0,label:"Speaker layout",help:"Experimental: match the Windows output's 5.1 or 7.1 layout.\nSynthetic tests only; no live host/client validation.\nBUD only; stereo fallback. Reconnect to apply.",y:630,help_height:68,field_height:32,kind:Kind::Choice,control_text:""};
pub const INPUT_MODE:SettingRow=SettingRow{tab:3,group:0,label:"Input mode",help:"Standard input or experimental USB HID over BUD.\nMouse and keyboard can be selected independently.",y:242,help_height:48,field_height:32,kind:Kind::Choice,control_text:""};
pub const HID_REQUIREMENTS:SettingRow=SettingRow{tab:3,group:0,label:"HID requirements",help:"Enable USB/Yubikey passthrough on the host.\nHID mouse uses the primary display only.\nUse the same keyboard layout on host and client.",y:354,help_height:64,field_height:32,kind:Kind::Info,control_text:""};
pub const COMPATIBILITY:SettingRow=SettingRow{tab:3,group:0,label:"Compatibility",help:"Absolute desktop and relative camera movement.\nCamera mode follows the host cursor state.\nKeyboard layout is not switched automatically.",y:466,help_height:64,field_height:32,kind:Kind::Info,control_text:""};
pub const SETTINGS: &[SettingRow] = &[
    STUN_SERVERS,
    CONNECTION_ROUTE,
    TURN_PROVIDER,
    TURN_SERVERS,
    USERNAME,
    PASSWORD,
    TURN_KEY_ID,
    API_TOKEN,
    CREDENTIAL_LIFETIME,
    CREDENTIAL_CACHE,
    VIDEO_GPU,
    VIDEO_FORMAT,
    DISPLAY_TIMING,
    CHROMA,
    SESSION_MENU,
    AI_UPSCALING_EXPERIMENTAL,
    TRANSPORT,
    VIDEO_PRESENTATION,
    TCP_TLS_QOS,
    EVENT_LOG,
    PLAYBACK_BUFFER,
    MINIMUM_BUFFER_MS,
    MAXIMUM_BUFFER_MS,
    AUDIO_FORMAT,
    SPEAKER_LAYOUT,
    INPUT_MODE,
    HID_REQUIREMENTS,
    COMPATIBILITY,
];

pub struct StatRow {
    pub label: &'static str,
    pub tab: usize,
    pub section: bool,
}
pub const STATS_TABS: [&str; 5] = ["Overview", "Network", "Video", "Audio", "Timings"];
pub const STAT_NETWORK: &str = "NETWORK";
pub const STAT_ROUTE: &str = "Route";
pub const STAT_LOCAL_CANDIDATE: &str = "Local candidate";
pub const STAT_REMOTE_CANDIDATE: &str = "Remote candidate";
pub const STAT_TRANSPORT: &str = "Transport";
pub const STAT_TURN_SERVER_IN_USE: &str = "TURN server in use";
pub const STAT_TURN_TRANSPORT: &str = "TURN transport";
pub const STAT_SECURITY: &str = "SECURITY";
pub const STAT_DTLS_STATE: &str = "DTLS state";
pub const STAT_ENCRYPTION: &str = "Encryption";
pub const STAT_HOST_ENCODE_TIME: &str = "Host encode time";
pub const STAT_BUD_DELIVERY: &str = "BUD DELIVERY";
pub const STAT_DUPLICATE_FRAGMENTS: &str = "Duplicate fragments";
pub const STAT_OUTSIDE_RECEIVE_WINDOW: &str = "Outside receive window";
pub const STAT_COMPRESSED_VIDEO_BACKLOG: &str = "Compressed video backlog";
pub const STAT_SKIPPED_ENCODED_PICTURES: &str = "Skipped encoded pictures";
pub const STAT_AUDIO_RECEIVE_POOL_DROPS: &str = "Audio receive pool drops";
pub const STAT_INPUT_QUEUE_DROPS: &str = "Input queue drops";
pub const STAT_VIDEO: &str = "VIDEO";
pub const STAT_CODEC: &str = "Codec";
pub const STAT_PROFILE: &str = "Profile";
pub const STAT_HEVC_ADVERTISED: &str = "HEVC advertised";
pub const STAT_RESOLUTION: &str = "Resolution";
pub const STAT_DECODER: &str = "Decoder";
pub const STAT_DECODER_FALLBACK: &str = "Decoder fallback";
pub const STAT_DECODED_COLOR_DEPTH: &str = "Decoded color depth";
pub const STAT_DECODED_CHROMA: &str = "Decoded chroma";
pub const STAT_RGB_OUTPUT: &str = "RGB output";
pub const STAT_RGB_OUTPUT_FALLBACK: &str = "RGB output fallback";
pub const STAT_RENDERER: &str = "Renderer";
pub const STAT_GPU_ADAPTER: &str = "GPU adapter";
pub const STAT_ZERO_COPY_EXPERIMENTAL: &str = "Zero copy (experimental)";
pub const STAT_DISPLAY_OUTPUT: &str = "Display output";
pub const STAT_VIDEO_STATUS: &str = "Video status";
pub const STAT_HARDWARE_DECODE: &str = "Hardware decode";
pub const STAT_DECODER_OUTPUT: &str = "Decoder output";
pub const STAT_LOCAL_DROPPED_FRAMES: &str = "Local dropped frames";
pub const STAT_VIDEO_AI_UPSCALING: &str = "Video AI upscaling";
pub const STAT_DISPLAY_TIMING: &str = "Display timing";
pub const STAT_DECODED_VIDEO_RATE: &str = "Decoded video rate";
pub const STAT_PRESENT_SUBMISSIONS: &str = "Present submissions";
pub const STAT_PRESENTATION_RATE_SCOPE: &str = "Presentation rate scope";
pub const STAT_LOCAL_VIDEO_TIMING: &str = "LOCAL VIDEO TIMING";
pub const STAT_CPU_TIMING_VALUES: &str = "CPU timing values";
pub const STAT_RECENT_TIMING_WINDOW: &str = "Recent timing window";
pub const STAT_DISPLAY_CATCH_UP_SKIPS: &str = "Display catch-up skips";
pub const STAT_ADAPTIVE_CATCH_UP_ACTIVATIONS: &str = "Adaptive catch-up activations";
pub const STAT_TIMING_SCOPE: &str = "Timing scope";
pub const STAT_AUDIO: &str = "AUDIO";
pub const STAT_CHANNELS: &str = "Channels";
pub const STAT_MEASURED_AUDIO_TRAFFIC: &str = "Measured audio traffic";
pub const STAT_PLAYBACK_CHANNELS: &str = "Playback channels";
pub const STAT_PLAYBACK_BUFFER: &str = "Playback buffer";
pub const STAT_BUFFER_LIMITS: &str = "Buffer limits";
pub const STAT_AUDIO_OUTPUT: &str = "Audio output";
pub const STAT_AUDIO_RECOVERIES: &str = "Audio recoveries";
pub const STAT_AUDIO_UNDERRUNS: &str = "Audio underruns";
pub const STAT_AUDIO_DEVICE_BUFFER: &str = "Audio device buffer";
pub const STAT_DECODED_AUDIO_WAITING: &str = "Decoded audio waiting";
pub const STAT_MEDIA_ENCRYPTION: &str = "Media encryption";
pub const STAT_APP_PERFORMANCE: &str = "APP PERFORMANCE";
pub const STAT_CPU_ALL_CORES: &str = "CPU (all cores)";
pub const STAT_GPU_BUSIEST_ENGINE: &str = "GPU (busiest engine)";
pub const STAT_HOST_TIMING_SECTION: &str = "HOST TIMING";
pub const STAT_HOST_TIMING: &str = "Host timing";
pub const STAT_TIMING_VALUES: &str = "Timing values";
pub const STAT_VIDEO_TIMING: &str = "Video timing";
pub const STAT_AUDIO_PROCESSING: &str = "Audio processing";
pub const STAT_AUDIO_BLOCK_INTERVAL: &str = "Audio block interval";
pub const STAT_AUDIO_TIMING_REPORTS: &str = "Audio timing reports";
pub const STAT_AUDIO_REPORT_AGE: &str = "Audio report age";
pub const STAT_AUDIO_TIMING: &str = "Audio timing";
pub const STAT_INVALID_TIMING_REPORTS: &str = "Invalid timing reports";
pub const STAT_UNKNOWN_TIMING_FORMATS: &str = "Unknown timing formats";
pub const STATS: &[StatRow] = &[
    StatRow {
        label: STAT_NETWORK,
        tab: 1,
        section: true,
    },
    StatRow {
        label: STAT_ROUTE,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_LOCAL_CANDIDATE,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_REMOTE_CANDIDATE,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_TRANSPORT,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_TURN_SERVER_IN_USE,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_TURN_TRANSPORT,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_SECURITY,
        tab: 1,
        section: true,
    },
    StatRow {
        label: STAT_DTLS_STATE,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_ENCRYPTION,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_HOST_ENCODE_TIME,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_BUD_DELIVERY,
        tab: 1,
        section: true,
    },
    StatRow {
        label: STAT_DUPLICATE_FRAGMENTS,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_OUTSIDE_RECEIVE_WINDOW,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_COMPRESSED_VIDEO_BACKLOG,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_SKIPPED_ENCODED_PICTURES,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_RECEIVE_POOL_DROPS,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_INPUT_QUEUE_DROPS,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_VIDEO,
        tab: 2,
        section: true,
    },
    StatRow {
        label: STAT_CODEC,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_PROFILE,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_HEVC_ADVERTISED,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_RESOLUTION,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODER,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODER_FALLBACK,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODED_COLOR_DEPTH,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODED_CHROMA,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_RGB_OUTPUT,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_RGB_OUTPUT_FALLBACK,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_RENDERER,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_GPU_ADAPTER,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_ZERO_COPY_EXPERIMENTAL,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DISPLAY_OUTPUT,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_VIDEO_STATUS,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_HARDWARE_DECODE,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODER_OUTPUT,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_LOCAL_DROPPED_FRAMES,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_VIDEO_AI_UPSCALING,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DISPLAY_TIMING,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_DECODED_VIDEO_RATE,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_PRESENT_SUBMISSIONS,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_PRESENTATION_RATE_SCOPE,
        tab: 2,
        section: false,
    },
    StatRow {
        label: STAT_LOCAL_VIDEO_TIMING,
        tab: 4,
        section: true,
    },
    StatRow {
        label: STAT_CPU_TIMING_VALUES,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_RECENT_TIMING_WINDOW,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_DISPLAY_CATCH_UP_SKIPS,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_ADAPTIVE_CATCH_UP_ACTIVATIONS,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_TIMING_SCOPE,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO,
        tab: 3,
        section: true,
    },
    StatRow {
        label: STAT_CHANNELS,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_MEASURED_AUDIO_TRAFFIC,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_PLAYBACK_CHANNELS,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_PLAYBACK_BUFFER,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_BUFFER_LIMITS,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_OUTPUT,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_RECOVERIES,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_UNDERRUNS,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_DEVICE_BUFFER,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_DECODED_AUDIO_WAITING,
        tab: 3,
        section: false,
    },
    StatRow {
        label: STAT_MEDIA_ENCRYPTION,
        tab: 1,
        section: false,
    },
    StatRow {
        label: STAT_APP_PERFORMANCE,
        tab: 0,
        section: true,
    },
    StatRow {
        label: STAT_CPU_ALL_CORES,
        tab: 0,
        section: false,
    },
    StatRow {
        label: STAT_GPU_BUSIEST_ENGINE,
        tab: 0,
        section: false,
    },
    StatRow {
        label: STAT_HOST_TIMING_SECTION,
        tab: 4,
        section: true,
    },
    StatRow {
        label: STAT_HOST_TIMING,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_TIMING_VALUES,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_VIDEO_TIMING,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_PROCESSING,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_BLOCK_INTERVAL,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_TIMING_REPORTS,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_REPORT_AGE,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_AUDIO_TIMING,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_INVALID_TIMING_REPORTS,
        tab: 4,
        section: false,
    },
    StatRow {
        label: STAT_UNKNOWN_TIMING_FORMATS,
        tab: 4,
        section: false,
    },
];

pub const MENU_SETTINGS: &str = "Settings";
pub const MENU_STATS: &str = "Connection stats";
pub const MENU_HELP: &str = "Help";
pub const MENU_ABOUT: &str = "About EvoParseq";
pub const MENU_UPDATES: &str = "Check for updates";
pub const SESSION_LABELS: [&str; 6] = [
    "Resolution",
    "Bandwidth (Mbps)",
    "Frame rate (FPS)",
    "Frame pacing",
    "Orientation",
    "Immersive mode",
];
pub const SESSION_ACTIONS: [(&str, [i32; 4]); 6] = [
    ("Settings", [24, 838, 178, 36]),
    ("Apply", [214, 838, 178, 36]),
    ("Disconnect", [24, 888, 178, 36]),
    ("Close", [214, 888, 178, 36]),
    ("Hide button", [24, 938, 178, 36]),
    ("Send Ctrl+Alt+Del", [214, 938, 178, 36]),
];
pub const SESSION_METRICS: &str = "Decode CPU N/A | Encode N/A\nRTT N/A | RX N/A | D/P N/A/N/A fps";

pub const STAT_SAMPLE_RATE: &str = "Sample rate";

pub const STAT_MEMORY_RESIDENT_TOTAL: &str = "Memory resident (total)";

pub const STAT_MEMORY_RESIDENT_PRIVATE: &str = "Memory resident (private)";

pub const STAT_MEMORY_COMMITTED_PRIVATE: &str = "Memory committed (private)";

pub fn additional_stats(tab: usize) -> Vec<String> {
    let mut rows = Vec::new();
    match tab {
        0 => rows.extend(
            [
                STAT_MEMORY_RESIDENT_TOTAL,
                STAT_MEMORY_RESIDENT_PRIVATE,
                STAT_MEMORY_COMMITTED_PRIVATE,
            ]
            .map(str::to_owned),
        ),
        1 => {
            for name in ["Control", "Video", "Audio"] {
                rows.push(format!("{name} late fragments"));
                rows.push(format!("{name} NACK requests"));
            }
        }
        3 => rows.extend([STAT_CODEC, STAT_SAMPLE_RATE].map(str::to_owned)),
        4 => {
            rows.extend(
                crate::video_output::LocalTiming::default()
                    .stages()
                    .map(|(label, _)| label.to_owned()),
            );
            rows.extend(
                [
                    "Video processing",
                    "Capture segment",
                    "Host encode",
                    "Video frame interval",
                    "Video timing reports",
                    "Video report age",
                ]
                .map(str::to_owned),
            );
        }
        _ => {}
    }
    rows
}
