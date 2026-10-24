// Game Bar and Game DVR: the overlay Windows puts over a game, and the capture it keeps running
// behind one whether or not anybody asked.
//
// A fresh install has both on — Win+G raises Game Bar, and DVR holds the last thirty seconds ready
// to be saved. This module owns the two things a person can turn off, and reads both back from
// Windows before saying anything about them:
//
// * The package (`Microsoft.XboxGamingOverlay`), which the debloat catalogue already lists; here
//   it gets a name, a version and one button, and the Store id it can be put back with.
// * The capture switches. Windows keeps that answer in four registry values and all four have to
//   agree, which is why one toggle writes a set rather than one key. Two of them are per-user and
//   any process may write them; the other two are machine-wide policy and need an administrator,
//   so a refusal there is reported as a refusal — never smoothed over. The user-level switch alone
//   is enough to stop the recording; the policy is what keeps Windows from turning it back on.
//
// Everything that reads is read after the write, so the window shows the machine and not the
// intent. Errors are Polish because they are shown as they are; see `i18n-ok` below.

use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::debloat::powershell;
use crate::elevate;
use crate::log;
use crate::registry;

/// The Appx package behind the Win+G overlay.
pub const PACKAGE: &str = "Microsoft.XboxGamingOverlay";

/// What the Microsoft Store knows Game Bar by — the id `winget` installs it with, and the only
/// honest way back for a package this app has removed: the provisioned copy is Windows' business.
const STORE_ID: &str = "9NZKPSTSNW4P";

/// How long the elevated write may take to show up. An administrator has to answer a UAC prompt
/// first, so this waits rather than calling a slow "yes" a failure.
const ELEVATED_WAIT_MS: u64 = 30_000;
const ELEVATED_STEP_MS: u64 = 500;

/// One registry value that decides whether capture happens. `machine` marks the two that live in
/// the machine hive: those need an administrator, and failing to write them is not the same kind
/// of failure as failing to write a user's own switch.
struct Switch {
    machine: bool,
    root: HKEY,
    path: &'static str,
    name: &'static str,
}

const GAME_DVR: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\GameDVR";
const GAME_CONFIG: &str = "System\\GameConfigStore";
const POLICY_MANAGER: &str =
    "SOFTWARE\\Microsoft\\PolicyManager\\default\\ApplicationManagement\\AllowGameDVR";
const POLICY_DVR: &str = "SOFTWARE\\Policies\\Microsoft\\Windows\\GameDVR";

/// The four values behind one toggle, in the order the .reg files on the desktop wrote them.
const SWITCHES: &[Switch] = &[
    Switch {
        machine: false,
        root: HKEY_CURRENT_USER,
        path: GAME_DVR,
        name: "AppCaptureEnabled",
    },
    Switch {
        machine: false,
        root: HKEY_CURRENT_USER,
        path: GAME_CONFIG,
        name: "GameDVR_Enabled",
    },
    Switch {
        machine: true,
        root: HKEY_LOCAL_MACHINE,
        path: POLICY_MANAGER,
        name: "value",
    },
    Switch {
        machine: true,
        root: HKEY_LOCAL_MACHINE,
        path: POLICY_DVR,
        name: "AllowGameDVR",
    },
];

/// Game Bar and the capture switches, as Windows has them right now.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameBar {
    /// Whether the overlay package is registered for this user.
    pub installed: bool,
    /// The version Windows reports, when there is one.
    pub version: Option<String>,
    /// The user's own switches: `true` when Windows would record.
    pub capture: bool,
    /// The machine-wide policy: `false` when it forbids capture, `None` when nobody ever set it.
    pub policy: Option<bool>,
    /// Set when the machine level could not be written without an administrator.
    pub needs_admin: bool,
}

// ── The registry, one value at a time ─────────────────────────────────────────
// The four calls themselves live in `registry`, which the tweaks table uses too: one place where a
// path is opened with the right view is one place a 64-bit mistake cannot be made twice.

/// Reads a DWORD, or `None` when the key or the value is not there. A missing value is not an
/// error: Windows answers such a question from its own defaults, and so does `capture` below.
fn read(root: HKEY, path: &str, name: &str, machine: bool) -> Option<u32> {
    registry::read_dword(root, path, name, machine)
}

/// Writes a DWORD, creating the key when Windows has never had that value. `false` means the write
/// did not land — which is the whole answer for a machine-level switch under an ordinary account.
fn write(switch: &Switch, value: u32) -> bool {
    registry::write_dword(switch.root, switch.path, switch.name, switch.machine, value)
}

// ── Reading the machine ───────────────────────────────────────────────────────

/// Capture, from the user's two switches only: absent means Windows' default, which is on.
fn capture() -> bool {
    SWITCHES
        .iter()
        .filter(|switch| !switch.machine)
        .all(|switch| read(switch.root, switch.path, switch.name, false).unwrap_or(1) != 0)
}

/// The machine level: the policy when one is set, else the PolicyManager default, else nothing.
fn policy() -> Option<bool> {
    let dvr = read(HKEY_LOCAL_MACHINE, POLICY_DVR, "AllowGameDVR", true);
    let manager = read(HKEY_LOCAL_MACHINE, POLICY_MANAGER, "value", true);
    dvr.or(manager).map(|value| value != 0)
}

/// The package and the version Windows reports for it, in one PowerShell call. `2>$null` only
/// silences the "no packages were found" note a missing package earns; the answer is the stdout.
fn package() -> Result<Option<String>, String> {
    let script = format!(
        "$p = Get-AppxPackage -User $env:USERNAME -Name '{PACKAGE}' 2>$null | Select-Object -First 1; \
         if ($p) {{ $p.Version }}"
    );
    let out = powershell(&script)?;
    Ok(out
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string))
}

/// Everything the window shows, read from Windows: the package, the user switches, the policy.
pub fn status() -> Result<GameBar, String> {
    let version = package()?;
    Ok(GameBar {
        installed: version.is_some(),
        version,
        capture: capture(),
        policy: policy(),
        needs_admin: false,
    })
}

// ── Changing it ───────────────────────────────────────────────────────────────

/// Turns capture on or off. Both levels are written; the user's own two must land, and the machine
/// pair is attempted and reported. The state that comes back is a fresh reading, not the request.
pub fn set_capture(on: bool) -> Result<GameBar, String> {
    let value = u32::from(on);
    let mut user_written = 0;
    let mut machine_written = 0;
    for switch in SWITCHES {
        if write(switch, value) {
            if switch.machine {
                machine_written += 1;
            } else {
                user_written += 1;
            }
        }
    }
    let machine_total = SWITCHES.iter().filter(|switch| switch.machine).count();
    if user_written == 0 {
        // i18n-ok: shown as it is in the Debloat window.
        return Err("Nie udało się zapisać przełącznika nagrywania w rejestrze.".to_string());
    }
    log::line(format!(
        "gamebar capture={on}: {user_written} user, {machine_written}/{machine_total} machine values written"
    ));
    let mut state = status()?;
    state.needs_admin = machine_written < machine_total;
    Ok(state)
}

/// The elevated script: the machine-level values, and nothing else. Kept apart from the spawning so
/// what an administrator is asked to run can be read — and tested — as text.
fn policy_script(on: bool) -> String {
    let value = u32::from(on);
    let mut out = String::from("$ErrorActionPreference='Stop';");
    for switch in SWITCHES.iter().filter(|switch| switch.machine) {
        out.push_str(&format!(
            "New-Item -Path 'Registry::HKEY_LOCAL_MACHINE\\{}' -Force | Out-Null; \
             Set-ItemProperty -Path 'Registry::HKEY_LOCAL_MACHINE\\{}' -Name '{}' -Value {value} -Type DWord;",
            switch.path, switch.path, switch.name
        ));
    }
    out
}

/// Asks Windows for the administrator once, for the machine-level pair, and waits for the answer
/// to appear in the registry. A refused prompt is reported as a refusal: nothing changed.
pub fn policy_elevated(on: bool) -> Result<GameBar, String> {
    // The quoting, the encoding and the refusal message all live in `elevate`; what is left here is
    // this module's own question — did the policy actually change?
    elevate::run_elevated(&policy_script(on))?;
    let mut waited = 0;
    while waited < ELEVATED_WAIT_MS {
        std::thread::sleep(std::time::Duration::from_millis(ELEVATED_STEP_MS));
        waited += ELEVATED_STEP_MS;
        if policy() == Some(on) {
            break;
        }
    }
    let mut state = status()?;
    if state.policy != Some(on) {
        // i18n-ok: shown as it is in the Debloat window.
        return Err("Poziom maszyny nie zmienił się — sprawdź zgodę administratora.".to_string());
    }
    state.needs_admin = false;
    log::line(format!("gamebar policy={on}: set with an administrator"));
    Ok(state)
}

// ── The package ───────────────────────────────────────────────────────────────

/// `winget.exe`, wherever it is: the app-execution alias puts it on PATH for most accounts, and a
/// machine where the alias is off still has the real file under `WindowsApps`.
fn winget_exe() -> Vec<OsString> {
    let mut out = vec![OsString::from("winget.exe")];
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let alias: PathBuf = [&local, "Microsoft\\WindowsApps\\winget.exe"]
            .iter()
            .collect();
        out.push(alias.into_os_string());
    }
    out
}

fn winget(args: &[&str]) -> Result<String, String> {
    let mut last = String::new();
    for exe in winget_exe() {
        match Command::new(&exe)
            .args(args)
            .creation_flags(CREATE_NO_WINDOW.0)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    return Ok(String::from_utf8_lossy(&output.stdout).to_string());
                }
                let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if err.is_empty() {
                    format!("winget exit {:?}", output.status.code())
                } else {
                    err
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last = format!("winget: {e}");
            }
            Err(e) => return Err(format!("winget: {e}")),
        }
    }
    Err(last)
}

/// Puts Game Bar back the way the desktop's `reinstall` script did: from the Store, silently, and
/// then a reading of what Windows really registered. A failure that Windows contradicts is not a
/// failure — the reading is the answer.
pub fn install() -> Result<GameBar, String> {
    let started = std::time::Instant::now();
    let failure = winget(&[
        "install",
        STORE_ID,
        "--source",
        "msstore",
        "--accept-source-agreements",
        "--accept-package-agreements",
        "--disable-interactivity",
    ])
    .err();
    let state = status()?;
    log::line(format!(
        "gamebar install: installed={} in {}s",
        state.installed,
        started.elapsed().as_secs()
    ));
    if state.installed {
        return Ok(state);
    }
    Err(failure.unwrap_or_else(|| {
        // i18n-ok: shown as it is in the Debloat window.
        "Sklep nie zainstalował Game Bar.".to_string()
    }))
}

/// The state as text, for `oczi.exe --gamebar`: the same reading the window draws, printed, so the
/// switches can be checked against the machine without opening a GUI.
pub fn describe(state: &GameBar) -> String {
    let mut out = match &state.version {
        Some(version) => format!("Game Bar: installed ({version})"),
        None => "Game Bar: not installed".to_string(),
    };
    out.push_str(if state.capture {
        "\nCapture: on"
    } else {
        "\nCapture: off"
    });
    out.push_str(match state.policy {
        Some(true) => "\nMachine policy: allowed",
        Some(false) => "\nMachine policy: blocked",
        None => "\nMachine policy: not set",
    });
    if state.needs_admin {
        out.push_str("\nMachine policy needs an administrator");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_toggle_covers_all_four_values() {
        let user = SWITCHES.iter().filter(|switch| !switch.machine).count();
        let machine = SWITCHES.iter().filter(|switch| switch.machine).count();
        assert_eq!(user, 2, "AppCaptureEnabled and GameDVR_Enabled");
        assert_eq!(machine, 2, "the policy pair");
    }

    #[test]
    fn the_elevated_script_names_only_the_machine_values() {
        let off = policy_script(false);
        assert!(off.contains("AllowGameDVR"));
        assert!(off.contains(POLICY_MANAGER));
        assert!(off.contains(" -Value 0 "));
        // The user's own switches are written unelevated and must not need an administrator.
        assert!(!off.contains("AppCaptureEnabled"));
        assert!(!off.contains("GameDVR_Enabled"));
        assert!(policy_script(true).contains(" -Value 1 "));
        // Nothing an administrator runs may be built from anything but these two keys.
        assert_eq!(off.matches("HKEY_LOCAL_MACHINE").count(), 4);
    }

    #[test]
    fn a_missing_value_reads_as_none_rather_than_an_error() {
        // A key nobody has ever written is not a failure: `capture` then falls back to Windows'
        // own default, which is what makes a toggle on a fresh machine start from the truth.
        assert_eq!(
            read(
                HKEY_CURRENT_USER,
                "Software\\Oczi\\NoSuchKey",
                "NoSuchValue",
                false
            ),
            None
        );
    }
}
