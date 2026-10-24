// The registry, in the four shapes this app needs.
//
// Two modules read and write Windows settings — the Game Bar switches and the tweaks table — and
// both need the same handful of calls: open a key, read a DWORD, write one, delete a value. The
// wrappers live here so a path is opened in one place with one set of rights, and a 32/64-bit view
// mistake can only be made (or fixed) once.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_CREATE_KEY_DISPOSITION, REG_DWORD,
    REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ,
};

/// A key that closes itself, so an early return anywhere cannot leak it.
pub(crate) struct Key(pub(crate) HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The view a value lives in: the machine hive is read and written 64-bit, which is where the
/// policies Windows honours actually are.
fn view(machine: bool) -> u32 {
    if machine {
        KEY_WOW64_64KEY.0
    } else {
        0
    }
}

pub(crate) fn open(root: HKEY, path: &str, machine: bool, rights: REG_SAM_FLAGS) -> Option<Key> {
    let wide_path = wide(path);
    let mut out = HKEY::default();
    let flags = REG_SAM_FLAGS(rights.0 | view(machine));
    if unsafe { RegOpenKeyExW(root, PCWSTR(wide_path.as_ptr()), None, flags, &mut out) }
        != ERROR_SUCCESS
    {
        return None;
    }
    Some(Key(out))
}

/// Reads a DWORD, or `None` when the key or the value is not there. A missing value is not an
/// error: Windows answers such a question from its own defaults, and so does every caller here.
pub(crate) fn read_dword(root: HKEY, path: &str, name: &str, machine: bool) -> Option<u32> {
    let key = open(root, path, machine, KEY_READ)?;
    let wide_name = wide(name);
    let mut data = [0u8; 4];
    let mut size = data.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(wide_name.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS && size >= 4).then(|| u32::from_le_bytes(data))
}

/// Writes a DWORD, creating the key when Windows has never had that value. `false` means the write
/// did not land — which is the whole answer for a machine-level value under an ordinary account.
pub(crate) fn write_dword(root: HKEY, path: &str, name: &str, machine: bool, value: u32) -> bool {
    let Some(key) = create(root, path, machine) else {
        return false;
    };
    let wide_name = wide(name);
    let data = value.to_le_bytes();
    let written = unsafe {
        RegSetValueExW(
            key.0,
            PCWSTR(wide_name.as_ptr()),
            None,
            REG_DWORD,
            Some(&data),
        )
    };
    written == ERROR_SUCCESS
}

/// Writes an empty string value — a marker Windows reads as "this is set", used by the classic
/// context menu, whose whole setting is a CLSID with nothing in it.
pub(crate) fn write_empty_string(root: HKEY, path: &str, name: &str, machine: bool) -> bool {
    let Some(key) = create(root, path, machine) else {
        return false;
    };
    let wide_name = wide(name);
    // `Some(&[])` is an empty REG_SZ: one terminating NUL, which is what Windows writes itself.
    let data = [0u8; 1];
    let written =
        unsafe { RegSetValueExW(key.0, PCWSTR(wide_name.as_ptr()), None, REG_SZ, Some(&data)) };
    written == ERROR_SUCCESS
}

fn create(root: HKEY, path: &str, machine: bool) -> Option<Key> {
    let wide_path = wide(path);
    let mut out = HKEY::default();
    let mut created = REG_CREATE_KEY_DISPOSITION::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            PCWSTR(wide_path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            REG_SAM_FLAGS(KEY_SET_VALUE.0 | KEY_READ.0 | view(machine)),
            None,
            &mut out,
            Some(&mut created),
        )
    };
    (status == ERROR_SUCCESS).then_some(Key(out))
}

/// Whether a value is there at all — the question a marker is.
pub(crate) fn value_exists(root: HKEY, path: &str, name: &str, machine: bool) -> bool {
    let Some(key) = open(root, path, machine, KEY_READ) else {
        return false;
    };
    let wide_name = wide(name);
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(wide_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS
}

/// Removes a value, which is how Windows spells "back to the default". A key that is already clean
/// is not a failure.
pub(crate) fn delete_value(root: HKEY, path: &str, name: &str, machine: bool) -> bool {
    let Some(key) = open(root, path, machine, KEY_SET_VALUE) else {
        return false;
    };
    let wide_name = wide(name);
    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(wide_name.as_ptr())) };
    // ERROR_FILE_NOT_FOUND (2) means there was nothing to delete, which is the wished-for state.
    status == ERROR_SUCCESS || status.0 == 2
}

// The four calls above are wrapped here on purpose: a `REG_SZ` that is one NUL is "set" to Windows,
// and that is the whole of the classic-context-menu setting.

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    /// A round trip against a throwaway key: the same four calls the tweaks table makes, on a path
    /// nobody else reads. It also proves the delete-then-absent reading, which is what "default"
    /// means everywhere above.
    #[test]
    fn a_value_can_be_written_read_and_deleted() {
        let path = "Software\\Oczi\\RegistryProbe";
        let name = "Probe";
        assert!(
            write_dword(HKEY_CURRENT_USER, path, name, false, 0),
            "the first write must land"
        );
        assert_eq!(read_dword(HKEY_CURRENT_USER, path, name, false), Some(0));
        assert!(write_dword(HKEY_CURRENT_USER, path, name, false, 1));
        assert_eq!(read_dword(HKEY_CURRENT_USER, path, name, false), Some(1));
        assert!(value_exists(HKEY_CURRENT_USER, path, name, false));
        assert!(delete_value(HKEY_CURRENT_USER, path, name, false));
        assert_eq!(read_dword(HKEY_CURRENT_USER, path, name, false), None);
        assert!(!value_exists(HKEY_CURRENT_USER, path, name, false));
        // Deleting what is already gone is the state the caller wanted, not an error.
        assert!(delete_value(HKEY_CURRENT_USER, path, name, false));
    }

    #[test]
    fn a_marker_is_presence() {
        let path = "Software\\Oczi\\RegistryProbeMarker";
        assert!(write_empty_string(HKEY_CURRENT_USER, path, "", false));
        assert!(value_exists(HKEY_CURRENT_USER, path, "", false));
        assert!(delete_value(HKEY_CURRENT_USER, path, "", false));
        assert!(!value_exists(HKEY_CURRENT_USER, path, "", false));
    }

    #[test]
    fn a_key_nobody_ever_wrote_reads_as_none() {
        assert_eq!(
            read_dword(
                HKEY_CURRENT_USER,
                "Software\\Oczi\\NoSuchKey",
                "NoSuchValue",
                false
            ),
            None
        );
    }
}
