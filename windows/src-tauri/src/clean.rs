// Cleaning: the two piles that grow by themselves, measured before anything is deleted.
//
// This module answers one question and does one thing. The question is "how much is sitting in the
// temp folder and in the Recycle Bin right now" — walked and summed, so the number on the button is
// a measurement and not an estimate. The thing is emptying both, and then measuring again: what was
// freed is the difference, and files another program still holds open are counted as skipped rather
// than reported as gone.
//
// Nothing here needs an administrator: the user's own temp folder and that user's Recycle Bin are
// both theirs to empty. Windows' own temp folder is deliberately left alone — half of it is in use
// by the system, and deleting it under a running Windows is how a machine ends up worse.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

use crate::log;

/// What is lying around, in bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sizes {
    /// The user's `%TEMP%`, summed.
    pub temp: u64,
    /// Everything in the Recycle Bin, over all drives.
    pub recycle: u64,
    /// How many entries the Recycle Bin holds.
    pub recycle_items: u64,
}

/// The outcome of one sweep: what was freed, and what Windows would not let go.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Freed {
    pub temp: u64,
    pub recycle: u64,
    /// Files another program still holds open, left where they are.
    pub skipped: u64,
}

/// The user's own temp folder, as Windows spells it for this account.
fn temp_dir() -> Option<PathBuf> {
    std::env::var_os("TEMP")
        .or_else(|| std::env::var_os("TMP"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

/// Sums everything under `dir`, following one level of nesting through folders. Unreadable entries
/// count as zero rather than failing the walk: a locked file is not an error, it is a file in use.
fn size_of(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            total += size_of(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

/// The Recycle Bin's own accounting, asked of the shell rather than guessed from `$Recycle.Bin`,
/// which holds one folder per account and one per drive.
fn recycle() -> (u64, u64) {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };
    // A null path asks about every drive at once.
    let asked = unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) };
    if asked.is_err() {
        return (0, 0);
    }
    (info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64)
}

/// What the window shows before anything is clicked.
pub fn scan() -> Sizes {
    let temp = temp_dir().map(|dir| size_of(&dir)).unwrap_or(0);
    let (recycle, recycle_items) = recycle();
    log::line(format!(
        "clean scan: {temp} B temp, {recycle} B in {} recycled items",
        recycle_items
    ));
    Sizes {
        temp,
        recycle,
        recycle_items,
    }
}

/// Empties what it can and reports the difference it measured, not the difference it hoped for.
fn empty_temp() -> (u64, u64) {
    let Some(dir) = temp_dir() else {
        return (0, 0);
    };
    let before = size_of(&dir);
    let mut skipped = 0u64;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let result = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if result.is_err() {
            // In use by something that is running: leave it, and say how much stayed.
            skipped += if meta.is_dir() { 0 } else { meta.len() };
        }
    }
    let after = size_of(&dir);
    (before.saturating_sub(after), skipped)
}

/// Empties the Recycle Bin without the dialog, then measures both piles again.
pub fn run() -> Freed {
    let (temp, skipped) = empty_temp();
    let (recycle_before, _) = recycle();
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    let emptied = unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), 0x7) };
    if let Err(e) = emptied {
        log::line(format!("clean: recycle bin refused: {e}"));
    }
    let (recycle_after, _) = recycle();
    let freed = Freed {
        temp,
        recycle: recycle_before.saturating_sub(recycle_after),
        skipped,
    };
    log::line(format!(
        "clean run: {} B freed ({} B temp, {} B recycle), {} B skipped",
        freed.temp + freed.recycle,
        freed.temp,
        freed.recycle,
        freed.skipped
    ));
    freed
}

/// Leftovers a driver install and a Windows update leave behind, measured apart from the two piles
/// above: seven gigabytes of shader cache is not something a button called "empty both" may take
/// with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Residue {
    /// NVIDIA's shader caches and its downloaded driver packages, summed.
    pub nvidia: u64,
    /// The payloads Windows Update has already installed, still sitting in `SoftwareDistribution`.
    pub update: u64,
}

/// What one of the two residue sweeps actually freed, measured the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidueFreed {
    pub nvidia: u64,
    pub update: u64,
    /// Bytes the sweep could not take — a file held open, or a folder only an administrator may
    /// empty and no prompt was answered.
    pub skipped: u64,
}

/// Where a driver update parks its leftovers: the folders a downloaded and an unpacked installer
/// leave behind. The shader caches (`DXCache`, `GLCache`, `ComputeCache`) are deliberately not here
/// — they rebuild during play and are not something a cleanup may take. `Installer2` is what an
/// uninstall needs and `NGX` is the DLSS model set, so neither is a leftover either.
fn nvidia_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(program) = std::env::var_os("ProgramData") {
        out.push(
            PathBuf::from(program)
                .join("NVIDIA Corporation")
                .join("Downloader"),
        );
    }
    // Where the installer unpacks a package it was handed by hand.
    out.push(PathBuf::from("C:\\NVIDIA"));
    out.retain(|path| path.is_dir());
    out
}

/// Where Windows Update keeps what it has already applied or no longer needs: the payload folder
/// under `SoftwareDistribution`, the recovery-agent leftovers of the last update, and whatever a
/// feature upgrade left behind — the old installation and its staging folders. Readable only in part
/// for a plain process, which is why the number may read low here and the sweep asks for an
/// administrator.
fn update_dirs() -> Vec<PathBuf> {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let drive = root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("C:\\"));
    let mut out = vec![
        root.join("SoftwareDistribution").join("Download"),
        root.join("Windows.old"),
        drive.join("$WinREAgent"),
        drive.join("$WINDOWS.~BT"),
        drive.join("$Windows.~WS"),
    ];
    out.retain(|dir| dir.is_dir());
    out
}

/// A display driver Windows keeps in its driver store.
struct DriverPackage {
    /// The version the INF carries: four numbers, and the highest is the driver in use.
    version: (u32, u32, u32, u32),
    /// `oem88.inf` — the name `pnputil` deletes it by.
    published: String,
    /// Bytes on disk, summed.
    size: u64,
}

/// A version out of an INF or out of `pnputil`: `32.0.16.2124`.
fn parse_version(text: &str) -> Option<(u32, u32, u32, u32)> {
    let mut parts = text
        .trim()
        .split('.')
        .map(|part| part.trim().parse::<u32>().ok());
    let mut next = || parts.next().flatten();
    Some((next()?, next()?, next()?, next()?))
}

/// `pnputil /enum-drivers` prints one block per package, separated by blank lines. The block for an
/// NVIDIA display driver mentions `nvami.inf`; the published name is the `oem*.inf` in it and the
/// version is the four-number group. Read this way the parse does not depend on the labels Windows
/// prints, which are translated on a Polish machine.
fn published_names_from(text: &str) -> std::collections::HashMap<(u32, u32, u32, u32), String> {
    let mut out = std::collections::HashMap::new();
    let flat = text.replace('\r', "");
    for block in flat.split("\n\n") {
        if !block.to_ascii_lowercase().contains("nvami.inf") {
            continue;
        }
        let published = block.lines().find_map(|line| {
            line.split_whitespace()
                .map(|word| word.trim())
                .find(|word| {
                    let lower = word.to_ascii_lowercase();
                    lower.starts_with("oem") && lower.ends_with(".inf")
                })
                .map(str::to_string)
        });
        let version = block
            .lines()
            .find_map(|line| line.split_whitespace().find_map(parse_version));
        if let (Some(published), Some(version)) = (published, version) {
            out.insert(version, published);
        }
    }
    out
}

/// The published names in the driver store, asked of `pnputil` because that is the tool that can
/// later delete one.
fn published_names() -> std::collections::HashMap<(u32, u32, u32, u32), String> {
    match crate::debloat::powershell("pnputil /enum-drivers /class Display") {
        Ok(text) => published_names_from(&text),
        Err(e) => {
            log::line(format!("clean: pnputil would not list the display drivers: {e}"));
            Default::default()
        }
    }
}

/// Every NVIDIA display driver package in the store, with its version and size.
fn nvidia_packages() -> Vec<DriverPackage> {
    let root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let store = root
        .join("System32")
        .join("DriverStore")
        .join("FileRepository");
    let Ok(entries) = std::fs::read_dir(&store) else {
        return Vec::new();
    };
    let published = published_names();
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("nvami.inf_amd64") {
            continue;
        }
        let Ok(inf) = std::fs::read_to_string(entry.path().join("nvami.inf")) else {
            continue;
        };
        let version = inf.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("DriverVer")?.trim_start();
            let rest = rest.strip_prefix('=')?.trim();
            let (_, numbers) = rest.split_once(',')?;
            parse_version(numbers)
        });
        let Some(version) = version else { continue };
        let Some(published) = published.get(&version) else {
            continue;
        };
        out.push(DriverPackage {
            version,
            published: published.clone(),
            size: size_of(&entry.path()),
        });
    }
    out
}

/// The driver in use is the newest package; every other one is what an update left in the store —
/// the rollback copy Windows keeps until somebody says it is not needed.
fn older_packages() -> Vec<DriverPackage> {
    let mut all = nvidia_packages();
    let Some(newest) = all.iter().map(|package| package.version).max() else {
        return Vec::new();
    };
    all.retain(|package| package.version < newest);
    all
}

/// What the NVIDIA row measures: the older driver packages plus the two folders an installer
/// downloads and unpacks into.
fn nvidia_residue() -> u64 {
    let store: u64 = older_packages().iter().map(|package| package.size).sum();
    let downloads: u64 = nvidia_dirs().iter().map(|dir| size_of(dir)).sum();
    store + downloads
}

/// What the two residue rows show before anything is clicked.
pub fn residue_scan() -> Residue {
    let nvidia = nvidia_residue();
    let update: u64 = update_dirs().iter().map(|dir| size_of(dir)).sum();
    log::line(format!(
        "clean residue scan: {nvidia} B nvidia, {update} B windows update"
    ));
    Residue { nvidia, update }
}

/// Empties a folder's contents and keeps the folder itself: NVIDIA and Windows both expect their
/// cache folder to be there, and both rebuild what is missing. Returns what stayed behind.
fn empty_contents(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut skipped = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let result = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if result.is_err() {
            skipped += if meta.is_dir() {
                size_of(&path)
            } else {
                meta.len()
            };
        }
    }
    skipped
}

/// A published name as `pnputil` spells it. A string that came out of a program's output still has
/// to look like a driver name before it lands in an administrator's script.
fn is_published(name: &str) -> bool {
    let Some(number) = name
        .strip_prefix("oem")
        .and_then(|rest| rest.strip_suffix(".inf"))
    else {
        return false;
    };
    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
}

/// The script that takes the older driver packages out of the store: one `pnputil` call per package,
/// nothing else.
fn driver_script(packages: &[DriverPackage]) -> String {
    let mut script = String::from("$ErrorActionPreference='SilentlyContinue';");
    for package in packages.iter().filter(|p| is_published(&p.published)) {
        script.push_str(&format!(
            " pnputil.exe /delete-driver {} /uninstall /force;",
            package.published
        ));
    }
    script
}

/// The older driver packages and the installer's own folders. The store needs an administrator;
/// the download folders are the user's and go without a prompt.
fn clear_nvidia() -> Result<ResidueFreed, String> {
    let before = nvidia_residue();
    let packages = older_packages();
    if !packages.is_empty() {
        crate::elevate::run_elevated_wait(&driver_script(&packages))?;
    }
    let mut skipped = 0u64;
    for dir in nvidia_dirs() {
        skipped += empty_contents(&dir);
    }
    let after = nvidia_residue();
    let freed = before.saturating_sub(after);
    log::line(format!(
        "clean nvidia residue: {freed} B freed of {before} B ({} old packages), {skipped} B kept",
        packages.len()
    ));
    Ok(ResidueFreed {
        nvidia: freed,
        update: 0,
        skipped: skipped + after,
    })
}

/// The script an administrator runs to take the applied update payloads and the leftovers of the
/// last feature update. The update services are stopped first because Windows holds the payload
/// files open while they run, and started again afterwards so the next check for updates is not the
/// thing that breaks.
fn update_script(dirs: &[PathBuf]) -> String {
    let mut script = String::from("$ErrorActionPreference='SilentlyContinue'; Stop-Service -Name wuauserv,bits -Force;");
    for dir in dirs {
        script.push_str(&format!(
            " if (Test-Path -LiteralPath '{path}') {{ Get-ChildItem -LiteralPath '{path}' -Force | Remove-Item -Recurse -Force }};",
            path = dir.display()
        ));
    }
    script.push_str(" Start-Service -Name wuauserv,bits");
    script
}

/// Empties the update leftovers through one administrator prompt and measures what went. How much it
/// freed is the difference, not what the script hoped to delete.
fn clear_update() -> Result<ResidueFreed, String> {
    let dirs = update_dirs();
    if dirs.is_empty() {
        return Ok(ResidueFreed::default());
    }
    let before: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    crate::elevate::run_elevated_wait(&update_script(&dirs))?;
    let after: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    let freed = before.saturating_sub(after);
    log::line(format!(
        "clean update residue: {freed} B freed of {before} B in {} folders",
        dirs.len()
    ));
    Ok(ResidueFreed {
        nvidia: 0,
        update: freed,
        skipped: after,
    })
}

/// Clears one of the two piles. Two names, two buttons, and neither touches the temp folder or the
/// Recycle Bin.
pub fn residue_run(kind: &str) -> Result<ResidueFreed, String> {
    match kind {
        "nvidia" => clear_nvidia(),
        "update" => clear_update(),
        other => Err({
            // i18n-ok: a programming mistake, not a sentence for the window.
            format!("unknown residue: {other}")
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_the_sum_of_what_is_there() {
        let dir = std::env::temp_dir().join("oczi-clean-probe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(dir.join("nested").join("b.bin"), vec![0u8; 24]).unwrap();
        assert_eq!(size_of(&dir), 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_or_missing_folder_is_zero_and_not_an_error() {
        assert_eq!(size_of(Path::new("H:\\no\\such\\folder")), 0);
    }

    /// The administrator script gets the update folders and nothing else, and it puts the update
    /// services back the way it found them.
    #[test]
    fn the_update_script_stops_the_services_and_names_the_folders() {
        let dirs = [
            PathBuf::from("C:\\Windows\\SoftwareDistribution\\Download"),
            PathBuf::from("C:\\$WinREAgent"),
        ];
        let script = update_script(&dirs);
        assert!(script.contains("Stop-Service -Name wuauserv,bits -Force"));
        assert!(script.contains("Start-Service -Name wuauserv,bits"));
        assert!(script.contains("'C:\\Windows\\SoftwareDistribution\\Download'"));
        // The recovery-agent leftover sits in a folder whose name starts with `$`, which PowerShell
        // would expand outside the single quotes this script uses.
        assert!(script.contains("'C:\\$WinREAgent'"));
        // A sweep that names the Recycle Bin here would be the surprise the two buttons exist to
        // avoid.
        assert!(!script.contains("Recycle"));
    }

    /// Emptying keeps the folder: NVIDIA expects `DXCache` to be there and rebuilds what is missing.
    #[test]
    fn emptying_a_folder_takes_its_contents_and_leaves_it_standing() {
        let dir = std::env::temp_dir().join("oczi-residue-probe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 800]).unwrap();
        std::fs::write(dir.join("nested").join("b.bin"), vec![0u8; 224]).unwrap();
        assert_eq!(size_of(&dir), 1024);
        assert_eq!(empty_contents(&dir), 0);
        assert!(dir.is_dir(), "the cache folder itself has to stay");
        assert_eq!(size_of(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two names, two piles: anything else is a mistake in the caller, not something to guess at.
    #[test]
    fn an_unknown_residue_is_refused_rather_than_guessed() {
        assert!(residue_run("everything").is_err());
        assert!(!nvidia_dirs().iter().any(|dir| dir.ends_with("Installer2")));
        // Shader caches rebuild during play, so no sweep may name them.
        assert!(!nvidia_dirs().iter().any(|dir| {
            matches!(
                dir.file_name().and_then(|name| name.to_str()),
                Some("DXCache" | "GLCache" | "ComputeCache")
            )
        }));
    }

    #[test]
    fn a_version_is_four_numbers_or_nothing() {
        assert_eq!(parse_version("32.0.16.2124"), Some((32, 0, 16, 2124)));
        assert_eq!(parse_version(" 32.0.16.2124 "), Some((32, 0, 16, 2124)));
        assert_eq!(parse_version("1.2.3"), None);
        assert_eq!(parse_version("10/27/2026"), None);
    }

    /// Only the NVIDIA display blocks of `pnputil` become names — the AMD and spacedesk packages in
    /// the same list are somebody else's driver.
    #[test]
    fn only_the_nvidia_blocks_become_names() {
        let text = "Published Name:     oem88.inf\n\
                    Original Name:      nvami.inf\n\
                    Driver Version:     10/27/2026 32.0.16.2114\n\
                    \n\
                    Published Name:     oem74.inf\n\
                    Original Name:      spacedeskdisplay.inf\n\
                    Driver Version:     06/02/2025 10.2.398.176\n";
        let names = published_names_from(text);
        assert_eq!(
            names.get(&(32, 0, 16, 2114)).map(String::as_str),
            Some("oem88.inf")
        );
        assert_eq!(names.len(), 1, "a foreign driver became a name");
    }

    /// A name read out of a program's output still has to look like a driver name before it reaches
    /// an administrator's script.
    #[test]
    fn a_published_name_has_to_look_like_one() {
        assert!(is_published("oem88.inf"));
        assert!(!is_published("nvami.inf"));
        assert!(!is_published("oem.inf"));
        assert!(!is_published("oem88.inf; whoami"));
        let packages = [
            DriverPackage {
                version: (32, 0, 16, 2114),
                published: "oem88.inf".to_string(),
                size: 0,
            },
            DriverPackage {
                version: (32, 0, 16, 2100),
                published: "; Start-Process calc".to_string(),
                size: 0,
            },
        ];
        let script = driver_script(&packages);
        assert!(script.contains("pnputil.exe /delete-driver oem88.inf /uninstall /force"));
        assert!(!script.contains("Start-Process"));
    }
}

// The shell entry point takes a HWND and a root path, both null here on purpose: every drive, no
// window. The import is at the bottom so the module reads top-down.
use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
