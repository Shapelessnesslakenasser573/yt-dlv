//! InnerTube client definitions. YouTube returns different formats (and
//! different signature requirements) per client, so we query several and merge.
//! Versions/keys here mirror the long-standing public InnerTube values yt-dlp
//! uses; they drift over time and are the first thing to bump when extraction
//! regresses.

/// A single InnerTube client context.
#[derive(Debug, Clone, Copy)]
pub struct InnerTubeClient {
    /// Our key, e.g. `"web"`.
    pub key: &'static str,
    /// `context.client.clientName`, e.g. `"WEB"`.
    pub client_name: &'static str,
    pub client_version: &'static str,
    /// `X-YouTube-Client-Name` numeric id.
    pub client_id: u32,
    pub user_agent: &'static str,
    /// Public InnerTube API key for this client.
    pub api_key: &'static str,
    /// Whether formats from this client require running the JS player
    /// (signature / n descrambling).
    pub requires_player: bool,
    /// Optional `context.client.deviceModel`.
    pub device_model: Option<&'static str>,
    /// Optional `context.client.osName` / `osVersion`.
    pub os_name: Option<&'static str>,
    pub os_version: Option<&'static str>,
}

const WEB: InnerTubeClient = InnerTubeClient {
    key: "web",
    client_name: "WEB",
    client_version: "2.20260114.08.00",
    client_id: 1,
    user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    api_key: "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8",
    requires_player: true,
    device_model: None,
    os_name: Some("Windows"),
    os_version: Some("10.0"),
};

const IOS: InnerTubeClient = InnerTubeClient {
    key: "ios",
    client_name: "IOS",
    client_version: "21.02.3",
    client_id: 5,
    user_agent: "com.google.ios.youtube/21.02.3 (iPhone16,2; U; CPU iOS 18_2_1 like Mac OS X;)",
    api_key: "AIzaSyB-63vPrdThhKuerbB2N_l7Kwwcxj6yUAc",
    requires_player: false,
    device_model: Some("iPhone16,2"),
    os_name: Some("iOS"),
    os_version: Some("18.2.1.22C161"),
};

const ANDROID_VR: InnerTubeClient = InnerTubeClient {
    key: "android_vr",
    client_name: "ANDROID_VR",
    client_version: "1.65.10",
    client_id: 28,
    user_agent: "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; \
                 eureka-user Build/SQ3A.220605.009.A1) gzip",
    api_key: "AIzaSyA8eiZmM1FaDVjRy-df2KTyQ_vz_yYM39w",
    requires_player: false,
    device_model: Some("Quest 3"),
    os_name: Some("Android"),
    os_version: Some("12L"),
};

const TV: InnerTubeClient = InnerTubeClient {
    key: "tv",
    client_name: "TVHTML5",
    client_version: "7.20260114.12.00",
    client_id: 7,
    user_agent: "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/Version",
    api_key: "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8",
    requires_player: true,
    device_model: None,
    os_name: None,
    os_version: None,
};

/// All known clients.
pub const ALL: &[InnerTubeClient] = &[WEB, IOS, ANDROID_VR, TV];

/// Default clients to try, in order. `ios`/`android_vr` often yield directly
/// usable URLs without JS; `web` exercises the full sig/n path.
pub const DEFAULT_ORDER: &[&str] = &["ios", "android_vr", "web", "tv"];

pub fn by_key(key: &str) -> Option<InnerTubeClient> {
    ALL.iter().copied().find(|c| c.key == key)
}
