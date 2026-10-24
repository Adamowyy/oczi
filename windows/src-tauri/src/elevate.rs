// One way to run something as an administrator.
//
// Two things in this app need the machine hive: the Game DVR policy and a couple of Windows
// settings that only live under `HKLM`. Neither is worth its own copy of the quoting, so the
// argument marshalling lives here: PowerShell takes an `-EncodedCommand`, which is the script as
// UTF-16LE base64 and therefore immune to three levels of quoting over registry paths.
//
// A refused UAC prompt is an error, not a silent success — `Start-Process -Verb RunAs` fails and
// nothing runs, which is exactly what the caller has to report.

use crate::debloat::powershell;

/// Base64, for the `-EncodedCommand` above. Half a page of encoding beats a dependency for the one
/// place this app needs it.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |at: usize| *chunk.get(at).unwrap_or(&0) as u32;
        let bits = (byte(0) << 16) | (byte(1) << 8) | byte(2);
        let mut push = |index: u32, wanted: bool| {
            out.push(if wanted {
                ALPHABET[(index & 63) as usize] as char
            } else {
                '='
            });
        };
        push(bits >> 18, true);
        push(bits >> 12, true);
        push(bits >> 6, chunk.len() > 1);
        push(bits, chunk.len() > 2);
    }
    out
}

/// Runs `script` through one UAC prompt. Returns as soon as PowerShell has started, because the
/// prompt itself is the part that can be refused — what the script then does is the caller's to
/// read back from the machine.
pub(crate) fn run_elevated(script: &str) -> Result<(), String> {
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let command = format!(
        "Start-Process -FilePath 'powershell.exe' -Verb RunAs \
         -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{}'",
        base64(&utf16)
    );
    powershell(&command).map(|_| ()).map_err(|e| {
        // i18n-ok: shown as it is; a refused UAC prompt lands here.
        format!("Administrator nie wyraził zgody — nic nie zmieniono ({e}).")
    })
}

/// The `-EncodedCommand` an elevated PowerShell actually receives, for a test that can read it.
#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::{PCWSTR, PWSTR};

    #[test]
    fn base64_matches_the_documented_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // The listener PowerShell decodes is UTF-16LE, so the length is even by construction.
        let utf16: Vec<u8> = "abc"
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        assert_eq!(base64(&utf16), "YQBiAGMA");
    }

    /// The registry paths these scripts carry are long and full of backslashes; a script that
    /// loses one of them writes a value Windows never reads. Round-tripping through the same
    /// encoding PowerShell uses is the only cheap proof that nothing is mangled.
    #[test]
    fn a_script_round_trips_through_the_encoded_command() {
        let script = "New-Item -Path 'Registry::HKEY_LOCAL_MACHINE\\SOFTWARE\\Policies\\X' -Force";
        let encoded = base64(
            &script
                .encode_utf16()
                .flat_map(|unit| unit.to_le_bytes())
                .collect::<Vec<u8>>(),
        );
        // Decode the way PowerShell does: base64 → UTF-16LE → string.
        let bytes = decode(&encoded);
        let text = String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<u16>>(),
        )
        .expect("valid UTF-16");
        assert_eq!(text, script);
    }

    /// A decoder for the test only — the app never decodes, PowerShell does.
    fn decode(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut bits = 0u32;
        let mut count = 0;
        for byte in text.bytes() {
            if byte == b'=' {
                break;
            }
            let Some(index) = ALPHABET.iter().position(|a| *a == byte) else {
                continue;
            };
            bits = (bits << 6) | index as u32;
            count += 6;
            if count >= 8 {
                count -= 8;
                out.push((bits >> count) as u8);
            }
        }
        out
    }

    // The imports are here only so the test module compiles the same crate paths the app uses.
    #[allow(dead_code)]
    fn _compile_check() {
        let _ = PCWSTR::null();
        let _ = PWSTR::null();
    }
}
