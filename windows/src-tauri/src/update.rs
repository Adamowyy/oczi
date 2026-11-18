// Is there a newer release than the one running? One GitHub API call, no key — and, when
// there is, its installer is downloaded, checked and applied silently after Oczi exits.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

/// The repository releases are published from. Kept here rather than derived
/// from `CARGO_PKG_REPOSITORY` so a fork can point it elsewhere in one place.
const REPO: &str = "Adamowyy/oczi";

/// Shorter than the chat's 15 s: this runs at startup and must never be the
/// reason the island takes a moment to appear.
const TIMEOUT: Duration = Duration::from_secs(6);

/// GitHub refuses a request without a User-Agent.
const USER_AGENT: &str = concat!("Oczi/", env!("CARGO_PKG_VERSION"));

/// How long the update check waits after launch before it asks: the island is what the user
/// is looking at, and a request plus a 2.5 MB download may not compete with it.
const SETTLE: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// The release tag with any leading `windows-` and `v` trimmed: `0.1.4`.
    pub version: String,
    /// The release page, for the clickable line in the card.
    pub url: String,
}

/// An installer that is on disk and checked, waiting for the next exit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ready {
    pub version: String,
    /// Absolute path of the downloaded installer.
    pub installer: String,
}

/// True when `remote` is a higher version than `current`. The fields are
/// compared as numbers, so 0.10.0 is newer than 0.9.0. A version that cannot be
/// read is never treated as newer: a malformed tag must not nag the user.
pub fn is_newer(remote: &str, current: &str) -> bool {
    let parse = |v: &str| -> Option<Vec<u64>> {
        let mut fields = Vec::new();
        for part in v.trim().trim_start_matches(['v', 'V']).split('.') {
            // A suffix (`-beta.1`) makes the field non-numeric; stop there.
            let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                return None;
            }
            fields.push(digits.parse().ok()?);
        }
        Some(fields)
    };
    let (Some(remote), Some(current)) = (parse(remote), parse(current)) else {
        return false;
    };
    // Padded, so 0.2 and 0.2.0 compare as the same version.
    for i in 0..remote.len().max(current.len()) {
        let a = remote.get(i).copied().unwrap_or(0);
        let b = current.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// The version a release tag carries: `windows-v0.5.10` and `v0.5.10` are both `0.5.10`.
/// The tag is what the releases actually use, and reading the `windows-` prefix as part of
/// the version is what made every check answer "nothing newer".
pub fn version_of_tag(tag: &str) -> String {
    let trimmed = tag.trim();
    let without_prefix = trimmed
        .strip_prefix("windows-")
        .or_else(|| trimmed.strip_prefix("Windows-"))
        .unwrap_or(trimmed);
    without_prefix
        .trim_start_matches(['v', 'V'])
        .trim()
        .to_string()
}

/// The newest published release, when it is newer than the version running.
pub async fn latest(current: &str) -> Option<Release> {
    let Some(release) = fetch().await else {
        return None;
    };
    if !is_newer(&release.version, current) {
        return None;
    }
    Some(release)
}

/// `%LOCALAPPDATA%\Oczi\update`: where a downloaded installer waits for the next exit.
fn update_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Oczi").join("update"))
}

/// The installed copy's own path, when this process is that copy. A build run from
/// `target\release` is somebody's own dev copy and must never be replaced from here.
fn installed_exe() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let expected = PathBuf::from(local).join("Oczi").join("oczi.exe");
    let running = std::env::current_exe().ok()?;
    (running == expected).then_some(expected)
}

/// What a release offers: the installer, its size and its checksum.
struct Asset {
    url: String,
    size: u64,
    digest: String,
}

async fn fetch() -> Option<Release> {
    let body = release_json().await?;
    let version = version_of_tag(body.get("tag_name")?.as_str()?);
    let url = body.get("html_url")?.as_str()?.to_string();
    if version.is_empty() || url.is_empty() {
        return None;
    }
    Some(Release { version, url })
}

async fn release_json() -> Option<serde_json::Value> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| crate::log::line(format!("update check: no client: {e}")))
        .ok()?;
    client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        // A 404 is normal while no release exists yet, so the status is logged
        // rather than treated as an error worth reporting.
        .and_then(|r| r.error_for_status())
        .map_err(|e| crate::log::line(format!("update check: {e}")))
        .ok()?
        .json()
        .await
        .map_err(|e| crate::log::line(format!("update check: unreadable reply: {e}")))
        .ok()
}

/// The installer a release carries, named `Oczi-Windows-<version>-setup.exe` by `pack.mjs`.
/// The checksum is not optional: a file this app downloads and later runs has to be the file
/// GitHub says it published, or it is not run at all.
fn installer_of(body: &serde_json::Value, version: &str) -> Option<Asset> {
    let wanted = format!("Oczi-Windows-{version}-setup.exe");
    let assets = body.get("assets")?.as_array()?;
    let pick = assets
        .iter()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(wanted.as_str()))
        .or_else(|| {
            assets.iter().find(|a| {
                let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                name.starts_with("Oczi-Windows-") && name.ends_with("-setup.exe")
            })
        })?;
    let url = pick.get("browser_download_url")?.as_str()?.to_string();
    let size = pick.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
    let digest = pick.get("digest")?.as_str()?.to_string();
    if !digest.starts_with("sha256:") {
        return None;
    }
    Some(Asset { url, size, digest })
}

fn hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let sum = sha2::Sha256::digest(bytes);
    sum.iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads the installer into the update folder, refusing anything whose checksum or size
/// is not the one the release published. The file only takes its real name once it has been
/// checked, so a half-finished download can never be mistaken for a ready update.
async fn download(version: &str, asset: &Asset) -> Option<Ready> {
    let dir = update_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;
    let bytes = client
        .get(&asset.url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| crate::log::line(format!("update download: {e}")))
        .ok()?
        .bytes()
        .await
        .map_err(|e| crate::log::line(format!("update download: {e}")))
        .ok()?;

    if asset.size > 0 && bytes.len() as u64 != asset.size {
        crate::log::line(format!(
            "update download: {} B, release says {} B — refused",
            bytes.len(),
            asset.size
        ));
        return None;
    }
    let want = asset.digest.trim_start_matches("sha256:").to_ascii_lowercase();
    let got = hex(&bytes);
    if got != want {
        crate::log::line(format!(
            "update download: sha256 {got} — release says {want}"
        ));
        return None;
    }

    let part = dir.join("installer.part");
    std::fs::write(&part, &bytes).ok()?;
    let final_path = dir.join(format!("Oczi-Windows-{version}-setup.exe"));
    std::fs::rename(&part, &final_path).ok()?;
    Some(Ready {
        version: version.to_string(),
        installer: final_path.to_string_lossy().to_string(),
    })
}

/// Leaves a windowless watcher that waits for this process to go, then runs the installer
/// silently. Nothing is replaced under a running app: the new build is what starts next, and
/// the installer needs no prompt because it installs per-user into `%LOCALAPPDATA%\Oczi`.
fn arm(ready: &Ready) -> bool {
    if installed_exe().is_none() {
        return false;
    }
    let Some(dir) = update_dir() else {
        return false;
    };
    let pid = std::process::id();
    // `ping` is the sleep here: `timeout` wants a console and hangs without one. The PID is
    // matched as a whole word, so 12345 cannot keep the watcher waiting on 1234.
    let lines = [
        "@echo off".to_string(),
        ":wait".to_string(),
        format!("tasklist /NH /FI \"PID eq {pid}\" 2>nul | findstr /R /C:\"\\<{pid}\\>\" >nul || goto apply"),
        "ping -n 3 127.0.0.1 >nul".to_string(),
        "goto wait".to_string(),
        ":apply".to_string(),
        format!("start \"\" /wait \"{installer}\" /S", installer = ready.installer),
        format!("del /f /q \"{installer}\" 2>nul", installer = ready.installer),
        String::new(),
    ];
    let script = lines.join("\r\n");
    let bat = dir.join("apply_update.bat");
    if std::fs::write(&bat, script).is_err() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW, never DETACHED_PROCESS: a detached cmd gets a fresh console
        // and the user sees a black window flash on every update.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let spawned = std::process::Command::new("cmd.exe")
            .arg("/c")
            .arg(&bat)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(_) => true,
            Err(e) => {
                crate::log::line(format!("update: could not start the swap: {e}"));
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = bat;
        false
    }
}

/// An installer already in the update folder that is newer than this build — downloaded by
/// this run, or left behind by one that was killed before it could apply it.
pub fn pending() -> Option<Ready> {
    let dir = update_dir()?;
    let current = env!("CARGO_PKG_VERSION");
    let mut best: Option<Ready> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("Oczi-Windows-") else {
            continue;
        };
        let Some(version) = rest.strip_suffix("-setup.exe") else {
            continue;
        };
        if !is_newer(version, current) {
            continue;
        }
        if best
            .as_ref()
            .map(|b| is_newer(version, &b.version))
            .unwrap_or(true)
        {
            best = Some(Ready {
                version: version.to_string(),
                installer: path.to_string_lossy().to_string(),
            });
        }
    }
    best
}

/// Called once from `setup`, never on the UI thread: asks GitHub well after the island is up,
/// downloads the installer when it is newer than this build, and arms the swap. A machine
/// that is offline, and a dev build, log nothing and do nothing else.
pub fn start(app: tauri::AppHandle) {
    if installed_exe().is_none() {
        // A build run from `target\release` is somebody's own copy: replacing it with a
        // release would be a surprise, so self-update stays off outside the installed app.
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SETTLE).await;
        // Something from a previous run that never got applied: arm it before asking again.
        if let Some(ready) = pending() {
            if arm(&ready) {
                crate::log::line(format!("update: {} waits for the next exit", ready.version));
            }
        }
        let current = env!("CARGO_PKG_VERSION");
        let Some(body) = release_json().await else {
            return;
        };
        let tag = body.get("tag_name").and_then(|t| t.as_str()).unwrap_or("");
        let version = version_of_tag(tag);
        if !is_newer(&version, current) {
            return;
        }
        let Some(asset) = installer_of(&body, &version) else {
            crate::log::line(format!(
                "update: {version} carries no installer with a checksum"
            ));
            return;
        };
        crate::log::line(format!("update: {version} found, downloading"));
        let Some(ready) = download(&version, &asset).await else {
            return;
        };
        crate::log::line(format!(
            "update: {} downloaded ({} B) — installs after exit",
            ready.version, asset.size
        ));
        // So the card can say the app already has it, instead of offering the download.
        let _ = tauri::Emitter::emit_to(
            &app,
            crate::island::WINDOW_LABEL,
            "update-ready",
            ready.clone(),
        );
        if arm(&ready) {
            crate::log::line("update: swap armed".to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{is_newer, version_of_tag};

    #[test]
    fn a_higher_release_is_newer() {
        assert!(is_newer("0.1.4", "0.1.3"));
        assert!(is_newer("0.2.0", "0.1.9"));
        // Field by field, not as text: 0.10.0 comes after 0.9.0.
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        // A release tag usually carries a v.
        assert!(is_newer("v0.1.4", "0.1.3"));
    }

    #[test]
    fn the_version_running_is_not_newer() {
        assert!(!is_newer("0.1.3", "0.1.3"));
        assert!(!is_newer("v0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.2", "0.1.3"));
        // Padding: a shorter tag is not an older one.
        assert!(!is_newer("0.2", "0.2.0"));
    }

    #[test]
    fn an_unreadable_tag_never_nags() {
        assert!(!is_newer("", "0.1.3"));
        assert!(!is_newer("nightly", "0.1.3"));
        assert!(!is_newer("v", "0.1.3"));
        assert!(!is_newer("0.1.x", "0.1.3"));
    }

    #[test]
    fn a_release_tag_reads_as_a_version() {
        // The tag Oczi publishes under, which the older parser read as no version at all.
        assert_eq!(version_of_tag("windows-v0.5.10"), "0.5.10");
        assert_eq!(version_of_tag("v0.5.10"), "0.5.10");
        assert_eq!(version_of_tag("0.5.10"), "0.5.10");
        assert_eq!(version_of_tag(" windows-v0.6.0 "), "0.6.0");
        assert!(is_newer(&version_of_tag("windows-v0.5.11"), "0.5.10"));
    }
}
