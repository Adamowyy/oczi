// Health: what Windows already knows about itself, turned into one verdict.
//
// Nothing here runs in the background. A scan is one pass on demand — a few seconds of load
// measurement, then what Windows has already written down — and then it is over. The app never
// samples the machine while the user is not asking, and a scan costs nothing once it has returned.
//
// Three sources, all read-only, nothing leaves the machine:
//
// * A short burst of load samples (CPU, memory, the process eating the most CPU) taken *inside*
//   the scan. Long enough to see a spike, short enough that pressing the button feels like a
//   button; a deeper pass runs for a minute when the user asks for one.
// * The event log (System + Application, 72 h, level ≤ 3): crashes, hangs, bugchecks, unexpected
//   restarts, WHEA hardware errors, display-driver resets, services that failed or fell over,
//   firmware throttling, disk I/O errors. This is what a freeze that happened at noon leaves
//   behind, and it is why a one-off scan can still say something about the whole day.
// * The state of the machine right now: disks, autostart leftovers, a pending reboot, uptime.
//
// The verdict is a score and a list of findings. Findings carry a *token* and its numbers, never a
// sentence: the wording lives in the island's language tables, so the same scan reads in Polish and
// in English. Nothing here changes the machine — fixes are separate, named actions the user has to
// confirm after reading the finding (`fix`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::EventLog::{
    EvtClose, EvtNext, EvtQuery, EvtQueryChannelPath, EvtQueryForwardDirection, EvtRender,
    EvtRenderEventXml, EVT_HANDLE,
};
use windows::Win32::System::SystemInformation::{
    GetSystemInfo, GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, GetSystemTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use windows::Wdk::System::SystemInformation::{
    NtQuerySystemInformation, SystemProcessorPerformanceInformation,
};

use crate::log;

/// The shape of a normal scan: ten samples a quarter of a second apart. Three seconds is long
/// enough to catch a spike that lasts, and short enough that the answer is back before the card
/// has finished animating open.
const BURST_STEP_MS: u64 = 250;
const BURST_STEPS: u32 = 10;
/// The deep pass, when the user asks for one: a full minute of samples. Still one pass — it ends
/// by itself and leaves nothing running.
const DEEP_STEPS: u32 = 240;
/// How far back the event query looks, in milliseconds — the form the XPath filter wants. A normal
/// scan covers three days; the deep one covers a week, which is what catches a fault that came,
/// went, and was replaced by a quiet day.
const EVENT_WINDOW_MS: i64 = 72 * 3_600 * 1_000;
const EVENT_WINDOW_DEEP_MS: i64 = 7 * 24 * 3_600 * 1_000;

/// Processes whose CPU use says nothing about the machine's health: Windows' own plumbing, the
/// renderer every WebView2 app shares (Oczi's island is one of them), the audio engine, Defender
/// and the update workers. Reporting these was how a scan produced "msedgewebview2 used 100 % of
/// a core" about the window that was drawing the report — a finding nobody can act on is noise.
const NOISE: &[&str] = &[
    "msedgewebview2",
    "oczi",
    "hermes",
    "audiodg",
    "dwm",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "smss",
    "svchost",
    "system",
    "registry",
    "memory compression",
    "secure system",
    "fontdrvhost",
    "runtimebroker",
    "shellexperiencehost",
    "startmenuexperiencehost",
    "searchhost",
    "searchindexer",
    "explorer",
    "conhost",
    "powershell",
    "msmpeng",
    "nissrv",
    "tiworker",
    "trustedinstaller",
    "usoclient",
    "wuauclt",
    "dllhost",
    "taskhostw",
    "backgroundtaskhost",
    "wmiprvse",
];

/// …and the places a program has no business starting from: anything running out of a temp folder
/// or a downloads folder is worth naming, whatever its name is.
const ODD_PATHS: &[&str] = &["\\appdata\\local\\temp\\", "\\downloads\\", "\\temp\\"];
/// A machine with more errors than this in three days has a bigger story than a list can tell.
const EVENT_CAP: usize = 600;

// ── What a scan produces ──────────────────────────────────────────────────────

/// One thing worth telling the user about. `kind` is a token the island translates, `subject` is
/// whatever it happened to (a program, a service, a drive) and `value` is the number that goes
/// with it — neither is a sentence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub kind: String,
    /// `crit`, `warn` or `info` — how loud the card should be about it.
    pub severity: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub value: String,
    /// What the finding most likely means, as a token the island translates — `bugcheck.0x9f`,
    /// `module.<name>`, `service`, … Empty when there is nothing honest to add.
    #[serde(default)]
    pub cause: String,
    /// `true` when the app can do something about it, so the window offers a button. The fix
    /// itself is still the user's decision, taken after reading this.
    #[serde(default)]
    pub fixable: bool,
    /// For `fixable` ones: the action `fix()` understands, and what it acts on.
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub arg: String,
}

/// One thing the scan looked at, so the window can show a checklist rather than only problems.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    /// `ok`, `warn`, `bad` or `skip` — `skip` means this part could not be read at all.
    pub state: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// 0–100. 100 is a machine with nothing to report.
    pub score: u32,
    /// Local wall-clock seconds, plus the same moment as text — the island cannot turn a local
    /// stamp into a date itself without shifting it by the time zone.
    pub at: i64,
    pub at_text: String,
    /// How long the scan itself took, and how many load samples it took.
    pub took_ms: u32,
    pub samples: u32,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub mem_avg: f64,
    pub mem_max: f64,
    /// Logical processors, so a share can be read against the machine it belongs to.
    pub cores: u32,
    /// The process that used the most CPU during the burst.
    #[serde(default)]
    pub hog: String,
    pub hog_pct: f64,
    pub findings: Vec<Finding>,
    pub checks: Vec<Check>,
    /// A deep scan's reading, written by the model when a key is connected. Empty otherwise, and
    /// empty when the call failed — the findings are the report either way.
    #[serde(default)]
    pub advice: String,
}

impl Report {
    pub fn crit(&self) -> usize {
        self.findings.iter().filter(|f| f.severity == "crit").count()
    }

    pub fn warn(&self) -> usize {
        self.findings.iter().filter(|f| f.severity == "warn").count()
    }
}

// ── The load burst ────────────────────────────────────────────────────────────

/// What the samples inside one scan add up to.
#[derive(Debug, Clone, Default)]
struct Burst {
    /// Logical processors: a process pinning one core on a sixteen-core machine is not a machine
    /// that is struggling, and saying "100 % CPU" about it would be a false alarm.
    cores: u32,
    samples: u32,
    cpu_sum: f64,
    cpu_max: f64,
    /// Every CPU reading, so the peak can ignore the single highest one — the sample a scan caught
    /// while it was enumerating processes is the one most likely to be its own footprint.
    cpus: Vec<f64>,
    /// CPU this scan spent on itself, as a share of one core, summed over the burst. It is read
    /// from our own process and subtracted, so the numbers describe the machine and not the tool.
    own_cpu: f64,
    mem_sum: f64,
    mem_max: f64,
    /// Per process: the CPU it used, summed over the samples. A process that spiked once into a
    /// single 250 ms sample is not what "using the CPU" means — the average over the whole scan is,
    /// and that average is what the numbers on the card are.
    totals: HashMap<String, f64>,
    hog: Option<(String, f64)>,
    /// Whoever was on the processor in the sample that reached the highest load, as a share of the
    /// machine. A spike that no average can explain still has a name, and without it "CPU hit 99 %"
    /// is a number that tells nobody anything.
    peak_top: Option<(String, f64)>,
}

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// Kernel+user ticks this process has used so far. Both `burst` and the report use the difference
/// between two readings: the scan opening three hundred process handles is real CPU work, and it
/// must not be counted as the machine being busy.
fn own_cpu_ticks() -> u64 {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    let process = unsafe { GetCurrentProcess() };
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }.is_err() {
        return 0;
    }
    ticks(kernel) + ticks(user)
}

/// 0–100 CPU load of the whole machine since the previous call; `None` on the very first one,
/// which has nothing to subtract from.
fn system_cpu(previous: &mut Option<(u64, u64)>) -> Option<f64> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    // Kernel time already contains the idle time, so the total is kernel + user.
    let total = ticks(kernel) + ticks(user);
    let idle = ticks(idle);
    let percent = match *previous {
        Some((prev_idle, prev_total)) if total > prev_total => {
            let busy = (total - prev_total).saturating_sub(idle.saturating_sub(prev_idle));
            100.0 * busy as f64 / (total - prev_total) as f64
        }
        _ => f64::NAN,
    };
    *previous = Some((idle, total));
    percent.is_finite().then(|| percent.clamp(0.0, 100.0))
}

fn memory_percent() -> Option<f64> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    (status.ullTotalPhys > 0).then(|| {
        100.0 * status.ullTotalPhys.saturating_sub(status.ullAvailPhys) as f64
            / status.ullTotalPhys as f64
    })
}

/// Every running process with its name, so the per-process delta can be attributed.
fn process_list() -> Vec<(u32, String)> {
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut out = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            let bare = name.trim_end_matches(".exe");
            if !bare.is_empty() {
                out.push((entry.th32ProcessID, bare.to_string()));
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snapshot) };
    out
}

/// The process that used the most CPU since the previous sample, as a share of one core.
fn process_cpu(previous: &mut HashMap<u32, u64>, elapsed_secs: f64) -> Vec<(String, f64)> {
    if elapsed_secs <= 0.0 {
        return Vec::new();
    }
    let mut next: HashMap<u32, u64> = HashMap::with_capacity(previous.len());
    let mut per_name: HashMap<String, f64> = HashMap::new();
    for (pid, name) in process_list() {
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            continue;
        };
        let mut created = FILETIME::default();
        let mut exited = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok =
            unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) }
                .is_ok();
        let _ = unsafe { CloseHandle(process) };
        if !ok {
            continue;
        }
        let total = ticks(kernel) + ticks(user);
        next.insert(pid, total);
        let Some(prev) = previous.get(&pid) else {
            // A process that appeared since the last sample has no baseline; its first reading
            // would count everything it used since launch as if it happened in this instant.
            continue;
        };
        if total <= *prev {
            continue;
        }
        // 10 000 000 ticks is one second of one core.
        let pct = (total - prev) as f64 / (elapsed_secs * 10_000_000.0) * 100.0;
        *per_name.entry(name).or_insert(0.0) += pct;
    }
    *previous = next;
    let mut ranked: Vec<(String, f64)> = per_name
        .into_iter()
        // Windows' own plumbing never wins the title: naming audiodg as "the process using the
        // most CPU" is a true line that says nothing, and it was the first false trail this scan
        // produced.
        .filter(|(name, _)| !NOISE.contains(&name.to_lowercase().as_str()))
        // The share of a single core is capped, so a dozen threads cannot report 900 %.
        .map(|(name, pct)| (name, pct.min(100.0)))
        .filter(|(_, pct)| *pct >= 1.0)
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
}

/// Programs running out of a temp or downloads folder — the one thing in a process list that is
/// worth naming without a signature check. Deep scans only: on a normal scan the answer is a
/// second of extra work nobody asked for.
fn odd_processes() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (pid, name) in process_list() {
        if NOISE.contains(&name.to_lowercase().as_str()) {
            continue;
        }
        let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            continue;
        };
        let mut path = vec![0u16; 1024];
        let mut size = path.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(path.as_mut_ptr()),
                &mut size,
            )
        }
        .is_ok();
        let _ = unsafe { CloseHandle(process) };
        if !ok {
            continue;
        }
        let path = String::from_utf16_lossy(&path[..size as usize]).to_lowercase();
        if ODD_PATHS.iter().any(|odd| path.contains(odd)) && !out.iter().any(|n| n == &name) {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Folds one sample in. Pure, so the arithmetic behind "CPU was pegged at 98 %" is testable
/// without pegging anything.
fn fold(b: &mut Burst, cpu: f64, mem: f64, top: Option<(String, f64)>) {
    if cpu > b.cpu_max {
        if let Some((name, pct)) = &top {
            if !NOISE.contains(&name.to_lowercase().as_str()) {
                b.peak_top = Some((name.clone(), pct / b.cores.max(1) as f64));
            }
        }
    }
    b.samples += 1;
    b.cpu_sum += cpu;
    b.cpu_max = b.cpu_max.max(cpu);
    b.cpus.push(cpu);
    b.mem_sum += mem;
    b.mem_max = b.mem_max.max(mem);
    if let Some((name, pct)) = top {
        *b.totals.entry(name).or_insert(0.0) += pct;
    }
}

/// Picks the process that used the CPU for the whole window, not the one that touched it once.
/// `hog` carries the average share of one core; anything under a fifth of a core is not worth
/// naming at all.
fn settle_hog(b: &mut Burst) {
    let samples = b.samples.max(1) as f64;
    b.hog = b
        .totals
        .iter()
        .map(|(name, total)| (name.clone(), total / samples))
        .filter(|(name, pct)| !NOISE.contains(&name.to_lowercase().as_str()) && *pct >= 20.0)
        .max_by(|a, b| a.1.total_cmp(&b.1));
}

/// The measurement half of a scan: `steps` samples, a quarter of a second apart.
fn burst(steps: u32) -> Burst {
    let mut system = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system) };
    let mut out = Burst {
        cores: system.dwNumberOfProcessors.max(1),
        ..Default::default()
    };
    let mut previous_system: Option<(u64, u64)> = None;
    let mut previous_proc: HashMap<u32, u64> = HashMap::new();
    let mut last_tick = unsafe { GetTickCount64() };
    let mut own_previous = own_cpu_ticks();
    for step in 0..steps {
        std::thread::sleep(std::time::Duration::from_millis(BURST_STEP_MS));
        let now = unsafe { GetTickCount64() };
        let elapsed = now.saturating_sub(last_tick) as f64 / 1_000.0;
        last_tick = now;
        // The first sample only primes the deltas; reporting it would mean inventing a baseline.
        let raw = system_cpu(&mut previous_system);
        let top = process_cpu(&mut previous_proc, elapsed).into_iter().next();
        let own_now = own_cpu_ticks();
        let own_pct = if elapsed > 0.0 {
            (own_now.saturating_sub(own_previous)) as f64 / (elapsed * 10_000_000.0) * 100.0
        } else {
            0.0
        };
        own_previous = own_now;
        out.own_cpu += own_pct;
        if step == 0 {
            continue;
        }
        // Our own work is measured, not estimated: enumerating every process ten times is a few
        // percent of one core, and on an idle machine that is the difference between 2 % and 9 %.
        let cpu = (raw.unwrap_or(0.0) - own_pct / out.cores.max(1) as f64).max(0.0);
        fold(&mut out, cpu, memory_percent().unwrap_or(0.0), top);
    }
    settle_hog(&mut out);
    out
}

impl Burst {
    /// The peak with its single highest sample dropped: one reading taken while the scan was
    /// enumerating is the one most likely to be the scan itself, and a peak built on it is a number
    /// that scares people about nothing.
    fn cpu_peak(&self) -> f64 {
        if self.cpus.len() < 3 {
            return self.cpus.iter().copied().fold(0.0, f64::max);
        }
        let mut sorted = self.cpus.clone();
        sorted.sort_by(f64::total_cmp);
        sorted[sorted.len() - 2]
    }

    fn cpu_avg(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.cpu_sum / self.samples as f64
        }
    }

    fn mem_avg(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.mem_sum / self.samples as f64
        }
    }
}

// ── Watching for a freeze ─────────────────────────────────────────────────────

/// One entry of `SystemProcessorPerformanceInformation`, in the order Windows writes it. Only the
/// timing counters are read, and only as differences between two readings.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct ProcPerf {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
}

/// Eight bytes at `at`, as a native-order integer.
fn buf_at(buffer: &[u8], at: usize) -> Option<[u8; 8]> {
    buffer.get(at..at + 8)?.try_into().ok()
}

/// (dpc, interrupt) ticks summed over every processor, or `None` when the query is unavailable.
///
/// This is the measurement that names a *driver* rather than a program: a robotic-voice, everything
/// freezes stutter is usually a driver spending seconds in deferred procedure calls, and no process
/// list shows it. The counters are cumulative, so the caller takes the difference.
fn dpc_ticks() -> Option<(i64, i64)> {
    let mut system = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system) };
    let cpus = system.dwNumberOfProcessors.max(1) as usize;
    let stride = std::mem::size_of::<ProcPerf>().max(56);
    let mut buffer = vec![0u8; cpus * stride + 64];
    let mut returned = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(
            SystemProcessorPerformanceInformation,
            buffer.as_mut_ptr() as *mut std::ffi::c_void,
            buffer.len() as u32,
            &mut returned,
        )
    };
    if status.is_err() {
        return None;
    }
    // Windows writes one entry per processor and tells us how much it wrote: that division is the
    // only trustworthy stride. Reading with my own guess gave 49 % DPC on an idle machine — the
    // numbers were the next entry's, read at the wrong offset.
    let returned = returned as usize;
    let stride = if returned >= cpus * 48 {
        returned / cpus
    } else {
        stride
    };
    let mut dpc = 0i64;
    let mut interrupt = 0i64;
    for index in 0..cpus {
        let at = index * stride;
        if at + 40 > buffer.len() {
            break;
        }
        // dpc at byte 24, interrupt at 32: idle, kernel, user, dpc, interrupt, count.
        let dpc_at = i64::from_ne_bytes(buf_at(&buffer, at + 24)?);
        let interrupt_at = i64::from_ne_bytes(buf_at(&buffer, at + 32)?);
        dpc += dpc_at;
        interrupt += interrupt_at;
    }
    // A counter that went backwards is not a machine that unsent time; refuse the reading.
    (dpc >= 0 && interrupt >= 0).then_some((dpc, interrupt))
}

/// One moment the machine stopped. The whole point of a watch: a freeze of three seconds leaves
/// nothing in any log, so the only witness is a sampler that was running when it happened — and
/// what it can testify to is the gap, and who was on the processor when the gap ended.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stall {
    /// Local wall-clock seconds when the machine came back.
    pub at: i64,
    pub at_text: String,
    /// How long the machine was gone, beyond the second the sample was due.
    pub ms: u32,
    /// The machine's CPU around that moment, and the process that took it.
    #[serde(default)]
    pub top: String,
    pub top_pct: f64,
    /// Share of all processor time spent in driver code (DPC) around the stop. Double digits here
    /// is a driver, not a program.
    pub dpc: f64,
    /// Total machine load at that moment, so "13 % of the machine" has something to sit beside.
    #[serde(default)]
    pub cpu: f64,
    /// The three busiest programs then, as shares of the whole machine. A driver's own helper — the
    /// audio suite, the overlay — is almost never the first name, and it was the second name that
    /// would have shown it.
    #[serde(default)]
    pub tops: Vec<NameShare>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NameShare {
    pub name: String,
    pub pct: f64,
}

/// The result of one watch: long enough to catch a stutter, short enough to be a button.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub minutes: u32,
    pub started: i64,
    pub started_text: String,
    pub samples: u32,
    pub cores: u32,
    pub stalls: Vec<Stall>,
    pub cpu_avg: f64,
    pub cpu_peak: f64,
    pub mem_peak: f64,
    /// The process that used the most, over the whole watch.
    #[serde(default)]
    pub busiest: String,
    pub busiest_pct: f64,
    /// Peaks of driver (DPC) and hardware-interrupt time, as a share of all processor time — the
    /// number that catches the stutter no process list shows.
    pub dpc_peak: f64,
    pub interrupt_peak: f64,
    /// The user stopped it before its time; the numbers cover what ran.
    #[serde(default)]
    pub cancelled: bool,
    /// Who was on the processor across all the stops, counted once per stop: the same name under
    /// twelve timestamps is twelve lines of nothing, and the count is the actual finding.
    #[serde(default)]
    pub culprits: Vec<Culprit>,
    /// Stops where driver time, not a program, was the story.
    #[serde(default)]
    pub driver_stalls: u32,
    /// Stops where nothing could be named at all.
    #[serde(default)]
    pub blind_stalls: u32,
    /// Typical spacing between stops, in minutes — a freeze that returns every ten minutes is a
    /// different animal from twelve stops in twelve hours.
    #[serde(default)]
    pub gap_minutes: u32,
    /// Driver suites running on this machine, when the driver time was high. Suspects, named as
    /// suspects: a filter driver can ride on another vendor's code, and guessing would be worse
    /// than saying nothing.
    #[serde(default)]
    pub suspects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Culprit {
    pub name: String,
    pub count: u32,
    /// The highest share of the machine this name held in any one stop.
    pub pct: f64,
}

impl Watch {
    pub fn longest(&self) -> u32 {
        self.stalls.iter().map(|s| s.ms).max().unwrap_or(0)
    }
}

/// Set by `health_watch_cancel`. The watch is one loop on a worker thread, so calling it off is a
/// flag it reads between samples — the button cannot kill a thread, and does not need to.
static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Stops the watch that is running, if one is.
pub fn cancel() {
    CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Samples the machine once a second for `minutes`, and writes down every moment the sampler itself
/// was late — which is what a freeze looks like from the inside. Nothing runs afterwards: the loop
/// ends, the numbers are frozen in the result, and the app keeps no thread.
pub fn watch(minutes: u32) -> Watch {
    let minutes = minutes.clamp(1, 60);
    let started = crate::reminders::now_local();
    let mut out = Watch {
        minutes,
        started,
        started_text: crate::reminders::format_local(started),
        ..Default::default()
    };
    let total = std::time::Duration::from_secs(minutes as u64 * 60);
    CANCEL.store(false, std::sync::atomic::Ordering::Relaxed);
    // Logged at both ends: a watch that starts and has no matching "done" line is a loop that never
    // returned, which is the one failure the card cannot show.
    log::line(format!("health watch start: {minutes} min"));
    let begin = std::time::Instant::now();
    let mut previous_system: Option<(u64, u64)> = None;
    let mut previous_proc: HashMap<u32, u64> = HashMap::new();
    let mut previous_tick = unsafe { GetTickCount64() };
    let mut own_previous = own_cpu_ticks();
    let mut dpc_previous = dpc_ticks();
    let mut system = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system) };
    let cores = system.dwNumberOfProcessors.max(1) as f64;
    out.cores = cores as u32;

    while begin.elapsed() < total {
        std::thread::sleep(std::time::Duration::from_millis(1_000));
        if CANCEL.load(std::sync::atomic::Ordering::Relaxed) {
            out.cancelled = true;
            break;
        }
        let now_tick = unsafe { GetTickCount64() };
        let elapsed = now_tick.saturating_sub(previous_tick) as f64 / 1_000.0;
        previous_tick = now_tick;
        let raw = system_cpu(&mut previous_system);
        // The whole ranking, so a stop can say what came second and third as well: an audio suite
        // or an overlay is the thing behind a stall surprisingly often, and it is rarely first.
        let ranked = process_cpu(&mut previous_proc, elapsed);
        let top = ranked.first().map(|(name, pct)| (name.clone(), *pct));
        let own_now = own_cpu_ticks();
        let own_pct = if elapsed > 0.0 {
            own_now.saturating_sub(own_previous) as f64 / (elapsed * 10_000_000.0) * 100.0
        } else {
            0.0
        };
        own_previous = own_now;
        if raw.is_none() {
            continue;
        }
        let cpu = (raw.unwrap_or(0.0) - own_pct / cores).max(0.0);
        let mem = memory_percent().unwrap_or(0.0);
        // Driver and interrupt time as a share of everything the processors could do this second.
        let mut dpc_share = 0.0;
        if let Some((dpc, interrupt)) = dpc_ticks() {
            if let Some((prev_dpc, prev_interrupt)) = dpc_previous {
                let span = (elapsed * 10_000_000.0 * cores).max(1.0);
                dpc_share = dpc.saturating_sub(prev_dpc) as f64 / span * 100.0;
                let interrupt_share = interrupt.saturating_sub(prev_interrupt) as f64 / span * 100.0;
                out.dpc_peak = out.dpc_peak.max(dpc_share);
                out.interrupt_peak = out.interrupt_peak.max(interrupt_share);
            }
            dpc_previous = Some((dpc, interrupt));
        }
        out.samples += 1;
        out.cpu_avg += cpu;
        out.cpu_peak = out.cpu_peak.max(cpu);
        out.mem_peak = out.mem_peak.max(mem);
        // Every process number here is a share of the whole machine.
        let top = top.map(|(name, pct)| (name, pct / cores));
        if let Some((name, pct)) = &top {
            // Same rule as the scan: Windows' own processes never get to be "the busiest", because
            // naming one is a true sentence that helps nobody.
            if *pct > out.busiest_pct && !NOISE.contains(&name.to_lowercase().as_str()) {
                out.busiest_pct = *pct;
                out.busiest = name.clone();
            }
        }
        // A second of sleep that lasted half as long again is the machine having stopped, not the
        // scheduler being busy: Windows does not delay a thread by seconds because of a queue.
        // Anything past two minutes is the machine sleeping, which is not a freeze.
        if elapsed >= 1.6 && elapsed < 120.0 {
            let at = crate::reminders::now_local();
            let (name, pct) = top.clone().unwrap_or_default();
            out.stalls.push(Stall {
                at,
                at_text: crate::reminders::format_local(at),
                ms: ((elapsed - 1.0) * 1_000.0) as u32,
                top: name,
                top_pct: pct,
                dpc: dpc_share,
                cpu,
                tops: ranked
                    .iter()
                    .take(3)
                    .map(|(n, p)| NameShare {
                        name: n.clone(),
                        pct: p / cores,
                    })
                    .collect(),
            });
        }
    }
    if out.samples > 0 {
        out.cpu_avg /= out.samples as f64;
    }

    settle_stalls(&mut out);

    log::line(format!(
        "health watch done{}: {} min, {} samples, {} stalls (longest {} ms), busiest {} {:.0}%, dpc peak {:.0}%",
        if out.cancelled { " (cancelled)" } else { "" },
        out.minutes,
        out.samples,
        out.stalls.len(),
        out.longest(),
        out.busiest,
        out.busiest_pct,
        out.dpc_peak
    ));
    save_watch(&out);
    out
}

/// Folds the stops into what they add up to: one entry per name, how often they come, and which
/// driver suites are on the machine when the driver time was the story. Pure, so the arithmetic
/// behind "the same thing every ten minutes" is testable without freezing anything.
fn settle_stalls(out: &mut Watch) {
    // One entry per name instead of one line per stop.
    let mut by_name: Vec<Culprit> = Vec::new();
    for stall in &out.stalls {
        if stall.dpc >= 8.0 {
            out.driver_stalls += 1;
            continue;
        }
        if stall.top.is_empty() {
            out.blind_stalls += 1;
            continue;
        }
        match by_name.iter_mut().find(|c| c.name == stall.top) {
            Some(c) => {
                c.count += 1;
                c.pct = c.pct.max(stall.top_pct);
            }
            None => by_name.push(Culprit {
                name: stall.top.clone(),
                count: 1,
                pct: stall.top_pct,
            }),
        }
    }
    by_name.sort_by(|a, b| b.count.cmp(&a.count).then(b.pct.total_cmp(&a.pct)));
    by_name.truncate(3);
    out.culprits = by_name;

    // How often the stops come, because "every ten minutes" and "twice in three hours" send you to
    // completely different places.
    let mut times: Vec<i64> = out.stalls.iter().map(|s| s.at).collect();
    times.sort_unstable();
    if times.len() >= 2 {
        let mut gaps: Vec<i64> = times.windows(2).map(|w| (w[1] - w[0]) / 60).collect();
        gaps.sort_unstable();
        out.gap_minutes = gaps[gaps.len() / 2].max(0) as u32;
    }
    // Driver time in double digits is a driver, and the vendors whose filters are the usual
    // suspects are worth naming — as present on the machine, never as convicted.
    if out.dpc_peak >= 8.0 {
        out.suspects = driver_suspects();
    }
}

/// Driver suites that live in user space on this machine and are the usual companions of a stall
/// with heavy driver time. Matched on process names, which is a list of what is running now — the
/// report says "present", never "guilty".
fn driver_suspects() -> Vec<String> {
    const KNOWN: &[(&str, &str)] = &[
        ("sonar", "SteelSeries Sonar"),
        ("steelseries", "SteelSeries"),
        ("nahimic", "Nahimic"),
        ("rtkauduservice", "Realtek audio"),
        ("razer", "Razer"),
        ("voicemeeter", "Voicemeeter"),
        ("nvcontainer", "NVIDIA"),
        ("amdfendr", "AMD"),
        ("logi", "Logitech"),
    ];
    let running: Vec<String> = process_list()
        .into_iter()
        .map(|(_, name)| name.to_lowercase())
        .collect();
    let mut out: Vec<String> = Vec::new();
    for (needle, label) in KNOWN {
        if running.iter().any(|n| n.contains(needle)) && !out.iter().any(|o| o == label) {
            out.push((*label).to_string());
        }
    }
    out
}

fn watch_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("watch.json")
}

fn save_watch(watch: &Watch) {
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(watch) {
        if let Err(e) = std::fs::write(watch_path(), json) {
            log::line(format!("watch report not saved: {e}"));
        }
    }
}

/// The last watch ever run, so a freeze report survives a restart.
pub fn last_watch() -> Option<Watch> {
    let bytes = std::fs::read(watch_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The watch as text, for `oczi.exe --watch` and for the model.
pub fn describe_watch(watch: &Watch) -> String {
    let mut out = format!(
        "Obserwacja {} min od {}{}: {} próbek, {} zatrzymań, najdłuższe {} ms\n",
        watch.minutes,
        watch.started_text,
        if watch.cancelled { " (przerwana)" } else { "" },
        watch.samples,
        watch.stalls.len(),
        watch.longest()
    );
    out.push_str(&format!(
        "CPU średnio {:.0}%, szczyt {:.0}%; RAM szczyt {:.0}%\n",
        watch.cpu_avg, watch.cpu_peak, watch.mem_peak
    ));
    if watch.dpc_peak > 0.0 || watch.interrupt_peak > 0.0 {
        out.push_str(&format!(
            "Szczyt czasu sterowników (DPC) {:.0}%, przerwań {:.0}%\n",
            watch.dpc_peak, watch.interrupt_peak
        ));
    }
    if !watch.busiest.is_empty() {
        out.push_str(&format!(
            "Najwięcej CPU: {} ({:.0}% mocy maszyny, {} rdzeni)\n",
            watch.busiest, watch.busiest_pct, watch.cores
        ));
    }
    // Who was on the processor at the stops, counted once per name: the model reads this instead
    // of twelve timestamps and can go straight to the name that keeps appearing.
    if !watch.culprits.is_empty() || watch.driver_stalls > 0 || watch.blind_stalls > 0 {
        let mut parts: Vec<String> = watch
            .culprits
            .iter()
            .map(|c| format!("{} ×{} (do {:.0}% maszyny)", c.name, c.count, c.pct))
            .collect();
        if watch.driver_stalls > 0 {
            parts.push(format!("czas sterowników ×{}", watch.driver_stalls));
        }
        if watch.blind_stalls > 0 {
            parts.push(format!("bez nazwy ×{}", watch.blind_stalls));
        }
        out.push_str(&format!("Przy zatrzymaniach: {}\n", parts.join(", ")));
    }
    if watch.gap_minutes > 0 {
        out.push_str(&format!("Zatrzymania mniej więcej co {} min\n", watch.gap_minutes));
    }
    if !watch.suspects.is_empty() {
        out.push_str(&format!(
            "Sterowniki obecne na maszynie (podejrzani, nie winni): {}\n",
            watch.suspects.join(", ")
        ));
    }
    for stall in &watch.stalls {
        // The process named here is the one on the processor when the machine came back — the
        // closest thing to a culprit a sampler can honestly offer.
        out.push_str(&format!(
            "- {} zatrzymanie {} ms (DPC {:.0}%), proces: {}\n",
            stall.at_text,
            stall.ms,
            stall.dpc,
            if stall.top.is_empty() { "nieznany".to_string() } else { format!("{} ({:.0}% mocy maszyny)", stall.top, stall.top_pct) }
        ));
        // Whoever came second and third matters more than it looks: the game is always first.
        if stall.tops.len() > 1 {
            let also: Vec<String> = stall
                .tops
                .iter()
                .skip(1)
                .map(|t| format!("{} {:.0}%", t.name, t.pct))
                .collect();
            out.push_str(&format!("  obok niego: {}\n", also.join(", ")));
        }
    }
    out
}

// ── The event log ─────────────────────────────────────────────────────────────

/// The bits of an event this module reads: what Windows calls it, and the parameters that say
/// which program, service or driver it was about.
#[derive(Debug, Clone, Default)]
struct Event {
    id: u32,
    provider: String,
    /// The event's own parameters, in the order Windows wrote them: the program for an
    /// application error, the service for a service failure, the driver for a display reset.
    data: Vec<String>,
}

/// Text between `<tag …>` and `</tag>`, for the flat fields of the event XML.
fn tag_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let after = xml[start..].find('>')? + start + 1;
    let close = xml[after..].find("</")? + after;
    Some(xml[after..close].to_string())
}

/// Every `<Data>` value in the order Windows wrote them. The schema differs between providers
/// (`<Data Name="param1">` for an application error, a bare `<Data>` elsewhere), so this takes
/// whatever sits between the tags and leaves the meaning to the caller.
fn data_values(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in xml.split("<Data").skip(1) {
        let Some(gt) = chunk.find('>') else {
            continue;
        };
        let rest = &chunk[gt + 1..];
        let Some(end) = rest.find("</Data>") else {
            continue;
        };
        let value = rest[..end].trim();
        if !value.is_empty() && !value.starts_with('<') {
            out.push(value.to_string());
        }
    }
    out
}

fn parse_event(xml: &str) -> Option<Event> {
    let id: u32 = tag_value(xml, "EventID")?.trim().parse().ok()?;
    let provider = xml
        .split("<Provider Name=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_string();
    Some(Event {
        id,
        provider,
        data: data_values(xml),
    })
}

fn render(handle: EVT_HANDLE) -> Option<String> {
    // An event's XML is a couple of kilobytes; anything past this is not ours to print.
    let mut buffer = vec![0u8; 64 * 1024];
    let mut used = 0u32;
    let mut properties = 0u32;
    unsafe {
        EvtRender(
            None,
            handle,
            EvtRenderEventXml.0,
            buffer.len() as u32,
            Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
            &mut used,
            &mut properties,
        )
    }
    .ok()?;
    let units = used as usize / 2;
    let wide = unsafe { std::slice::from_raw_parts(buffer.as_ptr() as *const u16, units) };
    Some(String::from_utf16_lossy(wide))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Errors and warnings from one channel, newest first, capped. The query filters by level and by
/// age inside Windows, so a machine with a busy log is not read event by event.
fn channel_events(channel: &str, cap: usize, window_ms: i64) -> Vec<Event> {
    let query =
        format!("*[System[(Level<=3) and TimeCreated[timediff(@SystemTime) <= {window_ms}]]]");
    let channel_w = wide(channel);
    let query_w = wide(&query);
    let Ok(set) = (unsafe {
        EvtQuery(
            None,
            PCWSTR(channel_w.as_ptr()),
            PCWSTR(query_w.as_ptr()),
            EvtQueryChannelPath.0 | EvtQueryForwardDirection.0,
        )
    }) else {
        log::line(format!("health: cannot read the {channel} log"));
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut batch = vec![0isize; 32];
    while out.len() < cap {
        let mut got = 0u32;
        if unsafe { EvtNext(set, &mut batch, 2_000, 0, &mut got) }.is_err() || got == 0 {
            break;
        }
        for &raw in &batch[..got as usize] {
            let handle = EVT_HANDLE(raw);
            if let Some(xml) = render(handle) {
                if let Some(event) = parse_event(&xml) {
                    out.push(event);
                }
            }
            let _ = unsafe { EvtClose(handle) };
        }
    }
    let _ = unsafe { EvtClose(set) };
    out
}

/// One report Windows left behind in its WER archive: the file that names a crash's faulting module
/// and a bugcheck's code. This is what the event log does not carry — its 1001 says "a bugcheck
/// happened", the report says *which code, which module, which program*.
#[derive(Debug, Clone, Default)]
struct WerReport {
    event: String,
    fields: HashMap<String, String>,
    /// Local wall-clock seconds, from the report file itself.
    at: i64,
}

/// `Sig[3].Name=Fault Module Name` / `Sig[3].Value=nvwgf2umx.dll` — the archive's own key-value
/// lines, parsed the way Windows writes them: one `key=value` per line, BOM and blanks ignored.
fn parse_wer(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}');
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || value.trim().is_empty() {
            continue;
        }
        out.insert(key.to_string(), value.trim().to_string());
    }
    out
}

/// The value that belongs to a label. The archive numbers its fields (`Sig[3].Name=Fault Module
/// Name`, `Sig[3].Value=nvwgf2umx.dll`), so the label is looked up first and its sibling read by
/// name — matching the label text against a *value* is how this reads `BCCode` as if it were a code.
fn wer_field(fields: &HashMap<String, String>, needle: &str) -> Option<String> {
    let needle = needle.to_lowercase();
    for (key, label) in fields {
        if !key.ends_with(".Name") || !label.to_lowercase().contains(&needle) {
            continue;
        }
        let value_key = key.replace(".Name", ".Value");
        if let Some(value) = fields.get(&value_key) {
            let value = value.trim();
            if !value.is_empty() && value != "0" {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Every report in the WER archive newer than `window`, newest first. Reading the archive is a few
/// hundred small files at most; a machine that crashes often has thousands, so the walk is capped.
fn wer_reports(window_secs: i64, now: i64) -> Vec<WerReport> {
    const CAP: usize = 200;
    let mut out = Vec::new();
    for root in [
        "C:\\ProgramData\\Microsoft\\Windows\\WER\\ReportArchive",
        "C:\\ProgramData\\Microsoft\\Windows\\WER\\ReportQueue",
    ] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten().take(CAP) {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_dir() {
                continue;
            }
            // The file's own timestamp is enough to place the crash in time, and unlike the
            // EventTime field it cannot be a different epoch.
            let at = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if at == 0 || now - at > window_secs {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path().join("Report.wer")) else {
                continue;
            };
            let fields = parse_wer(&text);
            let event = fields.get("EventType").cloned().unwrap_or_default();
            if event.is_empty() {
                continue;
            }
            out.push(WerReport { event, fields, at });
        }
    }
    out.sort_by(|a, b| b.at.cmp(&a.at));
    out
}

/// A bugcheck code as Windows writes it: `159` → `0x9f`, already known codes lower-cased so the
/// language table can carry one row per family.
fn bugcheck_code(raw: &str) -> Option<String> {
    let raw = raw.trim();
    // WER writes the code in decimal (159 is 0x9F); the event log writes it decimal too, and a
    // value that already carries `0x` is hex. Reading "159" as hex produced 0x159 — a code that
    // exists in no table and would have been reported as the wrong bugcheck.
    let value = match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => raw.parse::<u32>().ok()?,
    };
    (value != 0).then(|| format!("0x{value:x}"))
}

/// What a bugcheck code means to a person, as a token. Only the codes a home machine actually
/// produces are here; anything else falls back to the code itself, which is still a real answer —
/// it is the string you type into a search box.
fn bugcheck_cause(code: &str) -> String {
    let known = matches!(
        code,
        "0x9f" | "0x124" | "0x101" | "0x3b" | "0x50" | "0xd1" | "0x1a" | "0xef" | "0x7e" | "0xc2" | "0x139"
    );
    if known {
        format!("bugcheck.{code}")
    } else {
        "bugcheck.unknown".to_string()
    }
}

/// What an event means for the machine's health, or `None` for the thousands of entries that say
/// nothing about it. This list is the verdict's whole vocabulary, so a noisy provider — DCOM's
/// 10016, say — needs no filter of its own: it simply is not here.
fn classify(e: &Event) -> Option<(&'static str, &'static str)> {
    let p = e.provider.trim_start_matches("Microsoft-Windows-");
    Some(match (p, e.id) {
        ("Kernel-Power", 41) => ("restart", "crit"),
        ("EventLog", 6008) => ("restart", "crit"),
        ("WER-SystemErrorReporting", 1001) => ("bugcheck", "crit"),
        ("WHEA-Logger", _) => ("hardware", "crit"),
        ("Application Error", 1000) => ("appCrash", "warn"),
        ("Application Hang", 1002) => ("appHang", "warn"),
        ("Display", 4101) => ("driverReset", "warn"),
        ("Service Control Manager", 7000)
        | ("Service Control Manager", 7009)
        | ("Service Control Manager", 7011)
        | ("Service Control Manager", 7031)
        | ("Service Control Manager", 7034) => ("service", "warn"),
        ("Kernel-Processor-Power", 37) => ("throttle", "warn"),
        ("disk", 7) | ("disk", 51) | ("disk", 153) => ("diskError", "warn"),
        ("Ntfs", 55) | ("Ntfs", 130) => ("diskError", "crit"),
        _ => return None,
    })
}

/// A parameter as a card can show it: a full path becomes its last segment, and a long value is
/// cut rather than allowed to push the row out of the window.
fn short_subject(raw: &str) -> String {
    let name = raw
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(raw)
        .trim()
        .trim_end_matches(',')
        .to_string();
    name.chars().take(48).collect()
}

// ── The scan ──────────────────────────────────────────────────────────────────

fn check(id: &str, state: &str) -> Check {
    Check {
        id: id.to_string(),
        state: state.to_string(),
    }
}

/// The sentence that goes under a finding. Every kind has one, because a coloured line with no
/// explanation is the thing a person cannot act on — the token is translated on the island side.
fn cause_of(kind: &str) -> String {
    let known = matches!(
        kind,
        "driverReset"
            | "restart"
            | "hardware"
            | "service"
            | "appCrash"
            | "appHang"
            | "hog"
            | "startupLeftover"
            | "startupMany"
            | "diskFull"
            | "diskError"
            | "oddProcess"
            | "reboot"
            | "throttle"
            | "cpuHigh"
            | "memHigh"
            | "uptime"
            | "module"
    );
    if known {
        format!("{kind}")
    } else {
        String::new()
    }
}

fn finding(kind: &str, severity: &str, subject: &str, value: &str) -> Finding {
    Finding {
        kind: kind.to_string(),
        severity: severity.to_string(),
        subject: subject.to_string(),
        value: value.to_string(),
        cause: cause_of(kind),
        fixable: false,
        action: String::new(),
        arg: String::new(),
    }
}

/// 100 minus what the findings cost, floored at 5: the machine still runs, and a percentage that
/// reads 0 % says "dead" rather than "look at this".
fn score(findings: &[Finding]) -> u32 {
    let mut score = 100i32;
    for f in findings {
        score -= match f.severity.as_str() {
            "crit" => 22,
            "warn" => 8,
            _ => 2,
        };
    }
    score.clamp(5, 100) as u32
}

/// Groups the events by what they were about, so forty crashed launches of one program read as
/// one finding with the number 40 rather than as forty identical lines.
fn event_findings(events: &[Event]) -> Vec<Finding> {
    // (kind, subject) → (count, severity)
    let mut grouped: HashMap<(String, String), (u32, &'static str)> = HashMap::new();
    for e in events {
        let Some((kind, severity)) = classify(e) else {
            continue;
        };
        // The subject is whichever program, service or driver the event was about — Windows puts
        // it in the first parameter. Anything without one is named after its provider, which is
        // the most a card can say about it.
        let subject = e
            .data
            .first()
            .map(|raw| short_subject(raw))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| e.provider.trim_start_matches("Microsoft-Windows-").to_string());
        let slot = grouped
            .entry((kind.to_string(), subject))
            .or_insert((0, severity));
        slot.0 += 1;
        // One critical event makes the whole group critical.
        if severity == "crit" {
            slot.1 = "crit";
        }
    }
    let mut out: Vec<Finding> = grouped
        .into_iter()
        .map(|((kind, subject), (count, severity))| {
            finding(&kind, severity, &subject, &count.to_string())
        })
        .collect();
    out.sort_by(|a, b| {
        let rank = |s: &str| match s {
            "crit" => 0,
            "warn" => 1,
            _ => 2,
        };
        rank(&a.severity)
            .cmp(&rank(&b.severity))
            .then_with(|| b.value.cmp(&a.value))
    });
    out
}

/// The load a burst saw, as findings. Kept apart from `scan` so the thresholds that decide what
/// counts as "heavy" sit in one place, above the event log's story.
fn load_findings(load: &Burst) -> Vec<Finding> {
    let mut out = Vec::new();
    // The trimmed peak, the same number the card prints: a finding that says 99 % beside a footer
    // that says 88 % is the kind of contradiction that makes a tool look broken.
    let peak = load.cpu_peak();
    if peak >= 95.0 {
        let mut f = finding("cpuHigh", "warn", "", &format!("{:.0}", peak));
        // Naming the process is the whole difference between "something happened" and an answer.
        if let Some((name, _)) = &load.peak_top {
            f.cause = format!("peak.{name}");
        }
        out.push(f);
    }
    if load.mem_max >= 92.0 {
        out.push(finding(
            "memHigh",
            "warn",
            "",
            &format!("{:.0}", load.mem_max),
        ));
    }
    out
}

/// Everything the scan reports, in one pass. `deep` takes a minute of samples instead of three
/// seconds — still one pass, still nothing left running afterwards.
pub fn scan(deep: bool) -> Report {
    let started = unsafe { GetTickCount64() };
    let at = crate::reminders::now_local();
    let load = burst(if deep { DEEP_STEPS } else { BURST_STEPS });

    let mut findings: Vec<Finding> = Vec::new();
    let mut checks: Vec<Check> = Vec::new();

    // ── What the machine is doing right now.
    findings.extend(load_findings(&load));
    checks.push(check(
        "load",
        if load.cpu_max >= 90.0 || load.mem_max >= 90.0 {
            "warn"
        } else {
            "ok"
        },
    ));
    // One process holding the machine on its own — measured against the whole machine, because a
    // single core at 100 % on a modern PC is one thread doing its job, and never a process Windows
    // runs itself: "msedgewebview2 used 100 %" was the report's own window.
    if let Some((name, pct)) = &load.hog {
        let share = pct / load.cores as f64;
        let noisy = NOISE.contains(&name.to_lowercase().as_str());
        if !noisy && share >= 25.0 && load.cpu_avg() >= 40.0 {
            let mut f = finding("hog", "warn", name, &format!("{share:.0}"));
            // Closing a program is the only repair here, and even that asks it to close as its own
            // quit would, so nothing is lost that the program would not lose anyway.
            f.fixable = true;
            f.action = "closeProcess".to_string();
            f.arg = name.clone();
            findings.push(f);
        }
    }

    // ── The event log: what already happened, whether or not anyone was watching. A deep scan
    // looks a week back instead of three days, which is the difference between "nothing today" and
    // "it happened on Thursday".
    let window = if deep {
        EVENT_WINDOW_DEEP_MS
    } else {
        EVENT_WINDOW_MS
    };
    let mut events = channel_events("System", EVENT_CAP, window);
    events.extend(channel_events("Application", EVENT_CAP, window));
    let from_log = event_findings(&events);
    checks.push(check(
        "events",
        if from_log.iter().any(|f| f.severity == "crit") {
            "bad"
        } else if from_log.is_empty() {
            "ok"
        } else {
            "warn"
        },
    ));
    findings.extend(from_log);

    // ── The machine's own state: room on the disks, what starts with it, what is waiting.
    let (disks, low_disk) = crate::pc::disk_health();
    for disk in &low_disk {
        findings.push(finding("diskFull", "warn", disk, ""));
    }
    checks.push(check(
        "disks",
        if low_disk.is_empty() { "ok" } else { "warn" },
    ));

    let (enabled, leftovers) = crate::startup::health_facts();
    for name in leftovers.iter().take(4) {
        // A leftover entry is the one thing in this scan that can be tidied from here, and the
        // switch-off is the same write Task Manager makes: the entry stays, it stops starting.
        let mut f = finding("startupLeftover", "info", name, "");
        f.fixable = true;
        f.action = "disableStartup".to_string();
        f.arg = name.clone();
        findings.push(f);
    }
    // How many entries are *on* is a fact about the machine, not a problem with it: it used to sit
    // in this list looking like one, so "4 things wrong" included a number that was not a thing.
    if enabled >= 12 {
        let mut f = finding("startupMany", "info", "", &enabled.to_string());
        // Never counted as a finding: the note exists to put the leftovers in perspective.
        f.kind = "startupVolume".to_string();
        findings.push(f);
    }
    checks.push(check(
        "startup",
        if leftovers.is_empty() { "ok" } else { "warn" },
    ));

    // ── Deep only: what Windows wrote down about its own crashes. The event log says a crash
    // happened; the archive says which code and which module — the difference between "it crashed"
    // and "nvwgf2umx.dll crashed it, three times, since Tuesday".
    if deep {
        let reports = wer_reports(window, at);
        let mut seen: HashMap<(String, String), u32> = HashMap::new();
        for report in &reports {
            let event = report.event.to_ascii_lowercase();
            if event.contains("bugcheck") || event.contains("bluescreen") {
                let Some(code) = wer_field(&report.fields, "bccode")
                    .as_deref()
                    .and_then(bugcheck_code)
                else {
                    continue;
                };
                *seen.entry(("bugcheck".to_string(), code)).or_insert(0) += 1;
            } else if event.contains("appcrash") || event.contains("apphang") {
                // The faulting module, not the application: the application is the victim, and the
                // event log already names it. This is the driver or DLL that took it down.
                let module = wer_field(&report.fields, "fault module name")
                    .filter(|m| !m.is_empty() && !m.eq_ignore_ascii_case("unknown"));
                if let Some(module) = module {
                    *seen
                        .entry((
                            if event.contains("hang") { "hangModule" } else { "crashModule" }.to_string(),
                            short_subject(&module),
                        ))
                        .or_insert(0) += 1;
                }
            }
        }
        let found = !seen.is_empty();
        for ((kind, subject), count) in seen {
            let mut f = finding(&kind, "crit", &subject, &count.to_string());
            f.cause = if kind == "bugcheck" {
                bugcheck_cause(&subject)
            } else {
                // The module's name is in the finding itself; what a person does not know is what a
                // "faulting module" is, and that is the sentence.
                "module".to_string()
            };
            findings.push(f);
        }
        // Green when nothing is in the archive, red when a crash report is: this is the check that
        // answers "did anything crash on this machine", so it is the one that must not lie.
        checks.push(check("wer", if found { "bad" } else { "ok" }));
    }

    // ── Deep only: a program running out of a temp or downloads folder. Not proof of anything, so
    // it is a note — but it is the one line in this report that can be about something unsafe.
    if deep {
        let odd = odd_processes();
        for name in &odd {
            findings.push(finding("oddProcess", "info", name, ""));
        }
        // Amber only when the check actually found something: a warning colour with nothing behind
        // it is the dot nobody can act on, which is exactly how this one read.
        checks.push(check("origin", if odd.is_empty() { "ok" } else { "warn" }));
    }

    if pending_reboot() {
        findings.push(finding("reboot", "info", "", ""));
        checks.push(check("reboot", "warn"));
    } else {
        checks.push(check("reboot", "ok"));
    }

    let uptime_secs = unsafe { GetTickCount64() } / 1_000;
    if uptime_secs >= 14 * 86_400 {
        findings.push(finding(
            "uptime",
            "info",
            "",
            &(uptime_secs / 86_400).to_string(),
        ));
    }

    // A dry run he can look at without waiting for a real crash: `OCZI_HEALTH_SIM=1` adds the three
    // findings a bad week leaves behind, so the card, the colour and the wording can be checked on
    // a healthy machine. Nothing else about the scan changes.
    if std::env::var("OCZI_HEALTH_SIM").is_ok() {
        let mut bug = finding("bugcheck", "crit", "0x9f", "2");
        bug.cause = bugcheck_cause("0x9f");
        findings.push(bug);
        let mut module = finding("crashModule", "crit", "nvwgf2umx.dll", "3");
        module.cause = "module.nvwgf2umx.dll".to_string();
        findings.push(module);
        findings.push(finding("driverReset", "warn", "nvlddmkm", "4"));
        checks.push(check("wer", "bad"));
    }

    let took = (unsafe { GetTickCount64() } - started) as u32;
    let report = Report {
        score: score(&findings),
        at,
        at_text: crate::reminders::format_local(at),
        took_ms: took,
        samples: load.samples,
        cpu_avg: load.cpu_avg(),
        cpu_max: load.cpu_peak(),
        mem_avg: load.mem_avg(),
        mem_max: load.mem_max,
        cores: load.cores,
        hog: load.hog.clone().map(|(n, _)| n).unwrap_or_default(),
        // Share of the whole machine, not of one core: "100 % of a core" on sixteen reads as a
        // machine at its limit, which is the opposite of what it means.
        hog_pct: load
            .hog
            .as_ref()
            .map(|(_, p)| p / load.cores.max(1) as f64)
            .unwrap_or(0.0),
        findings,
        checks,
        advice: String::new(),
    };
    log::line(format!(
        "health scan{}: {} ({} findings, {} ms, {} samples, {} events, {} disks, own {:.0}% core)",
        if deep { " deep" } else { "" },
        report.score,
        report.findings.len(),
        took,
        report.samples,
        events.len(),
        disks,
        load.own_cpu
    ));
    save_last(&report);
    report
}

/// A restart is waiting once Windows has staged one of these.
fn pending_reboot() -> bool {
    crate::startup::key_exists(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Component Based Servicing\\RebootPending",
    ) || crate::startup::key_exists(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired",
    )
}

// ── The last report ───────────────────────────────────────────────────────────

fn path() -> std::path::PathBuf {
    crate::settings::config_dir().join("health.json")
}

fn save_last(report: &Report) {
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    match serde_json::to_vec_pretty(report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path(), json) {
                log::line(format!("health report not saved: {e}"));
            }
        }
        Err(e) => log::line(format!("health report not encoded: {e}")),
    }
}

/// Hands a finished scan to the island, so every number on screen follows whatever ran — a scan
/// started from the report window included, which is how the card and the window stop disagreeing.
pub fn show(report: &Report) {
    let Some(app) = crate::app() else {
        return;
    };
    if let Err(e) = app.emit_to(crate::island::WINDOW_LABEL, "health", report) {
        log::line(format!("health report not sent: {e}"));
    }
}

/// The last verdict as text, for the model's `health_report` tool. It never scans by itself: a tool
/// call is already a conversation, and a three-second measurement inside one is a stall nobody
/// asked for — the answer says when the numbers are from, and the model can offer a fresh scan.
pub fn describe_last() -> String {
    match last() {
        Some(report) => describe(&report),
        // i18n-ok: model-facing, as above.
        None => "Nie ma jeszcze żadnego skanu zdrowia. Powiedz użytkownikowi, żeby kliknął Health w pigułce PC — wtedy będę wiedzieć, co jest na tej maszynie.".to_string(),
    }
}

/// The verdict the hub's stats line shows before anything is scanned again this session.
pub fn last() -> Option<Report> {
    let bytes = std::fs::read(path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

// ── Fixes ─────────────────────────────────────────────────────────────────────

/// The one place that changes the machine, and only for an action the user picked after reading
/// the finding. Closing a program is the only fix offered so far: it asks the program to close
/// first, exactly as `kill_process` does, and refuses a name it cannot mean.
pub fn fix(action: &str, arg: &str) -> Result<String, String> {
    match action {
        // The safe "remove": Windows' own switch-off, the one Task Manager flips, so the entry is
        // still there to be turned back on and nothing on the disk is touched.
        "disableStartup" => {
            let name = arg.trim();
            if name.is_empty() {
                // i18n-ok: model/UI-facing text, translated where it is shown.
                return Err("Nie wiem, który wpis wyłączyć.".to_string());
            }
            log::line(format!("health fix: switch off startup {name}"));
            crate::startup::disable(name)?;
            // i18n-ok: model/UI-facing, as above.
            Ok(format!("Wpis {name} nie będzie już startował razem z Windows."))
        }
        "closeProcess" => {
            let name = arg.trim();
            if name.is_empty() {
                // i18n-ok: model/UI-facing text, translated where it is shown.
                return Err("Nie wiem, który proces zamknąć.".to_string());
            }
            log::line(format!("health fix: close {name}"));
            crate::shell::kill_process(name)
        }
        other => Err(format!("Nieznana naprawa: {other}")),
    }
}

/// The scan as the text the model reads, and as the answer to `oczi.exe --health`. The sentences
/// are for the model and the window, which translate what they show — see the file header.
pub fn describe(report: &Report) -> String {
    let mut out = format!(
        "Zdrowie komputera: {}%\nSkan: {} ({} ms, {} próbek)\n",
        report.score, report.at_text, report.took_ms, report.samples
    );
    out.push_str(&format!(
        "CPU średnio {:.0}%, maksymalnie {:.0}%; RAM średnio {:.0}%, maksymalnie {:.0}%\n",
        report.cpu_avg, report.cpu_max, report.mem_avg, report.mem_max
    ));
    if !report.hog.is_empty() {
        out.push_str(&format!(
            "Najwięcej CPU: {} ({:.0}%)\n",
            report.hog, report.hog_pct
        ));
    }
    if report.findings.is_empty() {
        out.push_str("Brak znalezisk.\n");
    } else {
        out.push_str(&format!(
            "Znaleziska: {} krytycznych, {} ostrzeżeń, {} informacyjnych\n",
            report.crit(),
            report.warn(),
            report.findings.len() - report.crit() - report.warn()
        ));
        for f in &report.findings {
            out.push_str(&format!(
                "- [{}] {} {} {}\n",
                f.severity, f.kind, f.subject, f.value
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_add_up_to_one_name_and_a_rhythm() {
        let mut watch = Watch::default();
        watch.dpc_peak = 2.0;
        let stall = |at: i64, name: &str, pct: f64| Stall {
            at,
            at_text: String::new(),
            ms: 900,
            top: name.to_string(),
            top_pct: pct,
            dpc: 1.0,
            cpu: 60.0,
            tops: Vec::new(),
        };
        watch.stalls = vec![
            stall(0, "Overwatch", 13.0),
            stall(600, "Overwatch", 11.0),
            stall(1_200, "Overwatch", 13.0),
        ];
        settle_stalls(&mut watch);
        assert_eq!(watch.culprits.len(), 1);
        assert_eq!(watch.culprits[0].name, "Overwatch");
        assert_eq!(watch.culprits[0].count, 3);
        assert_eq!(watch.culprits[0].pct, 13.0);
        assert_eq!(watch.gap_minutes, 10);
        assert!(watch.suspects.is_empty());
    }

    #[test]
    fn a_burst_averages_what_it_sampled_and_keeps_the_worst() {
        let mut b = Burst::default();
        fold(&mut b, 10.0, 40.0, None);
        fold(&mut b, 30.0, 55.0, Some(("chrome".to_string(), 12.0)));
        fold(&mut b, 20.0, 45.0, Some(("anticheat".to_string(), 61.0)));
        settle_hog(&mut b);
        assert_eq!(b.samples, 3);
        assert!((b.cpu_avg() - 20.0).abs() < f64::EPSILON);
        assert!((b.cpu_max - 30.0).abs() < f64::EPSILON);
        assert!((b.mem_avg() - 46.666_666_666_666_668).abs() < 1e-9);
        assert!((b.mem_max - 55.0).abs() < f64::EPSILON);
        // The hog is whoever averaged the most over the window — 61 % of a core once is 20 % of it
        // across three samples, which is the number the card shows.
        assert_eq!(b.hog.as_ref().map(|(n, _)| n.as_str()), Some("anticheat"));
        assert!((b.hog.as_ref().unwrap().1 - 20.333_333_333_333_332).abs() < 1e-9);
    }

    #[test]
    fn one_short_spike_does_not_make_a_process_the_hog() {
        let mut b = Burst::default();
        // A tool that woke up for a quarter of a second and went back to sleep — measured over the
        // whole scan this is 10 % of a core, which is not a machine being held back by anything.
        for _ in 0..9 {
            fold(&mut b, 4.0, 40.0, Some(("python".to_string(), 0.0)));
        }
        fold(&mut b, 30.0, 40.0, Some(("python".to_string(), 100.0)));
        settle_hog(&mut b);
        assert_eq!(b.hog, None);
    }

    #[test]
    fn a_steady_consumer_beats_a_louder_one_off() {
        let mut b = Burst::default();
        // Eight samples: one program at 60 % of a core in four of them, another touching 100 % once.
        // The steady one averaged 30 % over the window and is the answer the report wants; the
        // single 100 % is 12.5 % of the average and would have won under the old peak rule.
        for step in 0..4 {
            fold(&mut b, 40.0, 40.0, Some(("steady".to_string(), 60.0)));
            let spike = if step == 0 { 100.0 } else { 0.0 };
            fold(&mut b, 40.0, 40.0, Some(("spike".to_string(), spike)));
        }
        settle_hog(&mut b);
        assert_eq!(b.hog.as_ref().map(|(n, _)| n.as_str()), Some("steady"));
    }

    #[test]
    fn windows_own_processes_are_never_named_as_the_hog() {
        let mut b = Burst::default();
        for _ in 0..4 {
            fold(&mut b, 40.0, 40.0, Some(("audiodg".to_string(), 90.0)));
            fold(&mut b, 40.0, 40.0, Some(("msedgewebview2".to_string(), 80.0)));
        }
        settle_hog(&mut b);
        assert_eq!(b.hog, None);
    }

    #[test]
    fn one_critical_finding_weighs_more_than_a_dozen_notes() {
        let mut findings = vec![finding("cpuHigh", "warn", "", "97")];
        assert_eq!(score(&findings), 92);
        findings.push(finding("hardware", "crit", "WHEA", "3"));
        assert_eq!(score(&findings), 70);
        findings.push(finding("uptime", "info", "", "20"));
        assert_eq!(score(&findings), 68);
        // …and it never reads as a dead machine.
        let many: Vec<Finding> = (0..20)
            .map(|_| finding("hardware", "crit", "WHEA", "1"))
            .collect();
        assert_eq!(score(&many), 5);
    }

    #[test]
    fn heavy_load_is_a_finding_and_normal_load_is_not() {
        let mut b = Burst::default();
        fold(&mut b, 40.0, 60.0, None);
        assert!(load_findings(&b).is_empty());
        fold(&mut b, 97.0, 93.0, None);
        let findings = load_findings(&b);
        let kinds: Vec<&str> = findings.iter().map(|f| f.kind.as_str()).collect();
        assert_eq!(kinds, vec!["cpuHigh", "memHigh"]);
    }

    #[test]
    fn the_event_xml_is_read_without_a_parser() {
        let xml = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><System><Provider Name="Microsoft-Windows-Kernel-Power" Guid="{331c3b3a}"/><EventID>41</EventID><Level>1</Level><TimeCreated SystemTime="2026-10-20T18:02:11.0000000Z"/></System><EventData><Data Name="BugcheckCode">0</Data><Data Name="BugcheckParameter1">0x0</Data></EventData></Event>"#;
        let event = parse_event(xml).expect("an event");
        assert_eq!(event.id, 41);
        assert_eq!(event.provider, "Microsoft-Windows-Kernel-Power");
        assert_eq!(classify(&event), Some(("restart", "crit")));
        assert_eq!(data_values(xml), vec!["0".to_string(), "0x0".to_string()]);
        // The entry a machine grows every day that means nothing.
        assert_eq!(
            classify(&Event {
                id: 10016,
                provider: "Microsoft-Windows-DistributedCOM".to_string(),
                data: Vec::new(),
            }),
            None
        );
    }

    #[test]
    fn the_wer_archive_is_read_by_label_with_its_sibling_value() {
        // A real archive file, cut to the lines this module reads.
        let text = "\u{feff}Version=1\r\nEventType=Bugcheck\r\nEventTime=133721446115508226\r\n\
                    Sig[0].Name=BCCode\r\nSig[0].Value=159\r\n\
                    Sig[1].Name=Parameter1\r\nSig[1].Value=0xffffd48f9a2c5120\r\n";
        let fields = parse_wer(text);
        // The label is a value, its answer is the field beside it: reading "BCCode" as a code is
        // the mistake this shape exists to prevent.
        assert_eq!(wer_field(&fields, "bccode").as_deref(), Some("159"));
        assert_eq!(bugcheck_code("159").as_deref(), Some("0x9f"));
        assert_eq!(bugcheck_cause("0x9f"), "bugcheck.0x9f");
        assert_eq!(bugcheck_cause("0xdead"), "bugcheck.unknown");
        // A zero code is not a bugcheck, and neither is nonsense.
        assert_eq!(bugcheck_code("0"), None);
        assert_eq!(bugcheck_code("zzz"), None);
    }

    #[test]
    fn an_app_crash_report_names_the_module_that_took_it_down() {
        let text = "EventType=APPCRASH\r\n\
                    Sig[0].Name=Application Name\r\nSig[0].Value=game.exe\r\n\
                    Sig[3].Name=Fault Module Name\r\nSig[3].Value=nvwgf2umx.dll\r\n";
        let fields = parse_wer(text);
        assert_eq!(
            wer_field(&fields, "fault module name").as_deref(),
            Some("nvwgf2umx.dll")
        );
        // The application is the victim, not the culprit, so it is not what gets reported here.
        assert_eq!(
            wer_field(&fields, "application name").as_deref(),
            Some("game.exe")
        );
    }

    #[test]
    fn forty_crashes_of_one_program_are_one_finding_named_after_the_program() {
        let crash = |app: &str| Event {
            id: 1000,
            provider: "Application Error".to_string(),
            data: vec![app.to_string()],
        };
        let mut events: Vec<Event> = (0..40).map(|_| crash("C:\\Apps\\Game\\game.exe")).collect();
        events.push(Event {
            id: 1001,
            provider: "Microsoft-Windows-WER-SystemErrorReporting".to_string(),
            data: vec!["0x0000009f".to_string()],
        });
        let findings = event_findings(&events);
        assert_eq!(findings.len(), 2);
        // Sorted loudest first, and the critical one is the loud one.
        assert_eq!(findings[0].kind, "bugcheck");
        assert_eq!(findings[0].severity, "crit");
        assert_eq!(findings[0].subject, "0x0000009f");
        assert_eq!(findings[1].kind, "appCrash");
        assert_eq!(findings[1].value, "40");
        // A path on a card is its last segment, not the whole thing.
        assert_eq!(findings[1].subject, "game.exe");
    }
}
