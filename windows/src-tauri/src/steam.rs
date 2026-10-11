// Steam games, for `launch_app`.
//
// A game is not a file to run: Steam owns it and starts it. What identifies one is the app id
// in `appmanifest_<id>.acf`, listed per library in `libraryfolders.vdf`, and `steam://rungameid`
// is the handle that opens it the way the Steam client does.
//
// Everything here is a read, and only a game that is installed is ever offered.
//
// `oczi.exe --steam` prints what this finds and stops, so the reader can be checked against
// the machine instead of being trusted.

use std::path::{Path, PathBuf};

/// One installed game, as its manifest describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub appid: String,
    pub name: String,
}

/// The game matching a name, or nothing. Matching is the same shape as a program's: the exact
/// name first, then a name that contains what was asked for, then the form without the case
/// ending a dictated name carries.
pub fn installed(what: &str) -> Option<Game> {
    let games = all();
    let needle = what.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    if let Some(game) = games.iter().find(|g| g.name.to_lowercase() == needle) {
        return Some(game.clone());
    }
    if let Some(game) = games
        .iter()
        .find(|g| g.name.to_lowercase().contains(&needle))
    {
        return Some(game.clone());
    }
    for stem in crate::shell::spoken_stems(&needle) {
        if let Some(game) = games
            .iter()
            .find(|g| g.name.to_lowercase().contains(&stem))
        {
            return Some(game.clone());
        }
    }
    None
}

/// Every game installed in every Steam library on this machine.
pub fn all() -> Vec<Game> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for library in libraries(&root) {
        games.extend(games_in(&library.join("steamapps")));
    }
    games.sort_by(|a, b| a.name.cmp(&b.name));
    games
}

/// Starts a game through Steam's URL handler, the way clicking Play does.
pub fn open(game: &Game) -> Result<String, String> {
    let url = format!("steam://rungameid/{}", game.appid);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .arg("/C")
            .raw_arg(format!("start \"\" \"{url}\""))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not start {} through Steam: {e}", game.name))?;
    }
    #[cfg(not(windows))]
    std::process::Command::new("cmd")
        .arg("/C")
        .arg(format!("start \"\" \"{url}\""))
        .spawn()
        .map_err(|e| format!("Could not start {} through Steam: {e}", game.name))?;
    Ok(format!("Started {} ({url}).", game.name))
}

/// The Steam installation, from the registry — its own record of where it was put, which is
/// the only reliable one: this machine keeps Steam on a drive of its own, not in Program Files.
fn steam_root() -> Option<PathBuf> {
    for (hive, key) in [
        ("HKEY_CURRENT_USER", "Software\\Valve\\Steam"),
        ("HKEY_LOCAL_MACHINE", "SOFTWARE\\WOW6432Node\\Valve\\Steam"),
    ] {
        for value in ["SteamPath", "InstallPath"] {
            if let Some(path) = read_string(hive, key, value) {
                let path = PathBuf::from(path);
                if path.join("steamapps").is_dir() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Every library Steam knows about, the installation itself included. `libraryfolders.vdf`
/// is where a second drive is registered, so a game on `E:` is found without ever walking it.
///
/// The same library is listed twice otherwise: the registry says `D:\gry\steam` and the file
/// says `D:\Gry\Steam`, and a `PathBuf` comparison is case-sensitive even though Windows is
/// not — which showed up as every game in the main library appearing twice in `--steam`.
fn libraries(root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    let text = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf"))
        .unwrap_or_default();
    for (key, value) in pairs(&text) {
        if key.eq_ignore_ascii_case("path") {
            let path = PathBuf::from(value.replace("\\\\", "\\"));
            if path.join("steamapps").is_dir() && !dirs.iter().any(|d| same_dir(d, &path)) {
                dirs.push(path);
            }
        }
    }
    dirs
}

/// Whether two paths name the same folder on this machine: asked of the filesystem where it
/// can be, compared case-insensitively where it cannot.
fn same_dir(a: &Path, b: &Path) -> bool {
    if let (Ok(a), Ok(b)) = (a.canonicalize(), b.canonicalize()) {
        return a == b;
    }
    let key = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    key(a) == key(b)
}

/// The games in one `steamapps` folder, read from the manifests Steam keeps there.
fn games_in(steamapps: &Path) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(steamapps) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().to_string();
        if !file.starts_with("appmanifest_") || !file.ends_with(".acf") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(game) = game_from_manifest(&text) {
            games.push(game);
        }
    }
    games
}

/// One manifest as a game. A manifest without a name or an id is not one, and a game that is
/// not fully installed is left out: `StateFlags` carries the install state bit, and offering
/// a game that is only half downloaded would look like a failed launch.
fn game_from_manifest(text: &str) -> Option<Game> {
    let mut appid = None;
    let mut name = None;
    let mut flags = None;
    for (key, value) in pairs(text) {
        match key.as_str() {
            "appid" => appid = Some(value),
            "name" => name = Some(value),
            "StateFlags" => flags = value.parse::<u32>().ok(),
            _ => {}
        }
    }
    let (appid, name) = (appid?, name?);
    // 4 is "fully installed"; a manifest with an unreadable state is taken at face value.
    if flags.is_some_and(|f| f & 4 == 0) {
        return None;
    }
    if appid.is_empty() || name.trim().is_empty() {
        return None;
    }
    Some(Game { appid, name })
}

/// The key/value pairs of a Valve key-value file — `libraryfolders.vdf`, `appmanifest_*.acf`.
///
/// The format nests blocks (`"AppState" { … }`), so a quoted string is a key only when the
/// next thing after it is not `{`; a block's own key is skipped and everything inside it is
/// read as it comes. One linear scan therefore handles any depth, which is all this needs:
/// a manifest's fields sit one level down, a library's paths two.
fn pairs(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((token, tail)) = next_token(rest) {
        rest = tail;
        let trimmed = rest.trim_start();
        if trimmed.starts_with('{') {
            // A block: its key has no value of its own.
            rest = trimmed;
            continue;
        }
        let Some((value, tail)) = next_token(trimmed) else {
            break;
        };
        rest = tail;
        out.push((token, value));
    }
    out
}

/// The next quoted string and what follows it.
fn next_token(text: &str) -> Option<(String, &str)> {
    let start = text.find('"')? + 1;
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some((rest[..end].to_string(), &rest[end + 1..]))
}

/// One string value out of the registry, or nothing. Windows-only by nature; anywhere else
/// this is simply absent, and so is Steam's registry record.
#[cfg(windows)]
fn read_string(hive: &str, sub: &str, name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_SZ, REG_VALUE_TYPE,
    };

    let root: HKEY = match hive {
        "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        _ => HKEY_CURRENT_USER,
    };
    let wide = |text: &str| -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    };
    let sub = wide(sub);
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(root, PCWSTR(sub.as_ptr()), None, KEY_READ, &mut key) } != ERROR_SUCCESS
    {
        return None;
    }
    let name = wide(name);
    let mut kind = REG_VALUE_TYPE(0);
    let mut len = 0u32;
    let status = unsafe {
        RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, Some(&mut kind), None, Some(&mut len))
    };
    let mut out = None;
    if status == ERROR_SUCCESS && kind == REG_SZ && len > 1 {
        let mut data = vec![0u8; len as usize];
        let status = unsafe {
            RegQueryValueExW(
                key,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut len),
            )
        };
        if status == ERROR_SUCCESS {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let text = String::from_utf16_lossy(&units);
            let text = text.trim_end_matches('\0').trim().to_string();
            if !text.is_empty() {
                out = Some(text);
            }
        }
    }
    let _ = unsafe { RegCloseKey(key) };
    out
}

#[cfg(not(windows))]
fn read_string(_hive: &str, _sub: &str, _name: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact shape of this machine's own manifests (Dead by Daylight, Project Zomboid,
    /// Counter-Strike 2), so the reader is tested against the real file and not against a
    /// guess about the format.
    const DEAD_BY_DAYLIGHT: &str = "\"AppState\"\n{\n\t\"appid\"\t\t\"381210\"\n\t\"Universe\"\t\t\"1\"\n\t\"name\"\t\t\"Dead by Daylight\"\n\t\"StateFlags\"\t\t\"4\"\n\t\"installdir\"\t\t\"Dead by Daylight\"\n\t\"UserConfig\"\n\t{\n\t\t\"language\"\t\t\"polish\"\n\t}\n}\n";

    const LIBRARY_FOLDERS: &str = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Gry\\\\Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"381210\"\t\t\"0\"\n\t\t}\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"E:\\\\Gry\\\\Steam\"\n\t}\n}\n";

    #[test]
    fn reads_a_game_out_of_its_manifest() {
        let game = game_from_manifest(DEAD_BY_DAYLIGHT).expect("game");
        assert_eq!(game.appid, "381210");
        assert_eq!(game.name, "Dead by Daylight");
    }

    #[test]
    fn a_half_installed_game_is_not_offered() {
        // StateFlags 1026 = update required, not playable.
        let partial = DEAD_BY_DAYLIGHT.replace("\"StateFlags\"\t\t\"4\"", "\"StateFlags\"\t\t\"1026\"");
        assert!(game_from_manifest(&partial).is_none());
    }

    #[test]
    fn a_nested_block_is_not_mistaken_for_a_value() {
        // `"UserConfig" { … }` must not swallow the fields that follow it, and the block's
        // own key must not become a pair.
        let pairs = pairs(DEAD_BY_DAYLIGHT);
        assert!(pairs.contains(&("name".to_string(), "Dead by Daylight".to_string())));
        assert!(pairs.contains(&("language".to_string(), "polish".to_string())));
        assert!(!pairs.iter().any(|(k, _)| k == "UserConfig"));
    }

    #[test]
    fn finds_a_second_library_and_unescapes_the_path() {
        let pairs = pairs(LIBRARY_FOLDERS);
        let paths: Vec<String> = pairs
            .into_iter()
            .filter(|(k, _)| k == "path")
            .map(|(_, v)| v.replace("\\\\", "\\"))
            .collect();
        assert_eq!(paths, vec!["D:\\Gry\\Steam", "E:\\Gry\\Steam"]);
    }

    #[test]
    fn the_same_library_twice_is_one_library() {
        // How it actually comes out on a real machine: the registry says `D:\gry\steam` and
        // `libraryfolders.vdf` says `D:\Gry\Steam`. Reading both listed every game twice.
        use std::fs;
        let dir = std::env::temp_dir().join("oczi-steam-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("steamapps")).expect("create");
        let listed = dir.to_string_lossy().to_uppercase().replace('\\', "\\\\");
        let vdf = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{listed}\"\n\t}}\n}}\n"
        );
        fs::write(dir.join("steamapps").join("libraryfolders.vdf"), vdf).expect("write");
        assert_eq!(libraries(&dir).len(), 1, "the same folder in another case is not a second library");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_name_is_matched_the_way_it_is_spoken() {
        // The matching rules, exercised on the data rather than through the disk: exact name,
        // then contained, then without the case ending a dictated name carries.
        let games = vec![
            Game { appid: "381210".into(), name: "Dead by Daylight".into() },
            Game { appid: "730".into(), name: "Counter-Strike 2".into() },
        ];
        let find = |needle: &str| -> Option<String> {
            let needle = needle.to_lowercase();
            if let Some(g) = games.iter().find(|g| g.name.to_lowercase() == needle) {
                return Some(g.appid.clone());
            }
            if let Some(g) = games.iter().find(|g| g.name.to_lowercase().contains(&needle)) {
                return Some(g.appid.clone());
            }
            crate::shell::spoken_stems(&needle)
                .into_iter()
                .find_map(|s| {
                    games
                        .iter()
                        .find(|g| g.name.to_lowercase().contains(&s))
                        .map(|g| g.appid.clone())
                })
        };
        assert_eq!(find("dead by daylight").as_deref(), Some("381210"));
        assert_eq!(find("Daylight").as_deref(), Some("381210"));
        assert_eq!(find("counter-strike").as_deref(), Some("730"));
        assert_eq!(find("cs2"), None);
    }
}
