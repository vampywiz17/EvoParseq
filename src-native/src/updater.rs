//! Shared EvoParseq information and metadata-only stable release checks.
//! Downloads and installation are performed by the user through their browser.
use serde::Deserialize;
use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};

const API: &str = "https://api.github.com/repos/vampywiz17/EvoParseq/releases/latest";
pub const REPOSITORY: &str = crate::about::REPOSITORY;
const MAX_METADATA: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    Current,
    Available,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub phase: Phase,
    pub message: &'static str,
    pub release: Option<Available>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            message: "Ready to check for a newer stable GitHub release.",
            release: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub version: String,
    pub notes: String,
    pub download_url: String,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Debug)]
enum CheckResult {
    Current,
    NotPublished,
    Available(Available),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    About,
    Updates,
}
impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Self::About => crate::about::TITLE,
            Self::Updates => crate::about::UPDATES_TITLE,
        }
    }
    pub fn heading(self) -> &'static str {
        match self {
            Self::About => crate::about::HEADING,
            Self::Updates => crate::about::UPDATES_HEADING,
        }
    }
}
const RTX_CREDIT: &str = if cfg!(nvidia_vsr_sdk) {
    "Optional NVIDIA RTX Video SDK 1.1.0 / NVIDIA RTX technology. The optional NVIDIA runtime and linked SDK library are proprietary and excluded from the EvoParseq MIT license; see NVIDIA-RTX-VIDEO-LICENSE.pdf in the download package. VSR may increase latency, GPU load and memory use."
} else {
    ""
};
pub const MIGRATION: &str = "Upgrading from ParsecWebTurn: close the old application and keep settings.json in the same folder. Replace the executable using the transition ZIP, or use EvoParseq.exe from the normal ZIP. Network, video, audio and input settings are retained. Sign in again: the old web-core sign-in cannot be imported. Original files are not deleted automatically.";
pub struct Content {
    pub version: String,
    pub status: String,
    pub notes: String,
}
pub fn content(mode: Mode, view: &View) -> Content {
    if mode == Mode::About {
        let info = crate::about::content(
            env!("CARGO_PKG_VERSION"),
            "Windows x64",
            RTX_CREDIT,
            MIGRATION,
        );
        return Content {
            version: info.version,
            status: info.status,
            notes: info.notes,
        };
    }
    Content {
        version: format!("Installed: v{}{}", env!("CARGO_PKG_VERSION"), view.release.as_ref().map(|r| format!("\r\nAvailable: {} (stable)",r.version)).unwrap_or_default()),
        status: view.message.into(),
        notes: view.release.as_ref().map(|r| format!("{}\r\n\r\n{MIGRATION}",r.notes.replace("\r\n", "\n").replace('\n', "\r\n"))).unwrap_or_else(|| format!("Stable EvoParseq releases are checked on GitHub. Downloads open in your browser; this application does not replace files or install updates automatically.\r\n\r\nRelease channel: {REPOSITORY}\r\n\r\n{MIGRATION}")),
    }
}
/// Only fixed project links and strictly validated stable EvoParseq ZIP URLs
/// may be passed to the platform browser launcher.
pub fn allowed_url(url: &str) -> bool {
    if url == REPOSITORY {
        return true;
    }
    let Some(tail) = url.strip_prefix(&format!("{REPOSITORY}/releases/download/")) else {
        return false;
    };
    let Some((tag, name)) = tail.split_once('/') else {
        return false;
    };
    if stable_version(tag).is_err() {
        return false;
    }
    let version = tag.strip_prefix('v').unwrap_or(tag);
    name == format!("EvoParseq-v{version}-win64.zip")
}

#[derive(Default)]
pub struct State {
    view: Mutex<View>,
}
impl State {
    pub fn view(&self) -> View {
        self.view.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn check(self: &Arc<Self>) {
        self.check_with(latest);
    }
    fn check_with(
        self: &Arc<Self>,
        fetch: impl FnOnce() -> Result<CheckResult, &'static str> + Send + 'static,
    ) {
        {
            let mut view = self.view.lock().unwrap_or_else(|e| e.into_inner());
            if view.phase == Phase::Checking {
                return;
            }
            *view = View {
                phase: Phase::Checking,
                message: "Checking the latest stable GitHub release...",
                release: None,
            };
        }
        let state = self.clone();
        if std::thread::Builder::new().name("evoparseq-update-check".into()).spawn(move || {
            let result = fetch();
            let mut view = state.view.lock().unwrap_or_else(|e| e.into_inner());
            *view = match result {
                Ok(CheckResult::Available(release)) => View { phase: Phase::Available,
                    message: "A newer EvoParseq stable version is available. Download the ZIP in your browser, then close the app and replace the EXE manually.", release: Some(release) },
                Ok(CheckResult::Current) => View { phase: Phase::Current,
                    message: "No newer EvoParseq stable release is available. Development builds may be ahead of the latest stable release.", release: None },
                Ok(CheckResult::NotPublished) => View { phase: Phase::Current,
                    message: "No EvoParseq stable package has been published in this release channel yet. ParsecWebTurn packages are not offered as EvoParseq updates.", release: None },
                Err(message) => View { phase: Phase::Error, message, release: None },
            };
        }).is_err() {
            *self.view.lock().unwrap_or_else(|e| e.into_inner()) = View {
                phase: Phase::Error, message: "Cannot start the update check. Try again later.", release: None,
            };
        }
    }
}

fn stable_version(value: &str) -> Result<[u64; 3], &'static str> {
    let mut parts = value.strip_prefix('v').unwrap_or(value).split('.');
    let mut numbers = [0; 3];
    for number in &mut numbers {
        *number = numeric(parts.next().ok_or("Invalid release version.")?)?;
    }
    if parts.next().is_some() {
        return Err("Invalid release version.");
    }
    Ok(numbers)
}
fn numeric(value: &str) -> Result<u64, &'static str> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("Invalid release version.");
    }
    value.parse().map_err(|_| "Invalid release version.")
}
fn newer_stable(candidate: &str, installed: &str) -> Result<bool, &'static str> {
    let (base, development) = match installed.split_once("-dev.") {
        Some((base, sequence)) if numeric(sequence)? > 0 => (base, true),
        Some(_) => return Err("Invalid installed version."),
        None => (installed, false),
    };
    let candidate = stable_version(candidate)?;
    let installed = stable_version(base)?;
    Ok(candidate > installed || (candidate == installed && development))
}
fn download_url(release: &Release) -> Result<String, &'static str> {
    stable_version(&release.tag_name)?;
    if release.draft || release.prerelease {
        return Err("Only published stable releases are supported.");
    }
    let number = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let name = format!("EvoParseq-v{number}-win64.zip");
    let mut matches = release.assets.iter().filter(|a| a.name == name);
    let asset = matches
        .next()
        .ok_or("The stable release has no Windows ZIP.")?;
    if matches.next().is_some() {
        return Err("The stable release has duplicate Windows ZIPs.");
    }
    let expected = format!("{REPOSITORY}/releases/download/{}/{name}", release.tag_name);
    if asset.browser_download_url != expected {
        return Err("The download link is outside the expected repository.");
    }
    Ok(expected)
}
fn metadata(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, &'static str> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| "Cannot reach GitHub. The network may block update checks.")?;
    if !response.status().is_success() {
        return Err("GitHub update check failed, was blocked, or was rate limited.");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_METADATA as u64)
    {
        return Err("Release metadata is too large.");
    }
    let mut bytes = Vec::new();
    response
        .take((MAX_METADATA + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "Update check was interrupted.")?;
    if bytes.len() > MAX_METADATA {
        return Err("Release metadata is too large.");
    }
    Ok(bytes)
}
fn select(bytes: &[u8], installed: &str) -> Result<CheckResult, &'static str> {
    let release: Release =
        serde_json::from_slice(bytes).map_err(|_| "Invalid GitHub release response.")?;
    if release.draft || release.prerelease {
        return Err("Only published stable releases are supported.");
    }
    stable_version(&release.tag_name)?;
    if !release
        .assets
        .iter()
        .any(|a| a.name.starts_with("EvoParseq-v") && a.name.ends_with("-win64.zip"))
    {
        return Ok(CheckResult::NotPublished);
    }
    if !newer_stable(&release.tag_name, installed)? {
        return Ok(CheckResult::Current);
    }
    let download_url = download_url(&release)?;
    Ok(CheckResult::Available(Available {
        version: release.tag_name,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            .take(12000)
            .collect(),
        download_url,
    }))
}
fn latest() -> Result<CheckResult, &'static str> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("EvoParseq/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Cannot initialize the update check client.")?;
    select(&metadata(&client, API)?, env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(product: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "tag_name":"v1.0.0","draft":false,"prerelease":false,"body":"Notes\u{0000}",
            "assets":[{"name":format!("{product}-v1.0.0-win64.zip"),
            "browser_download_url":format!("{REPOSITORY}/releases/download/v1.0.0/{product}-v1.0.0-win64.zip")}]
        })).unwrap()
    }
    #[test]
    fn stable_selection_separates_products_and_never_downgrades() {
        assert!(matches!(
            select(&release("ParsecWebTurn"), "0.9.4").unwrap(),
            CheckResult::NotPublished
        ));
        let CheckResult::Available(r) = select(&release("EvoParseq"), "1.0.0-dev.7").unwrap()
        else {
            panic!()
        };
        assert_eq!(r.notes, "Notes");
        assert!(allowed_url(&r.download_url));
        for installed in ["1.0.0", "1.1.0-dev.1", "2.0.0"] {
            assert!(matches!(
                select(&release("EvoParseq"), installed).unwrap(),
                CheckResult::Current
            ));
        }
        for invalid in [
            "01.0.0",
            "1.0",
            "1.0.0-beta",
            "../1.0.0",
            "1.0.0.1",
            "18446744073709551616.0.0",
        ] {
            assert!(stable_version(invalid).is_err());
        }
        for invalid in ["1.0.0-dev.0", "1.0.0-dev.01", "1.0.0-beta"] {
            assert!(newer_stable("v1.0.0", invalid).is_err());
        }
    }
    #[test]
    fn download_requires_unique_published_canonical_project_asset() {
        let bytes = release("EvoParseq");
        let mut r: Release = serde_json::from_slice(&bytes).unwrap();
        let url = download_url(&r).unwrap();
        for invalid in [
            format!("{url}?redirect=x"),
            url.replace("github.com", "other.example"),
            "file:///EvoParseq.exe".into(),
        ] {
            assert!(!allowed_url(&invalid));
            r.assets[0].browser_download_url = invalid;
            assert!(download_url(&r).is_err());
        }
        r = serde_json::from_slice(&bytes).unwrap();
        r.assets.push(Asset {
            name: r.assets[0].name.clone(),
            browser_download_url: url,
        });
        assert!(download_url(&r).is_err());
        for flag in ["draft", "prerelease"] {
            let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            v[flag] = true.into();
            assert!(select(&serde_json::to_vec(&v).unwrap(), "0.9.4").is_err());
        }
        assert!(select(b"invalid JSON", "0.9.4").is_err());
    }
    #[test]
    fn metadata_rejects_redirects_and_bounds_bodies() {
        use std::io::Write;
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: https://other.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec(),
            [b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".as_slice(), &vec![b'x'; MAX_METADATA+1]].concat(),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request=[0;2048];
                assert!(stream.read(&mut request).unwrap()>0);
                let _ = stream.write_all(&response);
            });
            let client = reqwest::blocking::Client::builder().no_proxy().timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()).build().unwrap();
            assert!(metadata(&client,&url).is_err());
            server.join().unwrap();
        }
    }
    #[test]
    fn checking_is_nonblocking_single_flight_and_recoverable() {
        let state = Arc::new(State::default());
        let (tx, rx) = std::sync::mpsc::channel();
        state.check_with(move || {
            rx.recv().unwrap();
            Ok(CheckResult::NotPublished)
        });
        assert_eq!(state.view().phase, Phase::Checking);
        state.check_with(|| panic!("duplicate request"));
        tx.send(()).unwrap();
        fn finish(state: &State) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while state.view().phase == Phase::Checking && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_ne!(state.view().phase, Phase::Checking);
        }
        finish(&state);
        assert_eq!(state.view().phase, Phase::Current);
        state.check_with(|| Err("Network unavailable"));
        finish(&state);
        assert_eq!(state.view().phase, Phase::Error);
        state.check_with(|| {
            Ok(CheckResult::Available(Available {
                version: "v1.0.0".into(),
                notes: "Notes".into(),
                download_url: format!(
                    "{REPOSITORY}/releases/download/v1.0.0/EvoParseq-v1.0.0-win64.zip"
                ),
            }))
        });
        finish(&state);
        assert_eq!(state.view().phase, Phase::Available);
        assert!(content(Mode::Updates, &state.view())
            .version
            .contains("v1.0.0"));
        let about = content(Mode::About, &state.view());
        assert!(about.version.contains("Native Rust"));
        assert!(!about.version.contains("WebAssembly"));
        assert!(about.notes.contains("independent"));
    }
}
