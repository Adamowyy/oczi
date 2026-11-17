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

/// Leftovers a driver install, a Windows update, an installer and a dev toolchain leave behind,
/// measured apart from the two piles above: shader cache is not something a button called "empty
/// both" may take with it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Residue {
    /// NVIDIA's downloaded driver packages and the older package in the driver store, summed.
    pub nvidia: u64,
    /// The payloads Windows Update has already installed, still sitting in `SoftwareDistribution`.
    pub update: u64,
    /// `ProgramData\Package Cache` and `%LOCALAPPDATA%\Package Cache` — the installers kept for a
    /// repair, which nothing rebuilds.
    pub package_cache: u64,
    /// Packages in `%SystemRoot%\Installer` that no registered product names any more. Only an
    /// administrator can read the registry that decides it, so a plain process reads zero here.
    pub msi: u64,
    /// Cargo's downloaded crates, npm's and uv's caches — re-downloaded on the next build.
    pub dev: u64,
}

/// What one of the residue sweeps actually freed, measured the same way.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidueFreed {
    pub nvidia: u64,
    pub update: u64,
    pub package_cache: u64,
    pub msi: u64,
    pub dev: u64,
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
        let program = PathBuf::from(program);
        out.push(program.join("NVIDIA Corporation").join("Downloader"));
        // Every driver update the NVIDIA App downloads and unpacks a fresh copy here.
        out.push(
            program
                .join("NVIDIA Corporation")
                .join("NVIDIA App")
                .join("UpdateFramework"),
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

/// `Package Cache`: the installers an MSI-based setup keeps so it can be repaired or changed.
/// Nothing ever cleans them up, and a removed program leaves its own copy behind.
fn package_cache_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(program) = std::env::var_os("ProgramData") {
        out.push(PathBuf::from(program).join("Package Cache"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Package Cache"));
    }
    out.retain(|path| path.is_dir());
    out
}

/// The caches a dev toolchain can rebuild without asking: cargo's downloaded crates (source and
/// archive alike), npm's metadata, uv's wheels. Nothing is installed here, so nothing breaks.
fn dev_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        out.push(local.join("npm-cache"));
        out.push(local.join("uv").join("cache"));
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        out.push(PathBuf::from(roaming).join("npm-cache"));
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let registry = PathBuf::from(home).join(".cargo").join("registry");
        out.push(registry.join("cache"));
        out.push(registry.join("src"));
    }
    out.retain(|path| path.is_dir());
    out
}

/// Where Windows keeps one copy of every package an installer has ever run.
fn msi_store() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("Installer")
}

/// Every package a registered product still names, lowercased. Readable without a prompt — the
/// machine's own hive is — and `None` when nothing came back, because an empty set would make every
/// file in the store look like an orphan.
fn msi_reference_set() -> Option<std::collections::HashSet<String>> {
    let script = "$ErrorActionPreference='SilentlyContinue';\
         foreach($root in @('HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData','HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData','HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData')){{\
           foreach($sid in (Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue)){{\
             foreach($prod in (Get-ChildItem -LiteralPath (Join-Path $sid.PSPath 'Products') -ErrorAction SilentlyContinue)){{\
               $lp=(Get-ItemProperty -LiteralPath (Join-Path $prod.PSPath 'InstallProperties') -Name LocalPackage -ErrorAction SilentlyContinue).LocalPackage;\
               if($lp){{[IO.Path]::GetFileName($lp).ToLower()}};\
               foreach($patch in (Get-ChildItem -LiteralPath (Join-Path $prod.PSPath 'Patches') -ErrorAction SilentlyContinue)){{\
                 foreach($prop in (Get-ItemProperty -LiteralPath $patch.PSPath -ErrorAction SilentlyContinue).PSObject.Properties){{\
                   $value=[string]$prop.Value;\
                   if($value -match '\\.(msp|msi)$'){{[IO.Path]::GetFileName($value).ToLower()}}\
                 }}\
               }}\
             }}\
           }}\
         }}";
    let text = match crate::debloat::powershell(script) {
        Ok(text) => text,
        Err(e) => {
            log::line(format!("clean: the installer references would not list: {e}"));
            return None;
        }
    };
    let set: std::collections::HashSet<String> = text
        .lines()
        .map(|line| line.trim().to_ascii_lowercase())
        .filter(|line| line.ends_with(".msi") || line.ends_with(".msp"))
        .collect();
    if set.is_empty() {
        return None;
    }
    Some(set)
}

/// The packages no product names any more, as (bytes, count). Anything written in the last week is
/// left out: a setup may be in the middle of installing.
fn msi_orphans() -> (u64, u64) {
    let Some(references) = msi_reference_set() else {
        return (0, 0);
    };
    let Ok(entries) = std::fs::read_dir(msi_store()) else {
        return (0, 0);
    };
    let week = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let mut bytes = 0u64;
    let mut count = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !(name.ends_with(".msi") || name.ends_with(".msp")) || references.contains(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let fresh = meta
            .modified()
            .ok()
            .and_then(|at| at.elapsed().ok())
            .map(|age| age < week)
            .unwrap_or(true);
        if fresh {
            continue;
        }
        bytes += meta.len();
        count += 1;
    }
    (bytes, count)
}

/// The part of an elevated script that takes the orphaned packages away. It reads the references
/// again as an administrator, so a stale window cannot hand it a name it would regret.
fn msi_sweep() -> String {
    format!(
        "$refs=New-Object 'System.Collections.Generic.HashSet[string]';\
         foreach($root in @('HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData','HKCU:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData','HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Installer\\UserData')){{\
           foreach($sid in (Get-ChildItem -LiteralPath $root -ErrorAction SilentlyContinue)){{\
             foreach($prod in (Get-ChildItem -LiteralPath (Join-Path $sid.PSPath 'Products') -ErrorAction SilentlyContinue)){{\
               $lp=(Get-ItemProperty -LiteralPath (Join-Path $prod.PSPath 'InstallProperties') -Name LocalPackage -ErrorAction SilentlyContinue).LocalPackage;\
               if($lp){{[void]$refs.Add([IO.Path]::GetFileName($lp).ToLower())}};\
               foreach($patch in (Get-ChildItem -LiteralPath (Join-Path $prod.PSPath 'Patches') -ErrorAction SilentlyContinue)){{\
                 foreach($prop in (Get-ItemProperty -LiteralPath $patch.PSPath -ErrorAction SilentlyContinue).PSObject.Properties){{\
                   $value=[string]$prop.Value;\
                   if($value -match '\\.(msp|msi)$'){{[void]$refs.Add([IO.Path]::GetFileName($value).ToLower())}}\
                 }}\
               }}\
             }}\
           }}\
         }};\
         foreach($f in (Get-ChildItem -LiteralPath '{store}' -File -Force -ErrorAction SilentlyContinue)){{\
           if($f.Extension -notin '.msi','.msp'){{continue}};\
           if($refs.Contains($f.Name.ToLower())){{continue}};\
           if($f.LastWriteTime -gt (Get-Date).AddDays(-7)){{continue}};\
           Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue\
         }};",
        store = msi_store().display().to_string().replace('\'', "''")
    )
}

fn msi_script() -> String {
    format!("$ErrorActionPreference='SilentlyContinue';{}", msi_sweep())
}

/// The older driver packages, the installer's own folders, the two `Package Cache`s and the
/// toolchain caches are all plain file work; only the driver store needs an administrator.
fn clear_package_cache() -> Result<ResidueFreed, String> {
    let dirs = package_cache_dirs();
    let before: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    let machine: Vec<&PathBuf> = dirs
        .iter()
        .filter(|dir| {
            std::env::var_os("ProgramData")
                .map(|program| dir.starts_with(PathBuf::from(program)))
                .unwrap_or(false)
        })
        .collect();
    let mut skipped = 0u64;
    for dir in dirs.iter().filter(|dir| !machine.contains(dir)) {
        skipped += empty_contents(dir);
    }
    if !machine.is_empty() {
        crate::elevate::run_elevated_wait(&package_cache_script(&machine))?;
    }
    let after: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    let freed = before.saturating_sub(after);
    log::line(format!(
        "clean package cache: {freed} B freed of {before} B in {} folders, {skipped} B kept",
        dirs.len()
    ));
    Ok(ResidueFreed {
        package_cache: freed,
        skipped: skipped + after,
        ..Default::default()
    })
}

/// The elevated half of the `Package Cache` sweep: the folder stays, its contents go.
fn package_cache_script(dirs: &[&PathBuf]) -> String {
    let mut script = String::from("$ErrorActionPreference='SilentlyContinue';");
    for dir in dirs {
        script.push_str(&format!(
            " if (Test-Path -LiteralPath '{path}') {{ Get-ChildItem -LiteralPath '{path}' -Force | Remove-Item -Recurse -Force }};",
            path = dir.display().to_string().replace('\'', "''")
        ));
    }
    script
}

/// Which packages no product names any more. Readable without a prompt, so the window shows a
/// number; deleting them is what needs the administrator.
fn clear_msi_orphans() -> Result<ResidueFreed, String> {
    let before = msi_orphans().0;
    if before == 0 {
        return Ok(ResidueFreed::default());
    }
    crate::elevate::run_elevated_wait(&msi_script())?;
    let after = msi_orphans().0;
    let freed = before.saturating_sub(after);
    log::line(format!(
        "clean msi orphans: {freed} B freed of {before} B"
    ));
    Ok(ResidueFreed {
        msi: freed,
        skipped: after,
        ..Default::default()
    })
}

/// The toolchain caches; nothing here needs a prompt.
fn clear_dev() -> Result<ResidueFreed, String> {
    let dirs = dev_dirs();
    let before: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    let mut skipped = 0u64;
    for dir in &dirs {
        skipped += empty_contents(dir);
    }
    let after: u64 = dirs.iter().map(|dir| size_of(dir)).sum();
    let freed = before.saturating_sub(after);
    log::line(format!(
        "clean dev caches: {freed} B freed of {before} B in {} folders",
        dirs.len()
    ));
    Ok(ResidueFreed {
        dev: freed,
        skipped: skipped + after,
        ..Default::default()
    })
}

/// What every pile weighs right now, by the same reading the rows show.
fn pile_sizes() -> Residue {
    Residue {
        nvidia: nvidia_residue(),
        update: update_dirs().iter().map(|dir| size_of(dir)).sum(),
        package_cache: package_cache_dirs().iter().map(|dir| size_of(dir)).sum(),
        msi: msi_orphans().0,
        dev: dev_dirs().iter().map(|dir| size_of(dir)).sum(),
    }
}

/// Everything in one pass: the folders a plain process may empty are emptied here, and one
/// administrator prompt carries the driver store, the update payloads, the machine's `Package Cache`
/// and the orphaned installers. One prompt instead of four is the whole point of this button.
fn clear_all() -> Result<ResidueFreed, String> {
    let before = pile_sizes();
    let mut skipped = 0u64;
    for dir in nvidia_dirs() {
        skipped += empty_contents(&dir);
    }
    let cache_dirs = package_cache_dirs();
    let machine_cache: Vec<&PathBuf> = cache_dirs
        .iter()
        .filter(|dir| {
            std::env::var_os("ProgramData")
                .map(|program| dir.starts_with(PathBuf::from(program)))
                .unwrap_or(false)
        })
        .collect();
    let user_cache: Vec<&PathBuf> = cache_dirs
        .iter()
        .filter(|dir| !machine_cache.iter().any(|machine| machine == dir))
        .collect();
    for dir in &user_cache {
        skipped += empty_contents(dir);
    }
    for dir in dev_dirs() {
        skipped += empty_contents(&dir);
    }

    let mut script = String::from("$ErrorActionPreference='SilentlyContinue';");
    let packages = older_packages();
    if !packages.is_empty() {
        for package in packages.iter().filter(|p| is_published(&p.published)) {
            script.push_str(&format!(
                " pnputil.exe /delete-driver {} /uninstall /force;",
                package.published
            ));
        }
    }
    let dirs = update_dirs();
    if !dirs.is_empty() {
        script.push_str(" Stop-Service -Name wuauserv,bits -Force;");
        for dir in &dirs {
            script.push_str(&format!(
                " if (Test-Path -LiteralPath '{path}') {{ Get-ChildItem -LiteralPath '{path}' -Force | Remove-Item -Recurse -Force }};",
                path = dir.display().to_string().replace('\'', "''")
            ));
        }
        script.push_str(" Start-Service -Name wuauserv,bits;");
    }
    if !machine_cache.is_empty() {
        script.push_str(&package_cache_script(&machine_cache));
    }
    script.push_str(&msi_sweep());
    crate::elevate::run_elevated_wait(&script)?;

    let after = pile_sizes();
    let freed = ResidueFreed {
        nvidia: before.nvidia.saturating_sub(after.nvidia),
        update: before.update.saturating_sub(after.update),
        package_cache: before.package_cache.saturating_sub(after.package_cache),
        msi: before.msi.saturating_sub(after.msi),
        dev: before.dev.saturating_sub(after.dev),
        skipped: skipped
            + after.nvidia
            + after.update
            + after.package_cache
            + after.msi
            + after.dev,
    };
    log::line(format!(
        "clean everything: {} B freed, {} B kept, {} old driver packages",
        freed.nvidia + freed.update + freed.package_cache + freed.msi + freed.dev,
        freed.skipped,
        packages.len()
    ));
    Ok(freed)
}

/// What the residue rows show before anything is clicked.
pub fn residue_scan() -> Residue {
    let sizes = pile_sizes();
    log::line(format!(
        "clean residue scan: {} B nvidia, {} B windows update, {} B package cache, {} B msi orphans, {} B dev caches",
        sizes.nvidia, sizes.update, sizes.package_cache, sizes.msi, sizes.dev
    ));
    sizes
}

/// Clears one of the piles, or all of them under one prompt. None of them touches the temp folder,
/// the Recycle Bin or a shader cache.
pub fn residue_run(kind: &str) -> Result<ResidueFreed, String> {
    match kind {
        "nvidia" => clear_nvidia(),
        "update" => clear_update(),
        "installer" => clear_package_cache(),
        "msi" => clear_msi_orphans(),
        "dev" => clear_dev(),
        "all" => clear_all(),
        other => Err({
            // i18n-ok: a programming mistake, not a sentence for the window.
            format!("unknown residue: {other}")
        }),
    }
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
        ..Default::default()
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
        ..Default::default()
    })
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

    /// Only caches: nothing a toolchain keeps as its own install may end up in the dev sweep.
    #[test]
    fn the_dev_sweep_names_caches_and_nothing_installed() {
        for dir in dev_dirs() {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            assert!(
                matches!(name.as_str(), "cache" | "src" | "npm-cache"),
                "not a cache: {}",
                dir.display()
            );
            assert!(!dir.ends_with(".cargo"), "the toolchains themselves stay");
            assert!(!dir.ends_with(".rustup"));
        }
    }

    /// The orphan sweep names the store it reads, leaves anything written this week alone, and still
    /// may not touch a shader cache.
    #[test]
    fn the_msi_sweep_stays_inside_the_installer_folder() {
        let script = msi_script();
        assert!(script.contains("Installer"));
        assert!(script.contains(".msi") && script.contains(".msp"));
        assert!(script.contains("AddDays(-7)"), "a fresh setup is not an orphan");
        assert!(script.contains("PSPath"), "a registry key's Name has no drive in it");
        assert!(!script.contains("DXCache") && !script.contains("GLCache"));
        // A script whose braces do not balance never parses, and a sweep that never runs reports
        // nothing while looking like it worked.
        assert_eq!(
            script.matches('{').count(),
            script.matches('}').count(),
            "unbalanced braces"
        );
    }

    /// A name that is not one of the piles is refused rather than guessed at.
    #[test]
    fn only_the_known_piles_have_names() {
        assert!(residue_run("nvidia cache").is_err());
        assert!(residue_run("").is_err());
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
