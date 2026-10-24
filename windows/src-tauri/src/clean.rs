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
}

// The shell entry point takes a HWND and a root path, both null here on purpose: every drive, no
// window. The import is at the bottom so the module reads top-down.
use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
