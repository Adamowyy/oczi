// The Windows settings that are not a package and not a startup entry: the switches people go
// looking for in Settings and then give up on.
//
// Everything here is a table, not a module per setting. One row says where the value lives, what it
// reads when the switch is on and when it is off, and whether writing it needs an administrator —
// so the window is a list, and a new setting is one row plus its two strings.
//
// Two rules the table keeps:
//
// * `on` always means "the switch is on", whatever the underlying number is. Bing search is off at
//   zero, the Copilot kill-switch is on at one; the row carries the polarity so the window never has
//   to think about it.
// * A value Windows has never written is not a lie to be corrected — it is the default, so an
//   absent value reads as "off" and `reset` puts it back by deleting it.
//
// Every change is recorded with the value it replaced, which is what makes "undo" honest: nothing
// is guessed back, the old number is written again.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_SET_VALUE};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::elevate;
use crate::log;
use crate::registry;

// ── Where these live ──────────────────────────────────────────────────────────

const ADVANCED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced";
const CONTENT_DELIVERY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\ContentDeliveryManager";
const SEARCH: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Search";
const COPILOT_POLICY: &str = "Software\\Policies\\Microsoft\\Windows\\WindowsCopilot";
/// The CLSID Explorer reads the context menu from; an empty string value means "use the old one".
const CONTEXT_MENU: &str =
    "Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\\InprocServer32";
const TELEMETRY_POLICY: &str = "SOFTWARE\\Policies\\Microsoft\\Windows\\DataCollection";

/// What kind of value a row is. Copy because the table is a const and every reader takes it by
/// value: a two-word enum, cheaper to duplicate than to borrow.
#[derive(Clone, Copy)]
enum Kind {
    /// A DWORD switch, with the number each state wants.
    Dword { on: u32, off: u32 },
    /// An empty string that exists or does not: Windows reads the classic context menu from
    /// exactly this, and there is no number to write.
    Marker,
}

/// One setting the window can offer.
struct Tweak {
    id: &'static str,
    /// Which block it is drawn in: `explorer`, `taskbar`, `privacy`.
    group: &'static str,
    /// Machine hive: writing it needs an administrator, and the window says so.
    machine: bool,
    path: &'static str,
    name: &'static str,
    kind: Kind,
    /// Explorer has to restart before Windows reads the new value.
    restart: bool,
}

/// The catalogue, in the order the window draws it. Only what is in here is ever offered.
const TWEAKS: &[Tweak] = &[
    // ── Explorer ─────────────────────────────────────────────────────────────
    Tweak {
        id: "context.classic",
        group: "explorer",
        machine: false,
        path: CONTEXT_MENU,
        name: "",
        kind: Kind::Marker,
        restart: true,
    },
    Tweak {
        id: "explorer.recent",
        group: "explorer",
        machine: false,
        path: ADVANCED,
        name: "ShowRecent",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: true,
    },
    Tweak {
        id: "explorer.frequent",
        group: "explorer",
        machine: false,
        path: ADVANCED,
        name: "ShowFrequent",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: true,
    },
    Tweak {
        id: "explorer.ads",
        group: "explorer",
        machine: false,
        path: ADVANCED,
        name: "ShowSyncProviderNotifications",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: false,
    },
    // ── Taskbar ──────────────────────────────────────────────────────────────
    Tweak {
        id: "taskbar.widgets",
        group: "taskbar",
        machine: false,
        path: ADVANCED,
        name: "TaskbarDa",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: true,
    },
    Tweak {
        id: "taskbar.copilot",
        group: "taskbar",
        machine: false,
        path: ADVANCED,
        name: "ShowCopilotButton",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: true,
    },
    Tweak {
        id: "taskbar.search",
        group: "taskbar",
        machine: false,
        path: ADVANCED,
        // 0 hidden, 1 icon, 2 box. "On" is the icon: a search that is there without eating the
        // taskbar, which is what the box costs on a small screen.
        name: "SearchboxTaskbarMode",
        kind: Kind::Dword { on: 1, off: 2 },
        restart: true,
    },
    // ── Privacy ──────────────────────────────────────────────────────────────
    Tweak {
        id: "privacy.bing",
        group: "privacy",
        machine: false,
        path: SEARCH,
        name: "BingSearchEnabled",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: true,
    },
    Tweak {
        id: "privacy.copilot",
        group: "privacy",
        machine: false,
        path: COPILOT_POLICY,
        name: "TurnOffWindowsCopilot",
        kind: Kind::Dword { on: 1, off: 0 },
        restart: true,
    },
    Tweak {
        id: "privacy.silentapps",
        group: "privacy",
        machine: false,
        path: CONTENT_DELIVERY,
        name: "SilentInstalledAppsEnabled",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: false,
    },
    Tweak {
        id: "privacy.suggestions",
        group: "privacy",
        machine: false,
        path: CONTENT_DELIVERY,
        name: "SubscribedContent-338393Enabled",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: false,
    },
    Tweak {
        id: "privacy.softlanding",
        group: "privacy",
        machine: false,
        path: CONTENT_DELIVERY,
        name: "SoftLandingEnabled",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: false,
    },
    Tweak {
        id: "privacy.telemetry",
        group: "privacy",
        machine: true,
        path: TELEMETRY_POLICY,
        name: "AllowTelemetry",
        kind: Kind::Dword { on: 0, off: 1 },
        restart: false,
    },
];

// ── What the window shows ─────────────────────────────────────────────────────

/// One row as Windows has it right now.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TweakState {
    pub id: String,
    pub group: String,
    /// The switch: `true` when the tweak is active.
    pub on: bool,
    /// The value lives in the machine hive: writing it will ask for an administrator.
    pub admin: bool,
    /// The last attempt needed one and did not get it.
    pub needs_admin: bool,
    /// Explorer has to restart before this shows up.
    pub restart: bool,
}

/// One change this app made, with the value it replaced — `None` meaning the value was not there.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub id: String,
    /// Local wall-clock seconds.
    pub at: i64,
    pub was: Option<i64>,
    /// What the switch was turned to.
    pub on: bool,
}

fn find(id: &str) -> Result<&'static Tweak, String> {
    TWEAKS.iter().find(|tweak| tweak.id == id).ok_or_else(|| {
        // i18n-ok: shown as it is in the Debloat window.
        format!("Nie znam ustawienia {id}.")
    })
}

fn root_of(tweak: &Tweak) -> windows::Win32::System::Registry::HKEY {
    if tweak.machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    }
}

/// The raw value, for history and undo: a DWORD as it is, a marker as 1 or nothing.
fn current(tweak: &Tweak) -> Option<i64> {
    match tweak.kind {
        Kind::Dword { .. } => {
            registry::read_dword(root_of(tweak), tweak.path, tweak.name, tweak.machine)
                .map(|value| value as i64)
        }
        Kind::Marker => {
            registry::value_exists(root_of(tweak), tweak.path, tweak.name, tweak.machine)
                .then_some(1)
        }
    }
}

/// Whether the switch is on. An absent value is Windows' default, which for every row here is the
/// state the tweak exists to change.
fn is_on(tweak: &Tweak) -> bool {
    match tweak.kind {
        Kind::Dword { on, .. } => current(tweak) == Some(on as i64),
        Kind::Marker => current(tweak).is_some(),
    }
}

fn write(tweak: &Tweak, on: bool) -> bool {
    match tweak.kind {
        Kind::Dword { on: on_value, off } => {
            let value = if on { on_value } else { off };
            registry::write_dword(root_of(tweak), tweak.path, tweak.name, tweak.machine, value)
        }
        Kind::Marker => {
            if on {
                registry::write_empty_string(root_of(tweak), tweak.path, tweak.name, tweak.machine)
            } else {
                registry::delete_value(root_of(tweak), tweak.path, tweak.name, tweak.machine)
            }
        }
    }
}

/// Every row, as Windows has it now.
pub fn list() -> Vec<TweakState> {
    TWEAKS
        .iter()
        .map(|tweak| TweakState {
            id: tweak.id.to_string(),
            group: tweak.group.to_string(),
            on: is_on(tweak),
            admin: tweak.machine,
            needs_admin: false,
            restart: tweak.restart,
        })
        .collect()
}

/// Turns one on or off and answers with the whole table re-read, so the window never merges state.
pub fn set(id: &str, on: bool) -> Result<Vec<TweakState>, String> {
    let tweak = find(id)?;
    let was = current(tweak);
    let written = write(tweak, on);
    if written {
        record(tweak.id, was, on);
        log::line(format!("tweak {}: {} (was {:?})", tweak.id, on, was));
    } else if !tweak.machine {
        // A value in the user's own hive that refuses to be written is a real failure.
        // i18n-ok: shown as it is in the Debloat window.
        return Err(format!("Nie udało się zapisać {}.", tweak.id));
    }
    // The reading decides: a machine value that stayed put is reported, not retried.
    let landed = is_on(tweak);
    let mut states = list();
    if let Some(state) = states.iter_mut().find(|state| state.id == tweak.id) {
        state.needs_admin = tweak.machine && landed != on;
        if state.needs_admin {
            log::line(format!("tweak {}: needs an administrator", tweak.id));
        }
    }
    Ok(states)
}

/// Back to Windows' own default: the value is deleted rather than set to a guessed number.
pub fn reset(id: &str) -> Result<Vec<TweakState>, String> {
    let tweak = find(id)?;
    let was = current(tweak);
    registry::delete_value(root_of(tweak), tweak.path, tweak.name, tweak.machine);
    record(tweak.id, was, false);
    log::line(format!("tweak {}: reset (was {:?})", tweak.id, was));
    Ok(list())
}

/// The same write, through one administrator prompt — only ever for a machine-level row.
pub fn set_elevated(id: &str, on: bool) -> Result<Vec<TweakState>, String> {
    let tweak = find(id)?;
    if !tweak.machine {
        return Err({
            // i18n-ok: shown as it is in the Debloat window.
            format!("{} nie potrzebuje administratora.", tweak.id)
        });
    }
    let was = current(tweak);
    elevate::run_elevated(&elevated_script(tweak, on))?;
    // The elevated process runs on its own; wait for the registry to show it rather than claiming
    // it before Windows has written it.
    let mut waited = 0u64;
    while waited < 30_000 {
        std::thread::sleep(Duration::from_millis(500));
        waited += 500;
        if is_on(tweak) == on {
            break;
        }
    }
    if is_on(tweak) == on {
        record(tweak.id, was, on);
        log::line(format!("tweak {}: {} with an administrator", tweak.id, on));
        return Ok(list());
    }
    Err({
        // i18n-ok: shown as it is in the Debloat window.
        "Poziom maszyny nie zmienił się — sprawdź zgodę administratora.".to_string()
    })
}

/// The script the administrator runs: one value, and nothing else.
fn elevated_script(tweak: &Tweak, on: bool) -> String {
    let path = format!("Registry::HKEY_LOCAL_MACHINE\\{}", tweak.path);
    match tweak.kind {
        Kind::Dword { on: on_value, off } => {
            let value = if on { on_value } else { off };
            format!(
                "$ErrorActionPreference='Stop'; New-Item -Path '{path}' -Force | Out-Null; \
                 Set-ItemProperty -Path '{path}' -Name '{}' -Value {value} -Type DWord",
                tweak.name
            )
        }
        Kind::Marker => {
            if on {
                format!(
                    "$ErrorActionPreference='Stop'; New-Item -Path '{path}' -Force | Out-Null; \
                     Set-ItemProperty -Path '{path}' -Name '' -Value '' -Type String"
                )
            } else {
                format!("$ErrorActionPreference='Stop'; Remove-ItemProperty -Path '{path}' -Name '' -ErrorAction SilentlyContinue")
            }
        }
    }
}

// ── The history ───────────────────────────────────────────────────────────────

fn history_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("tweaks.json")
}

fn history() -> Vec<Change> {
    match std::fs::read(history_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::line(format!("tweak history unreadable ({e}), starting empty"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save(list: &[Change]) {
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        if let Err(e) = std::fs::write(history_path(), json) {
            log::line(format!("tweak history not saved: {e}"));
        }
    }
}

fn record(id: &str, was: Option<i64>, on: bool) {
    let mut list = history();
    list.push(Change {
        id: id.to_string(),
        at: crate::reminders::now_local(),
        was,
        on,
    });
    // A list nobody scrolls is not a history; the last two hundred is a year of fiddling.
    if list.len() > 200 {
        let drop = list.len() - 200;
        list.drain(..drop);
    }
    save(&list);
}

/// Everything this app changed, newest first.
pub fn changes() -> Vec<Change> {
    let mut list = history();
    list.sort_by(|a, b| b.at.cmp(&a.at));
    list
}

/// Puts back the value a change replaced, and forgets that change so the next undo walks further
/// back. A value that was not there before is deleted, which is how Windows spells "default".
pub fn undo(id: &str) -> Result<Vec<TweakState>, String> {
    let tweak = find(id)?;
    let mut changes = history();
    let Some(index) = changes.iter().rposition(|change| change.id == id) else {
        return Err({
            // i18n-ok: shown as it is in the Debloat window.
            format!("Nie ma czego cofnąć dla {id}.")
        });
    };
    let was = changes.remove(index).was;
    let restored = match (tweak.kind, was) {
        (Kind::Marker, Some(_)) => {
            registry::write_empty_string(root_of(tweak), tweak.path, tweak.name, tweak.machine)
        }
        (Kind::Marker, None) => {
            registry::delete_value(root_of(tweak), tweak.path, tweak.name, tweak.machine)
        }
        (Kind::Dword { .. }, Some(value)) => registry::write_dword(
            root_of(tweak),
            tweak.path,
            tweak.name,
            tweak.machine,
            value as u32,
        ),
        (Kind::Dword { .. }, None) => {
            registry::delete_value(root_of(tweak), tweak.path, tweak.name, tweak.machine)
        }
    };
    if !restored {
        // i18n-ok: shown as it is in the Debloat window.
        return Err(format!("Nie udało się przywrócić {id}."));
    }
    save(&changes);
    log::line(format!("tweak {id}: undone"));
    Ok(list())
}

// ── The one thing a restart is for ────────────────────────────────────────────

/// Restarts Explorer, because half of these settings are only read when it starts. The user asked
/// for it by name in the window — nothing here calls it on its own.
pub fn restart_explorer() -> Result<(), String> {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW.0)
        .output();
    std::thread::sleep(Duration::from_millis(800));
    Command::new("explorer.exe")
        .creation_flags(CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|e| format!("explorer: {e}"))?;
    log::line("explorer restarted from the tweaks tab".to_string());
    Ok(())
}

/// Whether the current process can write the machine hive at all — a cheap check the window uses
/// to say "this will ask for an administrator" before the click. It only opens the key for writing
/// and closes it again: nothing is written to answer the question.
pub fn is_elevated() -> bool {
    registry::open(HKEY_LOCAL_MACHINE, TELEMETRY_POLICY, true, KEY_SET_VALUE).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_a_unique_id_and_a_known_group() {
        let mut ids: Vec<&str> = TWEAKS.iter().map(|tweak| tweak.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "an id is written twice");
        for tweak in TWEAKS {
            assert!(
                matches!(tweak.group, "explorer" | "taskbar" | "privacy"),
                "{} has group {}",
                tweak.id,
                tweak.group
            );
            assert!(!tweak.name.is_empty() || matches!(tweak.kind, Kind::Marker));
        }
    }

    #[test]
    fn on_and_off_are_never_the_same_number() {
        for tweak in TWEAKS {
            if let Kind::Dword { on, off } = tweak.kind {
                assert_ne!(on, off, "{} would write the same value both ways", tweak.id);
            }
        }
    }

    #[test]
    fn the_polarity_is_what_the_window_would_expect() {
        // Bing search off means zero; the Copilot kill-switch on means one. Both rows say "on" in
        // the window when the quiet state is in place, however Windows spells it.
        let bing = TWEAKS.iter().find(|t| t.id == "privacy.bing").unwrap();
        let copilot = TWEAKS.iter().find(|t| t.id == "privacy.copilot").unwrap();
        assert_eq!(bing.kind_on(), 0);
        assert_eq!(copilot.kind_on(), 1);
    }

    #[test]
    fn an_unknown_id_is_refused_rather_than_guessed() {
        assert!(find("privacy.something.else").is_err());
        assert!(find("").is_err());
        assert!(find("privacy.bing").is_ok());
    }

    #[test]
    fn the_elevated_script_names_only_the_row_it_was_given() {
        let telemetry = TWEAKS.iter().find(|t| t.id == "privacy.telemetry").unwrap();
        // This row is the one with inverted polarity: its "on" is zero, so the switch being off is
        // the value one. The script has to follow the row, not the word.
        let on = elevated_script(telemetry, true);
        let off = elevated_script(telemetry, false);
        assert!(on.contains("AllowTelemetry"));
        assert!(on.contains("-Value 0 "));
        assert!(off.contains("-Value 1 "));
        assert!(on.contains("HKEY_LOCAL_MACHINE"));
        // A user-level row has no business in a script an administrator runs.
        assert!(!on.contains("ShowRecent"));
    }

    impl Tweak {
        /// The number the row writes when the switch goes on, for the polarity test above.
        fn kind_on(&self) -> u32 {
            match self.kind {
                Kind::Dword { on, .. } => on,
                Kind::Marker => 1,
            }
        }
    }
}
