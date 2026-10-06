// This PC, the machine Oczi runs on, read straight from Windows. No key, no
// request, nothing leaves the computer: the one integration that works out of
// the box. Numbers only, so the card can show them without anything to load.

use std::sync::Mutex;

use serde_json::{json, Value};
use windows::core::PCWSTR;
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows::Win32::System::Threading::GetSystemTimes;

/// `GetDriveTypeW` answers this for a fixed (internal) disk. Win32 only names it
/// in the C headers, so the number is spelled out here.
const DRIVE_FIXED: u32 = 3;

/// The two totals the previous CPU sample ended on. Windows only ever hands out
/// running totals, so the load is the difference between two calls, the first
/// one has nothing to subtract from and reports nothing.
static LAST_CPU: Mutex<Option<(u64, u64)>> = Mutex::new(None);

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// 0–100 over the span since the last sample, or `None` on the very first one.
fn cpu_percent() -> Option<f64> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;

    // Kernel time already contains the idle time, so the total is kernel + user.
    let total = ticks(kernel) + ticks(user);
    let idle = ticks(idle);

    let mut last = LAST_CPU.lock().unwrap();
    let percent = match *last {
        Some((prev_idle, prev_total)) if total > prev_total => {
            let busy = (total - prev_total).saturating_sub(idle.saturating_sub(prev_idle));
            100.0 * busy as f64 / (total - prev_total) as f64
        }
        _ => f64::NAN,
    };
    *last = Some((idle, total));
    percent.is_finite().then(|| percent.clamp(0.0, 100.0))
}

/// (total, used) physical memory in bytes.
fn memory() -> Option<(u64, u64)> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some((status.ullTotalPhys, status.ullTotalPhys.saturating_sub(status.ullAvailPhys)))
}

/// Every fixed disk, with its free room. Removable drives, network shares and a
/// card reader would be noise on a card three rows tall, so they are skipped.
fn disks() -> Vec<Value> {
    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let wide: Vec<u16> = format!("{letter}:\\")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let path = PCWSTR(wide.as_ptr());
        if unsafe { GetDriveTypeW(path) } != DRIVE_FIXED {
            continue;
        }
        let (mut free, mut total) = (0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(path, None, Some(&mut total), Some(&mut free)) }.is_err() {
            continue;
        }
        if total == 0 {
            continue;
        }
        out.push(json!({
            "mount": format!("{letter}:"),
            "freeBytes": free,
            "totalBytes": total,
            "usedPercent": 100.0 * (total - free) as f64 / total as f64,
        }));
    }
    out
}

/// (percent, on mains), both `None` on a machine without a battery, which
/// Windows reports as 255 rather than by failing the call.
fn battery() -> (Option<u8>, Option<bool>) {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return (None, None);
    }
    let percent = (status.BatteryLifePercent != 255).then_some(status.BatteryLifePercent);
    let plugged = match status.ACLineStatus {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    };
    (percent, plugged)
}

/// One sample of the machine, as the card's data blob.
pub fn sample() -> Value {
    let mut info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut info) };

    let (total_mem, used_mem) = memory().unwrap_or((0, 0));
    let (battery_percent, plugged) = battery();

    json!({
        "cpu": cpu_percent(),
        "cores": info.dwNumberOfProcessors,
        "memTotalBytes": total_mem,
        "memUsedBytes": used_mem,
        "memPercent": if total_mem > 0 {
            100.0 * used_mem as f64 / total_mem as f64
        } else {
            0.0
        },
        "disks": disks(),
        "battery": battery_percent,
        "plugged": plugged,
        "uptimeSecs": unsafe { GetTickCount64() } / 1000,
    })
}
