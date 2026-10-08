// What this PC really starts with Windows, read from Windows itself.

use std::collections::HashMap;
use std::path::Path;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_BINARY, REG_DWORD,
    REG_EXPAND_SZ, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

/// The three Run keys Windows offers, with the view each one has to be read in.
/// The 32-bit hive is reachable under `WOW6432Node` from a 64-bit process, which is
/// why its own view flag earns its place there.
const RUN_KEYS: &[(HKEY, &str, &str, u32)] = &[
    (
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
        "user",
        0,
    ),
    (
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run",
        "machine",
        KEY_WOW64_64KEY.0,
    ),
    (
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Run",
        "machine, 32-bit",
        KEY_WOW64_64KEY.0,
    ),
];

/// Where Task Manager keeps the on/off switch for a startup entry: one binary value
/// per name, whose first byte is 2 or 6 when it is on and 3 or 7 when it is off.
const SWITCH_KEYS: &[(HKEY, &str, u32)] = &[
    (
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run",
        0,
    ),
    (
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run32",
        0,
    ),
    (
        HKEY_CURRENT_USER,
        "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\StartupFolder",
        0,
    ),
    (
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run",
        KEY_WOW64_64KEY.0,
    ),
    (
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\StartupFolder",
        KEY_WOW64_64KEY.0,
    ),
];

/// A key that closes itself, so an early return in the readers below cannot leak it.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open(root: HKEY, sub: &str, view: u32) -> Option<Key> {
    let path = wide(sub);
    let mut out = HKEY::default();
    let flags = REG_SAM_FLAGS(KEY_READ.0 | view);
    if unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, flags, &mut out) } != ERROR_SUCCESS
    {
        return None;
    }
    Some(Key(out))
}

/// Value names in a key. Reading the data of each one is left to `read`, so a key
/// with many big values costs nothing here.
fn names(key: &Key) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buffer = [0u16; 512];
        let mut len = buffer.len() as u32;
        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buffer[..len as usize]));
        index += 1;
    }
    out
}

/// Subkey names in a key, the services key is a folder of them.
fn subkeys(key: &Key) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0u32;
    loop {
        let mut buffer = [0u16; 512];
        let mut len = buffer.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                key.0,
                index,
                Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                &mut len,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buffer[..len as usize]));
        index += 1;
    }
    out
}

/// One value as raw bytes: its type and its data. The size is asked for first, so a
/// value larger than any fixed buffer still comes back whole.
fn read(key: &Key, name: &str) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let path = wide(name);
    let mut kind = REG_VALUE_TYPE(0);
    let mut len = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(path.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS && status != ERROR_MORE_DATA {
        return None;
    }
    let mut data = vec![0u8; len as usize];
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(path.as_ptr()),
            None,
            Some(&mut kind),
            Some(data.as_mut_ptr()),
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    data.truncate(len as usize);
    Some((kind, data))
}

/// `%ProgramFiles%\x` is written into the registry, and `C:\Program Files\x` is what
/// the disk has to be asked about.
fn expand(text: &str) -> String {
    let source = wide(text);
    let needed = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), None) };
    if needed == 0 {
        return text.to_string();
    }
    let mut out = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(source.as_ptr()), Some(&mut out)) };
    if written == 0 {
        return text.to_string();
    }
    let end = out.iter().position(|c| *c == 0).unwrap_or(out.len());
    String::from_utf16_lossy(&out[..end])
}

fn text_of(kind: REG_VALUE_TYPE, data: &[u8]) -> Option<String> {
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return None;
    }
    let mut utf16: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    while utf16.last() == Some(&0) {
        utf16.pop();
    }
    let raw = String::from_utf16_lossy(&utf16);
    // Expanded either way: half the entries are written with `%ProgramFiles%` in them
    // and the other half with the real path, and a plain string with no `%` in it
    // comes back unchanged.
    Some(expand(&raw))
}

fn dword_of(kind: REG_VALUE_TYPE, data: &[u8]) -> Option<u32> {
    if kind != REG_DWORD || data.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

/// Task Manager's switch, read out of its one-byte form: 2 and 6 mean on, 3 and 7
/// mean off, anything else is a value this app does not understand and stays out of
/// the answer entirely.
fn switch_state(byte: u8) -> Option<bool> {
    match byte {
        2 | 6 => Some(true),
        3 | 7 => Some(false),
        _ => None,
    }
}

/// The on/off switch per name, keyed in lower case because Windows matches startup
/// names without case.
fn switches() -> HashMap<String, bool> {
    let mut out: HashMap<String, bool> = HashMap::new();
    for (root, path, view) in SWITCH_KEYS {
        let Some(key) = open(*root, path, *view) else {
            continue;
        };
        for name in names(&key) {
            if name.is_empty() {
                continue;
            }
            let Some((kind, data)) = read(&key, &name) else {
                continue;
            };
            if kind != REG_BINARY || data.is_empty() {
                continue;
            }
            if let Some(on) = switch_state(data[0]) {
                out.entry(name.to_lowercase()).or_insert(on);
            }
        }
    }
    out
}

/// The executable a command line starts: whatever is inside the quotes, or the first
/// word otherwise. Only an absolute path is handed back, a bare name means PATH and
/// guessing at it would be worse than saying nothing.
fn program_of(command: &str) -> Option<String> {
    let trimmed = command.trim();
    let raw = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        trimmed.split_whitespace().next()?
    };
    let path = expand(raw);
    let looks_absolute = path.len() > 3 && (path.as_bytes()[1] == b':' || path.starts_with("\\\\"));
    looks_absolute.then_some(path)
}

/// How the program behind an entry looks on the disk.
fn file_state(program: Option<&str>) -> &'static str {
    match program {
        Some(path) if Path::new(path).exists() => "file present",
        Some(_) => "file MISSING",
        None => "file unknown",
    }
}

fn run_entries() -> Vec<String> {
    let switch = switches();
    let mut out = Vec::new();
    for (root, path, label, view) in RUN_KEYS {
        let Some(key) = open(*root, path, *view) else {
            continue;
        };
        for name in names(&key) {
            if name.is_empty() {
                continue;
            }
            let command = read(&key, &name)
                .and_then(|(kind, data)| text_of(kind, &data))
                .unwrap_or_default();
            // No switch recorded means Windows has it on: the switch only appears
            // once something has been flipped in Task Manager.
            let on = switch.get(&name.to_lowercase()).copied().unwrap_or(true);
            let program = program_of(&command);
            out.push(format!(
                "- {name} ({label}) {}, {} — {}",
                if on { "on" } else { "off" },
                file_state(program.as_deref()),
                command.trim()
            ));
        }
    }
    out
}

/// The Startup folders: shortcuts, whose target is not read here, .lnk resolution
/// is a COM job, and the name is what the Task Manager list shows anyway.
fn folder_entries() -> Vec<String> {
    let switch = switches();
    let mut out = Vec::new();
    for (dir, label) in [
        (
            "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
            "user",
        ),
        (
            "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
            "machine",
        ),
    ] {
        let full = expand(dir);
        let Ok(entries) = std::fs::read_dir(&full) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.to_lowercase() == "desktop.ini" {
                continue;
            }
            let on = switch.get(&name.to_lowercase()).copied().unwrap_or(true);
            out.push(format!(
                "- {name} ({label}) {}",
                if on { "on" } else { "off" }
            ));
        }
    }
    out
}

/// Services set to start on their own, Microsoft's own left out: their paths say
/// nothing a person can act on, and there are forty of them.
fn service_entries() -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = open(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\CurrentControlSet\\Services",
        KEY_WOW64_64KEY.0,
    ) else {
        return out;
    };
    for name in subkeys(&root) {
        let Some(key) = open(
            HKEY_LOCAL_MACHINE,
            &format!("SYSTEM\\CurrentControlSet\\Services\\{name}"),
            KEY_WOW64_64KEY.0,
        ) else {
            continue;
        };
        let start = read(&key, "Start").and_then(|(kind, data)| dword_of(kind, &data));
        // 0 boot, 1 system, 2 auto. Anything else waits to be asked.
        if !matches!(start, Some(0..=2)) {
            continue;
        }
        let Some(command) = read(&key, "ImagePath").and_then(|(kind, data)| text_of(kind, &data))
        else {
            continue;
        };
        let low = command.to_lowercase();
        // Windows' own plumbing, and Defender with it: forty services whose paths say
        // nothing a person can act on.
        let windows_own = low.contains("\\windows\\")
            || low.contains("\\system32")
            || low.starts_with("\\systemroot")
            || low.contains("system32\\")
            || low.contains("microsoft")
            || low.starts_with('\\');
        if windows_own {
            continue;
        }
        let program = program_of(&command);
        let boots = match start {
            Some(0) => "boot",
            Some(1) => "system",
            _ => "auto",
        };
        out.push(format!(
            "- {name} [{boots}] {} — {}",
            file_state(program.as_deref()),
            command.trim()
        ));
    }
    out
}

/// The whole picture as the text the model reads, and as the answer to
/// `oczi.exe --startup`.
pub fn text() -> String {
    let mut out = String::new();
    out.push_str("What starts with this PC (read from Windows just now).\n\n");

    out.push_str(
        "Registry Run entries. \"on\"/\"off\" is the Task Manager switch; \"file MISSING\" \
         means the program is not on the disk any more, so the entry starts nothing:\n",
    );
    let runs = run_entries();
    if runs.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for line in runs {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\nStartup folders:\n");
    let folders = folder_entries();
    if folders.is_empty() {
        out.push_str("- (nothing)\n");
    } else {
        for line in folders {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\nServices that start on their own (Microsoft's own left out):\n");
    let services = service_entries();
    if services.is_empty() {
        out.push_str("- (none)\n");
    } else {
        for line in services {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str(
        "\nOnly an entry that is on and whose file is present actually starts. An entry \
         switched off in Task Manager does nothing, and one whose file is gone is a \
         leftover of an uninstalled program.",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_program_out_of_a_command_line() {
        assert_eq!(
            program_of("\"C:\\Program Files\\Riot Vanguard\\vgtray.exe\"").as_deref(),
            Some("C:\\Program Files\\Riot Vanguard\\vgtray.exe")
        );
        assert_eq!(
            program_of("D:\\Gry\\Steam\\steam.exe -silent").as_deref(),
            Some("D:\\Gry\\Steam\\steam.exe")
        );
        // A bare name is not a path: nothing can be said about the file.
        assert_eq!(program_of("steam.exe -silent"), None);
    }

    #[test]
    fn reads_the_switch_out_of_its_binary_value() {
        for (byte, expected) in [
            (2u8, Some(true)),
            (6, Some(true)),
            (3, Some(false)),
            (7, Some(false)),
            (0, None),
            (9, None),
        ] {
            assert_eq!(switch_state(byte), expected);
        }
    }

    #[test]
    fn skips_expandable_and_plain_strings_the_same_way() {
        let bytes: Vec<u8> = "C:\\x.exe\0"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        assert_eq!(text_of(REG_SZ, &bytes).as_deref(), Some("C:\\x.exe"));
        assert_eq!(text_of(REG_DWORD, &bytes), None);
    }
}
