// Crash: what Windows wrote down about the last time the machine went down on its own — and what
// that adds up to.
//
// A freeze leaves no bugcheck, no dump and no error dialog, so "it hung" is all the log seems to
// say. It says more than that if a few things are read together, which is what this module does in
// one pass, on demand:
//
// * Kernel-Power 41's own parameters. `BugcheckCode` non-zero means the machine went down through a
//   bugcheck and a dump exists somewhere; zero means the kernel never got that far, so it was a
//   hang or a power loss. `PowerButtonTimestamp` / `LongPowerButtonPressDetected` separate "someone
//   held the button" from "nobody touched it".
// * The dump files themselves. Windows writes them *after* a bugcheck writes its last words, so an
//   empty `Minidump` folder next to a 41 is the proof that no bugcheck ever happened.
// * WHEA-Logger. A machine-check the hardware reported is the one crash cause that leaves a trace
//   without any software cooperation.
// * The last thing Windows logged before it stopped, timed against the boot that followed. The gap
//   is how long the machine was gone, and the events in the minutes before it are the candidates
//   for a trigger — a power-mode switch by a helper program (the platform's own known weak spot
//   here), a sleep transition, a driver installed an hour earlier.
// * The history. One hang is an accident; the same verdict three times is a pattern, and a pattern
//   is the answer to "why does this keep happening".
//
// Nothing here runs on a timer: it is one pass at startup and one per health scan, and it changes
// nothing on the machine.

use serde::{Deserialize, Serialize};

use crate::health::{boot_local, query_events};
use crate::log;

/// How far back the forensic queries look. The event that marks a crash is written at the boot
/// after it, so the window has to reach past the last restart, not just to now.
const WINDOW_HOURS: i64 = 48;
/// A power-mode switch, a sleep transition or a firmware throttle is a *trigger* only when it lands
/// this close to the moment the machine stopped. Fifteen minutes is generous enough for a hang that
/// takes a while to bite and tight enough that yesterday's settings change is not blamed.
const TRIGGER_MINUTES: i64 = 15;
/// A driver installed this long before the hang is still a suspect: an unsigned or broken .sys
/// usually takes a day or two to show, but the first blue screen often comes within the hour.
const DRIVER_MINUTES: i64 = 360;
/// A hardware error this close to the hang is worth naming as the cause.
const HARDWARE_MINUTES: i64 = 30;

/// One crash, as it is stored and as the model reads it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Crash {
    /// When the machine stopped, in local wall-clock seconds: the last thing Windows logged before
    /// it went quiet, or the boot when even that is missing.
    pub at: i64,
    pub at_text: String,
    /// When the session that follows it started.
    pub boot_at: i64,
    pub boot_text: String,
    /// `hang` (no bugcheck, nobody pressed anything), `bugcheck`, `forced` (the button), `sleep`.
    pub kind: String,
    /// What the pass concluded, as a token the language tables carry: `powerMode`, `sleep`,
    /// `driver`, `throttle`, `hardware`, `bugcheck`, `forced`, `noEvidence`.
    pub verdict: String,
    /// How long before the stop the trigger landed. Zero when there is no trigger.
    pub minutes: i64,
    /// The program, driver or power source that landed there.
    pub trigger: String,
    /// The bugcheck code, when there was one.
    pub bugcheck: String,
    /// Which dump file is on disk, if any — or that there is none, which is itself the answer.
    pub dump: String,
    /// Hardware errors (WHEA) around the stop.
    pub whea: u32,
    /// Seconds between the last log entry and the boot. The machine was gone for at least this long.
    pub silence: i64,
    /// The pass's own findings, one short line each: what it read, and what it ruled out.
    pub evidence: Vec<String>,
    /// The Windows power mode right now — the setting this platform is known to hang on.
    pub power_mode: String,
    /// How many of the stored crashes share this verdict, and how many are stored at all.
    pub repeats: u32,
    pub total: u32,
}

/// The one-line-per-crash form kept in `crashes.json`: enough to recognise a pattern, small enough
/// that a year of them costs nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Brief {
    pub boot_at: i64,
    pub at: i64,
    pub at_text: String,
    pub kind: String,
    pub verdict: String,
    pub trigger: String,
    pub minutes: i64,
}

fn window_ms(hours: i64) -> i64 {
    hours * 3_600 * 1_000
}

/// The interesting slice of the System log, newest first. The XPath filters inside Windows, so a
/// busy log is not read event by event — and the ids are the ones a machine's death and the minutes
/// before it are made of.
fn markers(hours: i64) -> Vec<crate::health::Event> {
    let xpath = format!(
        "*[System[TimeCreated[timediff(@SystemTime) <= {}] and (EventID=41 or EventID=42 or \
         EventID=107 or EventID=105 or EventID=109 or EventID=12 or EventID=51 or EventID=37 or \
         EventID=1001 or EventID=7045 or EventID=6008 or \
         Provider[@Name='Microsoft-Windows-WHEA-Logger'])]]",
        window_ms(hours)
    );
    query_events("System", &xpath, 400, true)
}

/// The last thing anything logged before this session's boot. Walking newest-first and stopping at
/// the first event older than the boot is the only way to place the moment the machine stopped: the
/// log has no "the machine died here" entry, it simply has nothing after it.
fn last_before_boot(boot: i64) -> Option<crate::health::Event> {
    let xpath = format!(
        "*[System[TimeCreated[timediff(@SystemTime) <= {}]]]",
        window_ms(12)
    );
    query_events("System", &xpath, 800, true)
        .into_iter()
        .find(|e| e.at > 0 && e.at < boot - 3)
}

/// A numeric parameter of an event, as the event itself labels it.
fn named_num(e: &crate::health::Event, name: &str) -> u64 {
    e.named
        .get(name)
        .and_then(|raw| {
            let raw = raw.trim();
            match raw
                .strip_prefix("0x")
                .or_else(|| raw.strip_prefix("0X"))
            {
                Some(hex) => u64::from_str_radix(hex, 16).ok(),
                None => raw.parse::<u64>().ok(),
            }
        })
        .unwrap_or(0)
}

fn named_bool(e: &crate::health::Event, name: &str) -> bool {
    e.named
        .get(name)
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// A path as a program name: the last segment, which is what a person recognises in a card.
fn short(raw: &str) -> String {
    raw.rsplit(['\\', '/'])
        .next()
        .unwrap_or(raw)
        .trim()
        .chars()
        .take(48)
        .collect()
}

/// The newest dump on disk, with the moment it was written and its size. `max_age_days` is what
/// keeps a month-old file from answering a question about last night.
fn newest_dump(now: i64, max_age_days: i64) -> Option<(std::path::PathBuf, i64, u64)> {
    let at_of = |meta: &std::fs::Metadata| {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + crate::health::local_offset()
    };
    let mut best: Option<(i64, std::path::PathBuf, u64)> = None;
    for entry in std::fs::read_dir("C:\\Windows\\Minidump")
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || meta.len() == 0 {
            continue;
        }
        let at = at_of(&meta);
        if now - at > max_age_days * 86_400 {
            continue;
        }
        if best.as_ref().map(|(b, _, _)| at > *b).unwrap_or(true) {
            best = Some((at, entry.path(), meta.len()));
        }
    }
    if let Some((at, path, size)) = best {
        return Some((path, at, size));
    }
    let memory = std::path::PathBuf::from("C:\\Windows\\MEMORY.DMP");
    let meta = std::fs::metadata(&memory).ok()?;
    if meta.len() == 0 {
        return None;
    }
    Some((memory, at_of(&meta), meta.len()))
}

/// Which dump file Windows left behind, in one line. `none` is the load-bearing answer: a bugcheck
/// always writes one before it goes down, so no file means no bugcheck.
fn dump_files(now: i64) -> String {
    let Some((path, at, size)) = newest_dump(now, 7) else {
        // i18n-ok: model/log-facing text, translated where a card shows it.
        return "brak".to_string();
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.eq_ignore_ascii_case("MEMORY.DMP") {
        return format!(
            "MEMORY.DMP ({} MB, {})",
            size / 1_048_576,
            crate::reminders::format_local(at)
        );
    }
    format!(
        "{} ({} KB, {})",
        name,
        size / 1024,
        crate::reminders::format_local(at)
    )
}

// ── The dump itself ─────────────────────────────────────────────────────────

/// A first symbol download is the slow part of a first run; the analysis itself is seconds.
const DEBUGGER_TIMEOUT_SECS: u64 = 180;
/// A winget install gets its own budget: it downloads and deploys, where the debugger only reads.
const INSTALL_TIMEOUT_SECS: u64 = 420;

/// The debugger to run: the Store package's aliases first, then a Windows SDK install.
fn debugger() -> Option<std::path::PathBuf> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for alias in ["cdbX64.exe", "cdbX86.exe", "cdbARM64.exe"] {
            candidates.push(
                std::path::PathBuf::from(&local)
                    .join("Microsoft")
                    .join("WindowsApps")
                    .join(alias),
            );
        }
    }
    for kit in [
        "C:\\Program Files (x86)\\Windows Kits\\10\\Debuggers\\x64\\cdb.exe",
        "C:\\Program Files\\Windows Kits\\10\\Debuggers\\x64\\cdb.exe",
    ] {
        candidates.push(std::path::PathBuf::from(kit));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// The dump this process can open: the file itself, or a copy taken under one UAC prompt.
fn readable_copy(src: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if std::fs::File::open(src).is_ok() {
        return Ok(src.to_path_buf());
    }
    let dir = crate::settings::local_dir().join("dumps");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Nie mogę utworzyć {}: {e}", dir.display()))?;
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("dmp"))
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(path);
        }
    }
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "dump.dmp".to_string());
    let dest = dir.join(name);
    // The one prompt this needs: Windows keeps %WINDIR%\Minidump to administrators.
    let script = format!(
        "Copy-Item -LiteralPath '{}' -Destination '{}' -Force",
        src.display(),
        dest.display()
    );
    crate::elevate::run_elevated_wait(&script)?;
    match std::fs::metadata(&dest) {
        Ok(meta) if meta.len() > 0 => Ok(dest),
        // i18n-ok: model/log-facing text.
        _ => Err("Kopia zrzutu się nie udała — plik został nieczytelny nawet z uprawnieniami administratora.".to_string()),
    }
}

/// Runs one program to completion with a deadline and returns both streams and the timeout flag.
fn capture(
    program: &std::path::Path,
    args: &[&str],
    seconds: u64,
) -> Result<(String, bool), String> {
    use std::io::Read;

    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Nie mogę uruchomić {}: {e}", program.display()))?;
    // Both pipes on their own thread: a full stderr pipe stops the child just as dead as stdout.
    let drain = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stream.read_to_string(&mut buf);
            buf
        })
    };
    let out = child.stdout.take().map(|s| drain(Box::new(s)));
    let err = child.stderr.take().map(|s| drain(Box::new(s)));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("{} padł: {e}", program.display())),
        }
    }
    let text = format!(
        "{}\n{}",
        out.and_then(|h| h.join().ok()).unwrap_or_default(),
        err.and_then(|h| h.join().ok()).unwrap_or_default()
    );
    Ok((text, timed_out))
}

/// `!analyze -v` in one pass, with the symbol store the debugger downloads into on first use.
fn run_debugger(
    exe: &std::path::Path,
    dump: &std::path::Path,
    symbols: &std::path::Path,
) -> Result<String, String> {
    // Native backslashes: DbgHelp rejects the store outright when the path arrives as C:/..., and the
    let store = symbols.display().to_string().replace('/', "\\");
    let sympath = format!("srv*{store}*https://msdl.microsoft.com/download/symbols");
    let (text, timed_out) = capture(
        exe,
        &[
            "-z",
            &dump.to_string_lossy(),
            "-y",
            &sympath,
            "-c",
            "!analyze -v;q",
        ],
        DEBUGGER_TIMEOUT_SECS,
    )?;
    if timed_out && !text.contains("Bugcheck Analysis") {
        // i18n-ok: model/log-facing text.
        return Err(format!(
            "Debugger nie skończył w {DEBUGGER_TIMEOUT_SECS} s — pierwsze ładowanie symboli bywa wolne, więc druga próba zwykle jest szybka."
        ));
    }
    Ok(text)
}

/// The debugger, installed with one winget run when the machine has none.
fn ensure_debugger() -> Result<std::path::PathBuf, String> {
    if let Some(exe) = debugger() {
        return Ok(exe);
    }
    log::line("dump  no debugger installed — installing WinDbg from the Store".to_string());
    let powershell = std::path::Path::new("powershell.exe");
    let (text, timed_out) = capture(
        powershell,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "winget install --id Microsoft.WinDbg --accept-source-agreements --accept-package-agreements --disable-interactivity",
        ],
        INSTALL_TIMEOUT_SECS,
    )?;
    if timed_out {
        log::line("dump  the WinDbg install did not finish in time".to_string());
    }
    match debugger() {
        Some(exe) => {
            log::line(format!("dump  debugger ready: {}", exe.display()));
            Ok(exe)
        }
        // i18n-ok: model/log-facing text.
        None => Err(format!(
            "Nie ma debuggera i nie udało się go doinstalować. Ostatnie linie instalacji:\n{}",
            text.lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        )),
    }
}

/// The firmware and the newest drivers, so the answer can name what is worth updating.
fn firmware_and_drivers() -> String {
    let script = "Get-CimInstance Win32_BIOS | ForEach-Object { 'BIOS ' + $_.SMBIOSBIOSVersion + ' (' + $_.ReleaseDate.ToString('yyyy-MM-dd') + ')' }; Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceName -and $_.DriverDate } | Sort-Object DriverDate -Descending | Select-Object -First 6 | ForEach-Object { $_.DriverDate.ToString('yyyy-MM-dd') + ' | ' + $_.DriverProviderName + ' | ' + $_.DeviceName + ' | ' + $_.DriverVersion }";
    match capture(
        std::path::Path::new("powershell.exe"),
        &["-NoProfile", "-NonInteractive", "-Command", script],
        45,
    ) {
        Ok((text, _)) => text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Err(_) => String::new(),
    }
}

/// What `!analyze -v` answered, in the fields a person asks about.
struct Analysis {
    bugcheck_name: String,
    bugcheck_code: String,
    exception: String,
    access: String,
    module: String,
    image: String,
    bucket: String,
    process: String,
    session: String,
    frames: Vec<String>,
    stack_command: String,
}

/// One `KEY: value` line of the debugger's own output.
fn field(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|line| {
            line.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(':'))
                .map(|value| value.trim().to_string())
        })
        .unwrap_or_default()
}

/// The name the debugger puts in front of the code: `SYSTEM_THREAD_EXCEPTION_NOT_HANDLED (7e)`.
fn bugcheck_name(text: &str, code: &str) -> String {
    if code.is_empty() {
        return String::new();
    }
    let want = format!("({code})");
    text.lines()
        .map(str::trim)
        .find(|line| line.ends_with(&want) && !line.contains(':') && line.len() < want.len() + 64)
        .map(|line| line.trim_end_matches(&want).trim().to_string())
        .unwrap_or_default()
}

/// The `AV.Type` key, which says whether the bad access was a read, a write or an execute.
fn access_kind(text: &str) -> String {
    let mut wanted = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("Key") && line.contains("AV.Type") {
            wanted = true;
            continue;
        }
        if wanted {
            if let Some(value) = line.strip_prefix("Value") {
                return value.trim_start_matches(':').trim().to_string();
            }
            wanted = false;
        }
    }
    String::new()
}

/// The frames under `STACK_TEXT:`, symbol side only, since the hex is already in the log.
fn stack_frames(text: &str) -> Vec<String> {
    let mut frames = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("STACK_TEXT:") {
            inside = true;
            continue;
        }
        if !inside {
            continue;
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with("SYMBOL_NAME") {
            if frames.is_empty() {
                continue;
            }
            break;
        }
        if let Some(frame) = line.rsplit(" : ").next() {
            let frame = frame.trim();
            let clean = frame.contains('!') && !frame.contains(char::is_whitespace);
            if clean && !frames.iter().any(|f| f == frame) {
                frames.push(frame.to_string());
            }
        }
        if frames.len() >= 12 {
            break;
        }
    }
    frames
}

impl Analysis {
    fn parse(text: &str) -> Self {
        let code = field(text, "BUGCHECK_CODE");
        Analysis {
            bugcheck_name: bugcheck_name(text, &code),
            exception: field(text, "EXCEPTION_CODE"),
            access: access_kind(text),
            module: field(text, "SYMBOL_NAME"),
            image: match (field(text, "IMAGE_NAME"), field(text, "IMAGE_VERSION")) {
                (image, version) if !image.is_empty() && !version.is_empty() => {
                    format!("{image} {version}")
                }
                (image, _) => image,
            },
            bucket: field(text, "FAILURE_BUCKET_ID"),
            process: field(text, "PROCESS_NAME"),
            // Not a `KEY: value` line: the debugger prints it as prose.
            session: text
                .lines()
                .find_map(|line| line.trim().strip_prefix("Debug session time:"))
                .map(|rest| rest.trim().to_string())
                .unwrap_or_default(),
            frames: stack_frames(text),
            stack_command: field(text, "STACK_COMMAND"),
            bugcheck_code: code,
        }
    }

    fn known(&self) -> bool {
        u32::from_str_radix(self.bugcheck_code.trim(), 16).unwrap_or(0) != 0
    }

    /// The report the model answers from: what went down, what took it down, and the stack that says
    fn describe(&self, name: &str, size: u64, at: i64) -> String {
        let mut out = String::new();
        // i18n-ok: model/log-facing text.
        out.push_str(&format!(
            "Zrzut: {name} ({} KB, {})\n",
            size / 1024,
            crate::reminders::format_local(at)
        ));
        let mut head = format!("BUGCHECK: 0x{}", self.bugcheck_code);
        if !self.bugcheck_name.is_empty() {
            head.push(' ');
            head.push_str(&self.bugcheck_name);
        }
        if !self.access.is_empty() {
            // i18n-ok: model/log-facing text.
            head.push_str(&format!(" · błąd dostępu ({})", self.access));
        }
        out.push_str(&head);
        out.push('\n');
        if !self.module.is_empty() {
            // i18n-ok: model/log-facing text.
            out.push_str(&format!(
                "WINNY: {}{}{}\n",
                self.module,
                if self.image.is_empty() {
                    String::new()
                } else {
                    format!(" w {}", self.image)
                },
                if self.process.is_empty() {
                    String::new()
                } else {
                    format!(" (proces {})", self.process)
                }
            ));
        }
        if !self.exception.is_empty() && self.exception != self.bugcheck_code {
            // i18n-ok: model-facing text.
            out.push_str(&format!("KOD WYJĄTKU: {}\n", self.exception));
        }
        if !self.bucket.is_empty() {
            out.push_str(&format!("BUCKET: {}\n", self.bucket));
        }
        if !self.session.is_empty() {
            // i18n-ok: model-facing text.
            out.push_str(&format!("CZAS ZRZUTU: {}\n", self.session));
        }
        if !self.frames.is_empty() {
            // i18n-ok: model/log-facing text.
            out.push_str("STOS:\n");
            for (index, frame) in self.frames.iter().enumerate() {
                out.push_str(&format!("  {}. {frame}\n", index + 1));
            }
        }
        if !self.stack_command.is_empty() {
            out.push_str(&format!("STACK_COMMAND: {}\n", self.stack_command));
        }
        out.trim_end().to_string()
    }
}

/// The newest dump read by Microsoft's debugger, through everything Windows puts in front of it.
pub fn analyze_dump() -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
        + crate::health::local_offset();
    let Some((dump, at, size)) = newest_dump(now, 30) else {
        // i18n-ok: model/log-facing text.
        return Err("Nie ma żadnego zrzutu: Windows nie zapisał ani jednego, więc nie było bugchecka. crash_report mówi, co w takim razie zatrzymało maszynę.".to_string());
    };
    let name = dump
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    log::line(format!("dump  analysing {name} ({size} B)"));
    let exe = ensure_debugger()?;
    let local = readable_copy(&dump)?;
    let symbols = crate::settings::local_dir().join("symbols");
    let _ = std::fs::create_dir_all(&symbols);
    let text = run_debugger(&exe, &local, &symbols)?;
    let analysis = Analysis::parse(&text);
    if text.contains("WRONG_SYMBOLS") {
        log::line("dump  the debugger ran without matching symbols".to_string());
        // i18n-ok: model/log-facing text.
        return Err("Debugger przeczytał zrzut bez pasujących symboli, więc nazwy nie znaczą nic. To zwykle pierwsze uruchomienie — powtórz, symbole są już pobrane.".to_string());
    }
    if !analysis.known() {
        let head: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty() && !line.contains("NatVis") && !line.contains("DBGHELP")
            })
            .take(15)
            .collect();
        log::line("dump  the debugger found no bugcheck in the file".to_string());
        // i18n-ok: model/log-facing text.
        return Err(format!(
            "Debugger nie znalazł bugchecka w {name}. Jego pierwsze linie:\n{}",
            head.join("\n")
        ));
    }
    let mut report = analysis.describe(&name, size, at);
    let hardware = firmware_and_drivers();
    if !hardware.is_empty() {
        // i18n-ok: model/log-facing text.
        report.push_str("\n\nBIOS i najnowsze sterowniki:\n");
        report.push_str(&hardware);
    }
    log::line(format!(
        // i18n-ok: model/log-facing text.
        "dump  bugcheck 0x{} {} · {}",
        analysis.bugcheck_code, analysis.bugcheck_name, analysis.module
    ));
    Ok(report)
}

/// The analysis text for a debugger log the caller already has, so a test needs no dump or UAC prompt.
#[cfg(test)]
fn describe_text(text: &str) -> String {
    Analysis::parse(text).describe("sample.dmp", 3022908, 0)
}

/// The Windows power mode, by the GUIDs its own overlay settings hold. This is the setting a
pub fn power_mode() -> String {
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    const PATH: &str = "SYSTEM\\CurrentControlSet\\Control\\Power\\User\\PowerSchemes";
    let name_of = |guid: &str| match guid.trim().to_ascii_lowercase().as_str() {
        "961cc777-2547-4f9d-8174-7d86181b8a7a" => "Best performance".to_string(),
        "ded574b5-45a0-4f42-8737-46345c09c238" => "Best power efficiency".to_string(),
        "00000000-0000-0000-0000-000000000000" => "Balanced".to_string(),
        other => other.to_string(),
    };
    let ac = crate::registry::read_string(HKEY_LOCAL_MACHINE, PATH, "ActiveOverlayAcPowerScheme", true)
        .map(|g| name_of(&g));
    let dc = crate::registry::read_string(HKEY_LOCAL_MACHINE, PATH, "ActiveOverlayDcPowerScheme", true)
        .map(|g| name_of(&g));
    match (ac, dc) {
        // i18n-ok: model/log-facing text.
        (Some(ac), Some(dc)) if ac == dc => format!("{ac} (zasilacz i bateria)"),
        (Some(ac), Some(dc)) => format!("{ac} (zasilacz), {dc} (bateria)"),
        (Some(ac), None) => format!("{ac} (zasilacz)"),
        (None, Some(dc)) => format!("{dc} (bateria)"),
        // i18n-ok: model/log-facing text.
        (None, None) => "nieznany (brak wartości w rejestrze)".to_string(),
    }
}

/// What a crash's own trigger events say, in the order they matter: a hardware error outranks a
struct Trigger {
    verdict: &'static str,
    subject: String,
    minutes: i64,
}

/// The best candidate for "what happened just before", out of everything the log holds.
fn trigger(events: &[crate::health::Event], death: i64) -> Option<Trigger> {
    let mut best: Option<(u8, Trigger)> = None;
    let mut consider = |rank: u8, verdict: &'static str, subject: String, at: i64| {
        // Rounded up: "0 min before" reads as "at the same moment", which nothing in a log is.
        let minutes = (death - at + 59) / 60;
        if minutes < 0 || minutes > DRIVER_MINUTES {
            return;
        }
        let limit = if verdict == "driver" {
            DRIVER_MINUTES
        } else if verdict == "hardware" {
            HARDWARE_MINUTES
        } else {
            TRIGGER_MINUTES
        };
        if minutes > limit {
            return;
        }
        if best.as_ref().map(|(r, _)| rank < *r).unwrap_or(true) {
            best = Some((
                rank,
                Trigger {
                    verdict,
                    subject,
                    minutes,
                },
            ));
        }
    };

    for e in events {
        let provider = e.provider.trim_start_matches("Microsoft-Windows-");
        match (provider, e.id) {
            // A program resetting the power scheme — the switch this platform hangs on.
            ("UserModePowerService", 12) | ("UserModePowerService", 51) => {
                let who = e
                    .data
                    .first()
                    .map(|raw| short(raw))
                    .filter(|s| !s.is_empty())
                    // i18n-ok: model/log-facing text.
                    .unwrap_or_else(|| "program".to_string());
                consider(2, "powerMode", who, e.at);
            }
            // AC ↔ battery. A different transition, the same family of failure.
            ("Kernel-Power", 105) => {
                let what = match e.data.first().map(|s| s.trim()) {
                    Some("1") => "zasilacz",
                    Some("0") => "bateria",
                    _ => "",
                };
                consider(3, "powerSource", what.to_string(), e.at);
            }
            // A sleep transition, or the wake that follows it.
            ("Kernel-Power", 42) | ("Kernel-Power", 107) => {
                consider(4, "sleep", String::new(), e.at);
            }
            // A driver or service installed: the usual suspect for a hang nobody can explain.
            ("Service Control Manager", 7045) => {
                let name = e
                    .named
                    .get("ServiceName")
                    .map(|s| short(s))
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                consider(5, "driver", name, e.at);
            }
            // The firmware holding the processor back hard enough to look like a freeze.
            ("Kernel-Processor-Power", 37) => {
                consider(6, "throttle", String::new(), e.at);
            }
            ("WHEA-Logger", _) => {
                consider(0, "hardware", String::new(), e.at);
            }
            _ => {}
        }
    }
    best.map(|(_, trigger)| trigger)
}

/// One forensic pass over the boot this session followed. `None` when the previous shutdown was a
pub fn inspect() -> Option<Crash> {
    let boot = boot_local();
    let now = crate::reminders::now_local();
    let events = markers(WINDOW_HOURS);

    // The markers are written *at* the boot they describe, so the one that matters is the newest
    let dead_marker = events
        .iter()
        .find(|e| e.provider.ends_with("Kernel-Power") && e.id == 41 && (e.at - boot).abs() <= 900);
    let clean_marker = events
        .iter()
        .find(|e| e.provider == "EventLog" && e.id == 6008 && (e.at - boot).abs() <= 900);
    if dead_marker.is_none() && clean_marker.is_none() {
        return None;
    }

    // When it stopped: the last thing logged before the machine went quiet. Falling back to the
    let last = last_before_boot(boot);
    let death = last.as_ref().map(|e| e.at).unwrap_or(boot);

    // Only what was written *before* the machine stopped. The boot that follows writes its own
    let cutoff = if last.is_some() { death } else { boot - 3 };
    let pre_boot: Vec<crate::health::Event> = events
        .iter()
        .filter(|e| e.at > 0 && e.at < cutoff - 3)
        .cloned()
        .collect();

    let mut evidence: Vec<String> = Vec::new();
    // i18n-ok: model/log-facing lines; a card shows the token form.
    evidence.push(format!(
        // i18n-ok: model/log-facing text.
        "zgasł: {} → wstał: {} ({} s ciszy w dzienniku)",
        crate::reminders::format_local(death),
        crate::reminders::format_local(boot),
        (boot - death).max(0)
    ));
    if let Some(last) = &last {
        evidence.push(format!(
            // i18n-ok: model/log-facing text.
            "ostatni wpis przed ciszą: {} / {}",
            last.provider.trim_start_matches("Microsoft-Windows-"),
            last.id
        ));
    }

    let bugcheck_code = dead_marker
        .map(|e| named_num(e, "BugcheckCode"))
        .unwrap_or(0);
    let button_stamp = dead_marker
        .map(|e| named_num(e, "PowerButtonTimestamp"))
        .unwrap_or(0);
    let long_press = dead_marker
        .map(|e| named_bool(e, "LongPowerButtonPressDetected"))
        .unwrap_or(false);
    let sleep_in_progress = dead_marker
        .map(|e| named_num(e, "SleepInProgress"))
        .unwrap_or(0);
    let whea_boot = dead_marker
        .map(|e| named_num(e, "WHEABootErrorCount"))
        .unwrap_or(0);

    let whea = pre_boot
        .iter()
        .filter(|e| e.provider.ends_with("WHEA-Logger"))
        .count() as u32
        + whea_boot as u32;

    let kind = if bugcheck_code != 0 {
        "bugcheck"
    } else if button_stamp != 0 || long_press {
        "forced"
    } else if sleep_in_progress != 0 {
        "sleep"
    } else {
        "hang"
    };

    let bugcheck = if bugcheck_code != 0 {
        format!("0x{bugcheck_code:x}")
    } else {
        String::new()
    };

    let dump = dump_files(now);

    // What the 41 itself says. These four parameters are the difference between "it hung" and "it
    if let Some(marker) = dead_marker {
        evidence.push(format!(
            "41: BugcheckCode={} PowerButtonTimestamp={} LongPress={} SleepInProgress={}",
            named_num(marker, "BugcheckCode"),
            named_num(marker, "PowerButtonTimestamp"),
            named_bool(marker, "LongPowerButtonPressDetected"),
            named_num(marker, "SleepInProgress"),
        ));
    }
    evidence.push(format!(
        // i18n-ok: model/log-facing text.
        "BUGCHECK: {} · zrzut: {dump} · WHEA: {whea}",
        if bugcheck.is_empty() {
            // i18n-ok: model/log-facing text.
            "brak"
        } else {
            bugcheck.as_str()
        }
    ));

    let found = trigger(&pre_boot, death);
    let (verdict, minutes, subject) = match (kind, found.as_ref()) {
        ("bugcheck", _) => ("bugcheck".to_string(), 0, String::new()),
        (_, Some(t)) => (t.verdict.to_string(), t.minutes, t.subject.clone()),
        ("forced", _) => ("forced".to_string(), 0, String::new()),
        _ => ("noEvidence".to_string(), 0, String::new()),
    };

    // Every candidate goes into the evidence, not just the winner: on a machine that hangs for a
    let mut seen: Vec<String> = Vec::new();
    for e in &pre_boot {
        let provider = e.provider.trim_start_matches("Microsoft-Windows-");
        let what = match (provider, e.id) {
            ("UserModePowerService", 12) | ("UserModePowerService", 51) => Some(format!(
                // i18n-ok: model/log-facing.
                "tryb zasilania przełączony przez {}",
                e.data.first().map(|r| short(r)).unwrap_or_default()
            )),
            // i18n-ok: model/log-facing text.
            ("Kernel-Power", 105) => Some("zmiana źródła zasilania".to_string()),
            ("Kernel-Power", 42) | ("Kernel-Power", 107) => {
                // i18n-ok: model/log-facing.
                Some("przejście snu/czuwania".to_string())
            }
            ("Service Control Manager", 7045) => Some(format!(
                // i18n-ok: model/log-facing.
                "zainstalowany sterownik/usługa: {}",
                e.named.get("ServiceName").cloned().unwrap_or_default()
            )),
            ("Kernel-Processor-Power", 37) => {
                // i18n-ok: model/log-facing.
                Some("firmware ograniczył procesor".to_string())
            }
            ("WHEA-Logger", _) => Some(format!(
                // i18n-ok: model/log-facing.
                "błąd sprzętowy WHEA (id {})",
                e.id
            )),
            _ => None,
        };
        if let Some(what) = what {
            // Same window as the verdict: an event from before the last boot is not what made the
            let minutes = (death - e.at + 59) / 60;
            if minutes < 0 || minutes > DRIVER_MINUTES {
                continue;
            }
            let line = format!("{minutes} min przed: {what}");
            if !seen.contains(&line) {
                seen.push(line);
            }
        }
    }
    evidence.extend(seen.into_iter().take(8));

    let power_mode = power_mode();
    // i18n-ok: model/log-facing.
    evidence.push(format!("tryb zasilania teraz: {power_mode}"));

    Some(Crash {
        at: death,
        at_text: crate::reminders::format_local(death),
        boot_at: boot,
        boot_text: crate::reminders::format_local(boot),
        kind: kind.to_string(),
        verdict,
        minutes,
        trigger: subject,
        bugcheck,
        dump,
        whea,
        silence: (boot - death).max(0),
        evidence,
        power_mode,
        repeats: 0,
        total: 0,
    })
}

// ── The history ──────────────────────────────────────────────────────────────

fn path() -> std::path::PathBuf {
    crate::settings::config_dir().join("crashes.json")
}

fn last_path() -> std::path::PathBuf {
    crate::settings::config_dir().join("crash.json")
}

fn load_history() -> Vec<Brief> {
    std::fs::read(path())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_history(history: &[Brief]) {
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    match serde_json::to_vec_pretty(history) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path(), json) {
                log::line(format!("crash history not saved: {e}"));
            }
        }
        Err(e) => log::line(format!("crash history not encoded: {e}")),
    }
}

/// The full report of the newest crash, for the model's tool and the log.
fn save_last(crash: &Crash) {
    let dir = crate::settings::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_vec_pretty(crash) {
        let _ = std::fs::write(last_path(), json);
    }
}

/// The last crash as it was read, from disk. `None` before the first pass of this install.
pub fn last() -> Option<Crash> {
    let bytes = std::fs::read(last_path()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The pass plus the bookkeeping: store the crash once per boot, count how often the same verdict
pub fn record() -> Option<Crash> {
    let mut crash = inspect()?;
    let mut history = load_history();
    let first_time = !history
        .iter()
        .any(|b| (b.boot_at - crash.boot_at).abs() < 120);
    if first_time {
        history.push(Brief {
            boot_at: crash.boot_at,
            at: crash.at,
            at_text: crash.at_text.clone(),
            kind: crash.kind.clone(),
            verdict: crash.verdict.clone(),
            trigger: crash.trigger.clone(),
            minutes: crash.minutes,
        });
        // A year of weekly crashes is still a handful of lines; anything past this is a machine
        while history.len() > 20 {
            history.remove(0);
        }
        save_history(&history);
        log::line(format!(
            // i18n-ok: model/log-facing text.
            "crash: {} {} (werdykt {}, {} min wcześniej{})",
            crash.at_text,
            crash.kind,
            crash.verdict,
            crash.minutes,
            if crash.trigger.is_empty() {
                String::new()
            } else {
                format!(", {}", crash.trigger)
            }
        ));
        for line in &crash.evidence {
            log::line(format!("crash: {line}"));
        }
    }
    crash.total = history.len() as u32;
    crash.repeats = history
        .iter()
        .filter(|b| b.verdict == crash.verdict)
        .count() as u32;
    if first_time {
        save_last(&crash);
    }
    Some(crash)
}

/// The crash as the text the model reads: the verdict first, then what it was read from, then the
pub fn describe() -> String {
    let Some(crash) = record().or_else(last) else {
        // i18n-ok: model-facing text.
        return format!(
            // i18n-ok: model/log-facing text.
            "Brak śladów nieczystego zamknięcia: ten start jest po normalnym restarcie. Tryb zasilania teraz: {}.",
            power_mode()
        );
    };
    let mut out = String::new();
    // i18n-ok: model-facing text.
    // i18n-ok: model-facing.
    out.push_str(&format!(
        // i18n-ok: model/log-facing text.
        "Ostatni nieczysty start: {} → {} ({} s ciszy).\n",
        crash.at_text, crash.boot_text, crash.silence
    ));
    let verdict = match crash.verdict.as_str() {
        "powerMode" => format!(
            // i18n-ok: model-facing.
            "zawieszenie {} min po tym, jak {} przełączył tryb zasilania",
            crash.minutes, crash.trigger
        ),
        "powerSource" => format!(
            // i18n-ok: model-facing.
            "zawieszenie {} min po zmianie źródła zasilania",
            crash.minutes
        ),
        "sleep" => format!(
            // i18n-ok: model-facing.
            "zawieszenie {} min po przejściu snu/czuwania",
            crash.minutes
        ),
        "driver" => format!(
            // i18n-ok: model-facing.
            "zawieszenie {} min po instalacji sterownika ({})",
            crash.minutes, crash.trigger
        ),
        "throttle" => "procesor był duszony przez firmware przed zawieszeniem".to_string(),
        "hardware" => "sprzęt zgłosił błąd (WHEA) tuż przed zawieszeniem".to_string(),
        "forced" => "zasilanie odcięte przyciskiem, więc Windows nie zapisał przyczyny".to_string(),
        "bugcheck" => format!("BSOD {}", crash.bugcheck),
        // i18n-ok: model-facing.
        _ => "brak jednej poszlaki: nie ma zrzutu, nie ma błędu sprzętu, nic nie zmieniło się chwilę wcześniej".to_string(),
    };
    // i18n-ok: model-facing.
    out.push_str(&format!("Werdykt: {verdict}.\n"));
    // i18n-ok: model-facing.
    out.push_str(&format!(
        // i18n-ok: model/log-facing text.
        "Rodzaj: {} · zrzut: {} · WHEA: {}\n",
        crash.kind, crash.dump, crash.whea
    ));
    // i18n-ok: model-facing.
    out.push_str("Co z tego odczytano:\n");
    for line in &crash.evidence {
        out.push_str(&format!("- {line}\n"));
    }
    if crash.repeats >= 2 {
        // i18n-ok: model-facing.
        out.push_str(&format!(
            // i18n-ok: model/log-facing text.
            "Powtarzalność: {} z {} zapisanych crashów miało ten sam werdykt ({}) — to wzorzec, nie przypadek.\n",
            crash.repeats, crash.total, crash.verdict
        ));
    }
    out
}


#[cfg(test)]
mod dump_tests {
    use super::*;

    /// The lines `!analyze -v` actually printed for a 0x7e, cut down to the ones the parse reads.
    const SAMPLE: &str = "\
Bugcheck Analysis
SYSTEM_THREAD_EXCEPTION_NOT_HANDLED (7e)
Arguments:
Arg1: ffffffffc0000005, The exception code that was not handled
    Key  : AV.Type
    Value: Read
BUGCHECK_CODE:  7e
BUGCHECK_P1: ffffffffc0000005
EXCEPTION_CODE: (NTSTATUS) 0xc0000005 - Instrukcja w 0x%p odwo a a si  do pami ci
STACK_TEXT:
ffffb901`18477618 fffff803`c7ebcd11     : 00000000`00000000 ffffbb8d`ae73bdb0 : nt!HalpIommuIsInPermanentPassthrough+0xa
ffffb901`18477620 fffff803`c7ec7bc4     : ffffbb8d`ac85bd30 00000000`00000000 : nt!HalpIommuLeaveDmaDomain+0x69
ffffb901`18477670 fffff803`c81492a7     : 00000000`00000000 ffffcb80`5e3559c0 : nt!HalpIommuBlockDevice+0xd4

SYMBOL_NAME:  nt!HalpIommuIsInPermanentPassthrough+a
MODULE_NAME: nt
IMAGE_NAME:  ntkrnlmp.exe
IMAGE_VERSION:  10.0.26100.9818
FAILURE_BUCKET_ID:  AV_nt!HalpIommuIsInPermanentPassthrough
PROCESS_NAME:  System
STACK_COMMAND:  .cxr 0xffffb90118476bf0 ; kb
";

    #[test]
    fn the_report_names_the_bugcheck_the_module_and_the_stack() {
        let text = describe_text(SAMPLE);
        assert!(text.contains("BUGCHECK: 0x7e SYSTEM_THREAD_EXCEPTION_NOT_HANDLED"));
        assert!(text.contains("błąd dostępu (Read)"));
        assert!(text.contains("WINNY: nt!HalpIommuIsInPermanentPassthrough+a w ntkrnlmp.exe 10.0.26100.9818 (proces System)"));
        assert!(text.contains("BUCKET: AV_nt!HalpIommuIsInPermanentPassthrough"));
        assert!(text.contains("1. nt!HalpIommuIsInPermanentPassthrough+0xa"));
        assert!(text.contains("2. nt!HalpIommuLeaveDmaDomain+0x69"));
        assert!(text.contains("3. nt!HalpIommuBlockDevice+0xd4"));
        assert!(text.contains("STACK_COMMAND: .cxr 0xffffb90118476bf0 ; kb"));
        // The frames stop at the blank line instead of swallowing the keys that follow it.
        assert!(!text.contains("IMAGE_NAME"));
    }

    #[test]
    fn a_log_without_a_bugcheck_is_not_mistaken_for_one() {
        let text = "Loading Dump File [C:/Users/x/d.dmp]\nThe system cannot find the file specified.\n";
        assert!(!Analysis::parse(text).known());
    }

    #[test]
    fn the_debugger_aliases_are_looked_for_where_the_store_puts_them() {
        // The name is what a Store install exposes; `cdb.exe` exists only in an SDK install.
        let found = debugger();
        if let Some(path) = found {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            assert!(
                name.starts_with("cdb"),
                "unexpected debugger chosen: {}",
                path.display()
            );
        }
    }
}
