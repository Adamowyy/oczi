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

/// The weekday names Windows' `wDayOfWeek` indexes into.
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Today's local date as `YYYY-MM-DD`, plus the weekday, read from Windows so
/// the user's time zone and DST are already applied.
pub fn today() -> String {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} ({})",
        now.wYear,
        now.wMonth,
        now.wDay,
        weekday(now.wDayOfWeek)
    )
}

/// The same clock with the time of day: `Thursday 2026-10-08 14:03`. The model
/// needs the time, not just the date, before it can turn "in twenty minutes" or
/// "tomorrow at eight" into a reminder of its own.
pub fn now_line() -> String {
    let now = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}",
        weekday(now.wDayOfWeek),
        now.wYear,
        now.wMonth,
        now.wDay,
        now.wHour,
        now.wMinute
    )
}

fn weekday(day_of_week: u16) -> &'static str {
    WEEKDAYS.get(day_of_week as usize % 7).copied().unwrap_or("")
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
