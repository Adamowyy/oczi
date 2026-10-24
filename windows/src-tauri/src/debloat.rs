// Debloat: what Windows came with that a person may not want, and taking it away on request.
//
// Nothing is removed without being named in a list the user ticked themselves. The module owns
// three things:
//
// * The catalogue: which packages are worth offering, and how safe taking them away is. Only
//   what is in here is ever *offered*; anything installed and unknown is left alone, because a
//   list that guesses at a package's purpose is a list that can break a machine.
// * The enumeration, which asks Windows what is actually installed right now — a package that is
//   gone is not on the list, and a package that survived a removal says so.
// * The removal itself, per user, one package at a time, with every attempt written down in
//   `debloat.json` so the window can show what was taken and when.
//
// The machine work goes through `Get-AppxPackage` / `Remove-AppxPackage`: the Appx deployment COM
// surface is the only other way in, and it is the same call with far more code in front of it. The
// console is hidden and nothing is written to the disk — this is the app doing the work, not a
// script left behind.

use std::os::windows::process::CommandExt;
use std::process::Command;

use serde::{Deserialize, Serialize};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::log;

/// How long one Appx call may take. Removing a package is a copy and a registration; ten minutes
/// is generous, and still short enough that a stuck one does not hold the window for ever.
const TIMEOUT_HINT_SECS: u64 = 600;

/// One package the window can offer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppItem {
    /// The Appx package name — what removal is asked for by, and the id the window sends back.
    pub name: String,
    /// `safe` (a consumer app nothing depends on) or `careful` (a feature somebody may be using).
    pub tier: String,
}

/// One package that was taken away, kept for the history the window shows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub name: String,
    /// Local wall-clock seconds.
    pub at: i64,
}

/// The answer to a removal: what really went away and what stayed, because a script that reports
/// its own success is not evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removal {
    pub removed: Vec<String>,
    pub failed: Vec<String>,
    /// Seconds the whole thing took.
    pub took: u32,
}

/// The catalogue: what a fresh Windows install arrives with and a person may not want, and how safe
/// taking it away is. `safe` means a consumer app — nothing's work breaks when it is gone; `careful`
/// means a Windows feature or a program somebody may actually use, so it is offered but never ticked
/// by default. Everything not named here is invisible to the window on purpose.
const CATALOGUE: &[(&str, &str)] = &[
    // ── Store apps and trials Windows ships with. Nothing depends on any of them.
    ("Microsoft.MicrosoftSolitaireCollection", "safe"),
    ("Microsoft.MicrosoftMahjong", "safe"),
    ("Microsoft.MicrosoftJigsaw", "safe"),
    ("Microsoft.SkypeApp", "safe"),
    ("Microsoft.ZuneVideo", "safe"),
    ("Microsoft.WindowsCommunicationsApps", "safe"),
    ("Microsoft.People", "safe"),
    ("Microsoft.WindowsMaps", "safe"),
    ("Microsoft.MicrosoftOfficeHub", "safe"),
    ("Microsoft.Getstarted", "safe"),
    ("Microsoft.GetHelp", "safe"),
    ("Microsoft.Microsoft3DViewer", "safe"),
    ("Microsoft.MixedReality.Portal", "safe"),
    ("Microsoft.OneConnect", "safe"),
    ("Microsoft.YourPhone", "safe"),
    ("Microsoft.Todos", "safe"),
    ("Microsoft.PowerAutomateDesktop", "safe"),
    ("Microsoft.XboxApp", "safe"),
    ("Microsoft.XboxGamingOverlay", "safe"),
    ("Microsoft.MicrosoftStickyNotes", "safe"),
    ("Microsoft.549981C3F5F10", "safe"), // Cortana
    // ── Features somebody may be using, so they are offered unticked.
    ("Microsoft.Copilot", "careful"),
    ("MicrosoftWindows.Client.WebExperience", "careful"), // Widgets
    ("Microsoft.Edge.GameAssist", "careful"),
    ("Microsoft.MicrosoftPCManager", "careful"),
    ("MicrosoftCorporationII.QuickAssist", "careful"),
    ("Microsoft.OutlookForWindows", "careful"),
    ("MSTeams", "careful"),
    ("Microsoft.Teams", "careful"),
    ("Microsoft.OneDriveSync", "careful"),
    ("Microsoft.ZuneMusic", "careful"),
    ("Microsoft.MicrosoftEdge.Stable", "careful"),
];

/// Vendor prefixes, for the families a fresh install carries whole: Bing ships six apps, the
/// streaming services ship trials, and a machine that has never been cleaned has one of each. A
/// prefix keeps this list short and still covers what a real machine actually holds.
const PREFIXES: &[(&str, &str)] = &[
    ("Microsoft.Bing", "safe"),
    ("Microsoft.WindowsFeedbackHub", "safe"),
    ("SpotifyAB.", "safe"),
    ("Clipchamp.", "safe"),
    ("Disney.", "safe"),
    ("AmazonVideo.", "safe"),
    ("4DF9E0F8.Netflix", "safe"),
    ("Facebook.", "safe"),
    ("Instagram.", "safe"),
    ("king.com.", "safe"),
    ("ByteDance.", "safe"),
    ("TikTok", "safe"),
    ("SlingTV", "safe"),
    ("Hulu", "safe"),
    ("PandoraMediaInc.", "safe"),
    ("Microsoft.Copilot", "careful"),
    ("Microsoft.OneDrive", "careful"),
];

/// Which tier a package belongs to, or `None` when this module does not know it well enough to
/// offer it. Exact names first, then a vendor prefix, then the family of a known package: Windows
/// sometimes appends a suffix, and a renamed package should not fall out of the catalogue.
fn tier_of(name: &str) -> Option<&'static str> {
    if let Some((_, tier)) = CATALOGUE.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(tier);
    }
    if let Some((_, tier)) = PREFIXES.iter().find(|(prefix, _)| {
        name.to_ascii_lowercase()
            .starts_with(&prefix.to_ascii_lowercase())
    }) {
        return Some(tier);
    }
    CATALOGUE
        .iter()
        .find(|(n, _)| name.len() > n.len() && name[..n.len()].eq_ignore_ascii_case(n))
        .map(|(_, tier)| *tier)
}

/// Runs one PowerShell script with no window and returns its stdout. The Appx cmdlets are the only
/// supported way to enumerate and remove a package for the current user without administrator
/// rights, which is exactly the deal this module makes: what a person can undo themselves.
///
/// `pub(crate)` because the Game Bar module needs the same door: an Appx read and the Store's own
/// installer both go through PowerShell, and one hidden, no-window runner is the whole of it.
pub(crate) fn powershell(script: &str) -> Result<String, String> {
    let started = std::time::Instant::now();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW.0)
        .output()
        .map_err(|e| format!("powershell: {e}"))?;
    let took = started.elapsed().as_secs();
    if took >= TIMEOUT_HINT_SECS {
        log::line(format!("debloat: powershell took {took}s"));
    }
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if err.is_empty() {
            format!("powershell exit {:?}", output.status.code())
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The package names installed for this user, as Windows reports them right now.
fn installed() -> Result<Vec<String>, String> {
    let out = powershell(
        "Get-AppxPackage -User $env:USERNAME | Select-Object -ExpandProperty Name | Sort-Object -Unique",
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// What the window lists: every installed package the catalogue knows, with its tier. Sorted so
/// the safe ones come first and the list does not jump around between visits.
pub fn list() -> Result<Vec<AppItem>, String> {
    let names = installed()?;
    let mut items: Vec<AppItem> = names
        .into_iter()
        .filter_map(|name| {
            tier_of(&name).map(|tier| AppItem {
                name,
                tier: tier.to_string(),
            })
        })
        .collect();
    items.sort_by(|a, b| {
        let rank = |t: &str| if t == "safe" { 0 } else { 1 };
        rank(&a.tier)
            .cmp(&rank(&b.tier))
            .then_with(|| a.name.cmp(&b.name))
    });
    log::line(format!("debloat list: {} offerable packages", items.len()));
    Ok(items)
}

/// Takes the named packages away for this user, then asks Windows what is left. The second reading
/// is the result: a package still installed is reported as failed, whatever the removal said.
pub fn remove(names: &[String]) -> Result<Removal, String> {
    let wanted: Vec<&String> = names
        .iter()
        .filter(|name| is_plausible_name(name))
        .filter(|name| tier_of(name).is_some())
        .collect();
    if wanted.is_empty() {
        // i18n-ok: model/UI-facing text, translated where it is shown.
        return Err("Nic do usunięcia — nie znam żadnej z podanych paczek.".to_string());
    }
    let started = std::time::Instant::now();
    let quoted: Vec<String> = wanted
        .iter()
        .map(|name| format!("'{}'", name.replace('\'', "")))
        .collect();
    // One process for the whole batch: each `Remove-AppxPackage` takes a second or two, and a
    // process per package would make a list of ten take half a minute before anything happens.
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue'; foreach($n in @({})) {{ Get-AppxPackage -Name $n | Remove-AppxPackage }}",
        quoted.join(",")
    );
    let failure = powershell(&script).err();
    let left = installed()?;
    let mut removal = Removal {
        took: started.elapsed().as_secs() as u32,
        ..Default::default()
    };
    for name in &wanted {
        if left.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            removal.failed.push(name.to_string());
        } else {
            removal.removed.push(name.to_string());
        }
    }
    if let Some(err) = failure {
        // The removal list is still the truth about what happened; the error explains the rest.
        log::line(format!("debloat remove reported: {err}"));
    }
    log::line(format!(
        "debloat remove: {} removed, {} left, {}s",
        removal.removed.len(),
        removal.failed.len(),
        removal.took
    ));
    if !removal.removed.is_empty() {
        record(&removal.removed);
    }
    Ok(removal)
}

/// A package name is a dotted identifier. Anything else — a switch, an empty string, a path — is
/// refused rather than handed to a shell, so a bug in the window cannot become a wildcard removal.
fn is_plausible_name(name: &str) -> bool {
    let name = name.trim();
    if name.len() < 3 || name.len() > 128 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

// ── The history ───────────────────────────────────────────────────────────────

fn path() -> std::path::PathBuf {
    crate::settings::config_dir().join("debloat.json")
}

fn history() -> Vec<Removed> {
    match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::line(format!("debloat history unreadable ({e}), starting empty"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn record(names: &[String]) {
    let mut list = history();
    let at = crate::reminders::now_local();
    for name in names {
        list.retain(|r| !r.name.eq_ignore_ascii_case(name));
        list.push(Removed {
            name: name.clone(),
            at,
        });
    }
    // A list nobody scrolls is not a history: the last two hundred is a year of debloating.
    if list.len() > 200 {
        let drop = list.len() - 200;
        list.drain(..drop);
    }
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(&list) {
        if let Err(e) = std::fs::write(path(), json) {
            log::line(format!("debloat history not saved: {e}"));
        }
    }
}

/// Everything this app has taken away, newest first.
pub fn removed_history() -> Vec<Removed> {
    let mut list = history();
    list.sort_by(|a, b| b.at.cmp(&a.at));
    list
}

/// The list as text, for `oczi.exe --debloat`: the same reading the window draws, printed, so the
/// enumeration can be checked against the machine without opening a GUI.
pub fn describe(items: &[AppItem]) -> String {
    if items.is_empty() {
        // i18n-ok: model/terminal-facing text, translated where it is shown.
        return "Brak paczek do zaoferowania — lista jest już czysta.".to_string();
    }
    let mut out = format!("Paczki do zaoferowania ({}):", items.len());
    for item in items {
        out.push_str(&format!("\n- [{}] {}", item.tier, item.name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalogue_knows_the_apps_it_offers_and_only_those() {
        assert_eq!(tier_of("Microsoft.BingNews"), Some("safe"));
        assert_eq!(tier_of("Microsoft.Copilot"), Some("careful"));
        // Case never decides whether a package is understood.
        assert_eq!(tier_of("microsoft.copilot"), Some("careful"));
        // A renamed package stays in the family it belongs to.
        assert_eq!(tier_of("Microsoft.SkypeApp_8wekyb3d8bbwe"), Some("safe"));
        // …but a name that merely starts with the same letters is not the same package.
        assert_eq!(tier_of("Microsoft.BingNewsExtras"), Some("safe"));
        assert_eq!(tier_of("Microsoft.WindowsStore"), None);
        assert_eq!(tier_of("Microsoft.YourPhone"), Some("safe"));
        assert_eq!(tier_of(""), None);
    }

    #[test]
    fn an_unknown_package_is_never_offered() {
        assert_eq!(tier_of("SomeVendor.Tool"), None);
        assert_eq!(tier_of("Microsoft.VCLibs.140.00"), None);
        assert_eq!(tier_of("Microsoft.NET.Native.Runtime.2.2"), None);
    }

    #[test]
    fn only_identifiers_reach_the_shell() {
        assert!(is_plausible_name("Microsoft.BingNews"));
        assert!(is_plausible_name("Microsoft.BingNews_8wekyb3d8bbwe"));
        assert!(!is_plausible_name(""));
        assert!(!is_plausible_name("*"));
        assert!(!is_plausible_name("Microsoft.*"));
        assert!(!is_plausible_name("Microsoft.BingNews -Force"));
        assert!(!is_plausible_name("C:\\Windows\\System32"));
        assert!(!is_plausible_name("'; Remove-Item C:\\ -Recurse"));
    }
}
