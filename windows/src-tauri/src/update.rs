// Is there a newer release than the one running? One GitHub API call, no key.

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    /// The release tag with any leading `v` trimmed: `0.1.4`.
    pub version: String,
    /// The release page, for the clickable line in the card.
    pub url: String,
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

async fn fetch() -> Option<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| crate::log::line(format!("update check: no client: {e}")))
        .ok()?;
    let body: serde_json::Value = client
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
        .ok()?;

    let version = body.get("tag_name")?.as_str()?.trim().to_string();
    let url = body.get("html_url")?.as_str()?.to_string();
    if version.is_empty() || url.is_empty() {
        return None;
    }
    Some(Release { version, url })
}

#[cfg(test)]
mod tests {
    use super::is_newer;

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
}
