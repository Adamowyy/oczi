// Small helpers shared across the Rust modules. They live here rather than
// inside the chat client so that unrelated callers (Stripe's basic auth, for
// one) don't have to reach into it for a base64 encoder.

/// Standard base64 (RFC 4648) with padding. Small enough not to be worth a
/// dependency.
pub fn base64_for(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

// Today's local date as `YYYY-MM-DD`, plus the weekday.
pub fn today() -> String {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let weekday = WEEKDAYS
        .get(now.wDayOfWeek as usize % 7)
        .copied()
        .unwrap_or("");
    format!("{:04}-{:02}-{:02} ({weekday})", now.wYear, now.wMonth, now.wDay)
}

#[cfg(test)]
mod tests {
    use super::base64_for;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64_for(b""), "");
        assert_eq!(base64_for(b"f"), "Zg==");
        assert_eq!(base64_for(b"fo"), "Zm8=");
        assert_eq!(base64_for(b"foo"), "Zm9v");
        assert_eq!(base64_for(b"foob"), "Zm9vYg==");
        assert_eq!(base64_for(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_for(b"foobar"), "Zm9vYmFy");
    }
}
