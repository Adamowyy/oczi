// Running commands for the chat's terminal tools. Off unless the user enabled
// it; every command is written to the app log. Long jobs go to `spawn`.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Output handed back to the model, per stream. More than this is noise.
pub const MAX_OUTPUT: usize = 8_000;
pub const DEFAULT_TIMEOUT: u64 = 30;
/// The longest a foreground command may run. The user is watching the island while it waits,
/// so anything slower belongs in `terminal_job`; this cap is what makes that true.
pub const MAX_TIMEOUT: u64 = 60;

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub ms: u128,
}

impl Output {
    /// One block of text, the way the model should read it back.
    pub fn format(&self) -> String {
        let mut s = String::new();
        match self.code {
            Some(0) if !self.timed_out => s.push_str("exit 0"),
            Some(code) => s.push_str(&format!("exit {code}")),
            None => s.push_str("process killed"),
        }
        s.push_str(&format!(", {} ms", self.ms));
        if self.timed_out {
            s.push_str(", timed out and killed");
        }
        if !self.stdout.trim().is_empty() {
            s.push_str("\n\nstdout:\n");
            s.push_str(&clip(&self.stdout));
        }
        if !self.stderr.trim().is_empty() {
            s.push_str("\n\nstderr:\n");
            s.push_str(&clip(&self.stderr));
        }
        if self.stdout.trim().is_empty() && self.stderr.trim().is_empty() {
            s.push_str("\n(no output)");
        }
        s
    }
}

/// Keeps the tail, which is where the error is.
fn clip(text: &str) -> String {
    let text = text.trim_end();
    if text.chars().count() <= MAX_OUTPUT {
        return text.to_string();
    }
    let tail: String = text.chars().skip(text.chars().count() - MAX_OUTPUT).collect();
    format!("…(output clipped)\n{tail}")
}

/// `cmd.exe`, no console window, optional working directory.
fn command_for(cmd: &str, cwd: Option<&str>) -> Result<Command, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let mut c = Command::new("cmd");
    c.arg("/C");
    // Raw arg keeps the command exactly as written: a normal argument gets
    // quoted for CreateProcess and cmd then rejects quoted paths as bad syntax.
    #[cfg(windows)]
    c.raw_arg(cmd);
    #[cfg(not(windows))]
    c.arg(cmd);
    c.stdin(Stdio::null());
    if let Some(dir) = cwd {
        let dir = dir.trim();
        if !dir.is_empty() {
            if !Path::new(dir).is_dir() {
                return Err(format!("Working directory does not exist: {dir}"));
            }
            c.current_dir(dir);
        }
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(c)
}

/// Opens a program or file like a double-click. `what` is a path, or a name
/// looked up on the desktop, the public desktop and the Start menu.
pub fn launch(what: &str) -> Result<String, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let what = what.trim();
    if what.is_empty() {
        return Err("Nothing to open".to_string());
    }
    let target = if Path::new(what).exists() {
        PathBuf::from(what)
    } else if let Some(path) = find_app_tolerant(what) {
        path
    } else if let Some(game) = crate::steam::installed(what) {
        // A game is not a file to run: Steam owns it, and the app id in its own URL is the
        // only handle on it. Checked before giving up, never by walking the disk — a model
        // left to search for a game's executable spends minutes on it (see `steam.rs`).
        return crate::steam::open(&game);
    } else {
        return Err(format!(
            "Could not find \"{what}\" on the desktop, in the Start menu or in the Steam library."
        ));
    };

    let dir = target.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut command = Command::new("cmd");
    command.arg("/C");
    #[cfg(windows)]
    command.raw_arg(format!("start \"\" \"{}\"", target.display()));
    #[cfg(not(windows))]
    command.arg(format!("start \"\" \"{}\"", target.display()));
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    if dir.is_dir() {
        command.current_dir(&dir);
    }
    command
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", target.display()))?;
    Ok(format!("Started {}.", target.display()))
}

/// Opens an address in the user's default browser. The same route the island's own link
/// buttons take (`url.dll,FileProtocolHandler`), so a page the model opens and a link the
/// user clicks on a card behave identically.
pub fn open_url(url: &str) -> Result<String, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("An address must start with http:// or https://".to_string());
    }
    let mut command = Command::new("rundll32.exe");
    command.args(["url.dll,FileProtocolHandler", url]);
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map_err(|e| format!("Could not open {url}: {e}"))?;
    Ok(format!("Opened {url}."))
}

// ── Closing a program ─────────────────────────────────────────────────────────

/// Windows' own processes, security software, and our own window. A wrong program name is
/// a failed command; one of these is the shell, the session or the desktop itself, and
/// closing it takes the taskbar — or Oczi's island — with it. Oczi draws its window with
/// WebView2, so `msedgewebview2.exe` *is* Oczi and is refused here as well as in the
/// prompt that tells the model so.
const PROTECTED_PROCESSES: &[&str] = &[
    "system",
    "registry",
    "idle",
    "memory compression",
    "secure system",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "lsaiso",
    "dwm",
    "fontdrvhost",
    "svchost",
    "explorer",
    "sihost",
    "shellexperiencehost",
    "taskhostw",
    "audiodg",
    "spoolsv",
    "msmpeng",
    "nissrv",
    "securityhealthservice",
    "oczi",
    "msedgewebview2",
];

/// A process name as `taskkill /IM` wants it: bare, with no wildcard, no switch, no path.
/// This is the one argument a model gets wrong in the worst way — `*` closes the machine —
/// so it is normalised and refused here instead of being handed to taskkill.
pub fn image_name(name: &str) -> Result<String, String> {
    let raw = name.trim().trim_matches('"').trim();
    if raw.is_empty() {
        return Err("No process name given".to_string());
    }
    // A path is accepted for its file name alone (C:\Program Files\Steam\steam.exe), but
    // only when it really is one. "steam /F" is not a path, and taking the text after the
    // slash would turn that mistake into `f.exe` — a close that silently does nothing
    // instead of saying the name was wrong.
    let looks_like_path = raw.contains('\\') || raw.contains(':');
    let base = if looks_like_path {
        raw.rsplit(['\\', '/']).next().unwrap_or(raw).trim()
    } else {
        raw
    };
    let lower = base.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower).to_string();
    let plain = stem
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '(' | ')'));
    if stem.is_empty() || stem.len() > 64 || !plain {
        return Err(format!("\"{name}\" is not a process name"));
    }
    if PROTECTED_PROCESSES.contains(&stem.as_str()) {
        return Err(format!(
            "\"{stem}\" is part of Windows or of Oczi itself — it is not closed from here"
        ));
    }
    Ok(format!("{stem}.exe"))
}

/// `tasklist`'s CSV, as the pids of one image. "Not running" and "the process list could
/// not be read" are different answers and only one of them is a success, so the caller
/// gets an error rather than an empty list when the read itself failed.
fn pids_in(csv: &str, image: &str) -> Vec<u32> {
    let wanted = image.to_ascii_lowercase();
    let mut out = Vec::new();
    for line in csv.lines() {
        let line = line.trim();
        if !line.starts_with('"') {
            continue;
        }
        let mut fields = line.split("\",\"");
        let name = fields
            .next()
            .unwrap_or("")
            .trim_matches('"')
            .to_ascii_lowercase();
        if name != wanted {
            continue;
        }
        if let Some(pid) = fields
            .next()
            .and_then(|f| f.trim_matches('"').trim().parse::<u32>().ok())
        {
            out.push(pid);
        }
    }
    out
}

fn running_pids(image: &str) -> Result<Vec<u32>, String> {
    let cmd = format!("tasklist /FI \"IMAGENAME eq {image}\" /FO CSV /NH");
    let out = run(&cmd, None, Some(15))?;
    if out.code != Some(0) {
        return Err(format!("Could not read the process list: {}", out.format()));
    }
    Ok(pids_in(&out.stdout, image))
}

/// Waits for a close request to be answered, up to three seconds. An application needs a
/// moment to save and go, and reporting "still running" after 10 ms is what makes a model
/// force the close of something that was closing anyway.
fn wait_until_gone(image: &str) -> Vec<u32> {
    let mut last: Vec<u32> = Vec::new();
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(250));
        match running_pids(image) {
            Ok(pids) => {
                if pids.is_empty() {
                    return pids;
                }
                last = pids;
            }
            // An unreadable process list is not an empty one: keep the last answer we
            // had rather than reporting a close that did not happen.
            Err(_) => {}
        }
    }
    last
}

/// Closes a program by its process name. It is *asked* to close first — that is what lets
/// Discord and Steam save their state — and only forced when it did not answer, which is
/// what the user means by "kill it". Nothing running is a success, not an error.
pub fn kill_process(name: &str) -> Result<String, String> {
    let image = image_name(name)?;
    let before = running_pids(&image)?;
    if before.is_empty() {
        return Ok(format!("{image} was not running."));
    }
    run(&format!("taskkill /IM {image} /T"), None, Some(20))?;
    if wait_until_gone(&image).is_empty() {
        return Ok(format!(
            "Closed {image} — {} process(es), and it closed itself.",
            before.len()
        ));
    }
    // It did not answer a close request: a game with no window, or a program that is
    // stuck. Now it is killed, and the answer says so.
    crate::log::line(format!("taskkill had to force {image}"));
    run(&format!("taskkill /IM {image} /T /F"), None, Some(20))?;
    let left = wait_until_gone(&image);
    if left.is_empty() {
        return Ok(format!(
            "Killed {image} — {} process(es), forced after it did not close on its own.",
            before.len()
        ));
    }
    Ok(format!(
        "{image}: {} of {} process(es) are still running.",
        left.len(),
        before.len()
    ))
}

/// Shortcuts beat executables, and the desktop beats the Start menu.
fn find_app(name: &str) -> Option<PathBuf> {
    pick_app(&search_dirs(), name)
}

/// The name as it arrives from the tools, which is not always the name on disk: a dictated one
/// carries the Polish case ending it was spoken with. The plain name is tried first and the
/// stripped forms only when nothing matched at all, so a program whose real name ends the same
/// way is untouched and the only cost of a miss is a second walk.
fn find_app_tolerant(name: &str) -> Option<PathBuf> {
    if let Some(path) = find_app(name) {
        return Some(path);
    }
    for candidate in spoken_stems(name) {
        if let Some(path) = find_app(&candidate) {
            return Some(path);
        }
    }
    None
}

/// What a spoken name might be on disk: without the case ending that only the speaker added.
/// Polish borrows masculine names with `-a` in the accusative, so that is the ending worth
/// trying; a stem under three letters is not a name and is dropped. `steam` matches the same
/// way.
pub(crate) fn spoken_stems(name: &str) -> Vec<String> {
    let lowered = name.trim().to_lowercase();
    let mut stems = Vec::new();
    // i18n-ok: Polish case endings — data to match against, not UI copy.
    for ending in ["a", "ę", "u"] {
        if let Some(stem) = lowered.strip_suffix(ending) {
            if stem.chars().count() >= 3 {
                stems.push(stem.to_string());
            }
        }
    }
    stems
}

fn pick_app(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let needle = name.to_lowercase();
    let mut best: Option<(u8, PathBuf)> = None;
    for (rank, dir) in dirs.iter().enumerate() {
        for path in walk(dir, 3) {
            let Some(file) = path.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            let lower = file.to_lowercase();
            let stem = lower
                .strip_suffix(".exe")
                .or_else(|| lower.strip_suffix(".lnk"))
                .unwrap_or(&lower);
            if !stem.contains(&needle) {
                continue;
            }
            // An exact name wins, then a shortcut, then the first place found.
            let score = if stem == needle { 0 } else { 1 } + if lower.ends_with(".lnk") { 0 } else { 1 };
            let score = score + rank as u8;
            if best.as_ref().map(|(b, _)| score < *b).unwrap_or(true) {
                best = Some((score, path.clone()));
            }
        }
    }
    best.map(|(_, path)| path)
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Desktop"));
        dirs.push(home.join("OneDrive").join("Desktop"));
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        dirs.push(PathBuf::from(public).join("Desktop"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

fn walk(dir: &Path, depth: u8) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if path.is_dir() {
            if depth > 1 && !name.starts_with('.') {
                out.extend(walk(&path, depth - 1));
            }
            continue;
        }
        if name.ends_with(".exe") || name.ends_with(".lnk") || name.ends_with(".bat") {
            out.push(path);
        }
    }
    out
}

/// Kills the background jobs and clears their logs. Called when the app quits,
/// so nothing of ours keeps running behind the user's back.
pub fn shutdown() {
    let mut registry = JOBS.lock().unwrap();
    if let Some(jobs) = registry.as_mut() {
        for job in jobs.values_mut() {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
        jobs.clear();
    }
    drop(registry);
    clear_logs();
}

/// Job logs from an earlier run are of no use to anyone. Called at startup.
pub fn clear_logs() {
    if let Ok(entries) = fs::read_dir(jobs_dir()) {
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }
}

// Kills commands a previous run left behind when the app was killed, not quit.
pub fn kill_orphans() {
    let tracked: Vec<u32> = {
        let mut registry = JOBS.lock().unwrap();
        match registry.as_mut() {
            Some(jobs) => jobs.values_mut().map(|job| job.child.id()).collect(),
            None => Vec::new(),
        }
    };
    let Ok(entries) = fs::read_dir(jobs_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                if tracked.contains(&pid) {
                    continue;
                }
                if is_our_shell(pid) {
                    let mut kill = Command::new("taskkill");
                    kill.args(["/PID", &pid.to_string(), "/T", "/F"]);
                    kill.stdin(Stdio::null());
                    kill.stdout(Stdio::null());
                    kill.stderr(Stdio::null());
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                        kill.creation_flags(CREATE_NO_WINDOW);
                    }
                    let _ = kill.status();
                }
            }
        }
        let _ = fs::remove_file(&path);
    }
}

fn is_our_shell(pid: u32) -> bool {
    let mut list = Command::new("tasklist");
    list.args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]);
    list.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        list.creation_flags(CREATE_NO_WINDOW);
    }
    list.output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .to_lowercase()
                .contains("cmd.exe")
        })
        .unwrap_or(false)
}

/// Launchers hand the work to a new process and exit at once — `start "" chrome.exe` is how
/// the model opens a browser. That new process inherits our pipes, and holds them open for as
/// long as it runs, so reading until the pipe closes waits for the *browser to be closed*
/// rather than for the command to finish. Nothing useful can come back from them either, so
/// they are run with no pipes at all.
fn detaches(cmd: &str) -> bool {
    let lower = cmd.trim_start().to_ascii_lowercase();
    lower == "start"
        || lower.starts_with("start ")
        || lower.starts_with("start\"")
        || lower.starts_with("explorer ")
        || lower.starts_with("cmd /c start")
        || lower.starts_with("cmd.exe /c start")
}

/// Runs a command and waits for it, up to `timeout_s`.
pub fn run(cmd: &str, cwd: Option<&str>, timeout_s: Option<u64>) -> Result<Output, String> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Err("Empty command".to_string());
    }
    let timeout = timeout_s.unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT);
    let started = Instant::now();
    let detached = detaches(cmd);

    let mut command = command_for(cmd, cwd)?;
    if detached {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    } else {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start the command: {e}"))?;

    // Read both pipes on their own threads: a child that fills a pipe while we
    // wait on the other one would deadlock.
    let mut out_rx = None;
    let mut err_rx = None;
    if !detached {
        if let Some(mut stream) = child.stdout.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = stream.read_to_string(&mut buf);
                let _ = tx.send(buf);
            });
            out_rx = Some(rx);
        }
        if let Some(mut stream) = child.stderr.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = stream.read_to_string(&mut buf);
                let _ = tx.send(buf);
            });
            err_rx = Some(rx);
        }
    }

    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut timed_out = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("Could not wait for the command: {e}")),
        }
    };

    // Bounded on purpose: a grandchild can hold the pipe open long after the command itself is
    // gone (a script that starts a daemon, a tool that spawns a helper). Waiting for the pipe
    // to close in that case means waiting for that program to exit — which is how "open the
    // browser" left Oczi thinking for ever. Whatever arrived by now is what we report.
    let drain = |rx: Option<std::sync::mpsc::Receiver<String>>| {
        rx.and_then(|rx| rx.recv_timeout(Duration::from_millis(1000)).ok())
            .unwrap_or_default()
    };
    let stdout = drain(out_rx);
    let stderr = drain(err_rx);

    Ok(Output {
        code,
        stdout,
        stderr,
        timed_out,
        ms: started.elapsed().as_millis(),
    })
}

// ── Background jobs ───────────────────────────────────────────────────────────

pub struct Job {
    command: String,
    child: Child,
}

static JOBS: Mutex<Option<HashMap<u32, Job>>> = Mutex::new(None);
static NEXT_ID: Mutex<u32> = Mutex::new(1);

/// Where background jobs write. Inside the app's own local folder in production,
/// a temp folder when the module is compiled on its own for tests.
fn jobs_dir() -> PathBuf {
    let base = std::env::var_os("OCZI_JOBS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("LOCALAPPDATA")
                .map(|p| PathBuf::from(p).join("Oczi").join("jobs"))
                .unwrap_or_else(std::env::temp_dir)
        });
    let _ = fs::create_dir_all(&base);
    base
}

fn log_path(id: u32) -> PathBuf {
    jobs_dir().join(format!("{id}.log"))
}

/// The pid of a job, written next to its log. It is what makes it possible to
/// clean up after the app itself was killed instead of quitting.
fn pid_path(id: u32) -> PathBuf {
    jobs_dir().join(format!("{id}.pid"))
}

/// Starts a command in the background; its output goes to a log file.
pub fn spawn(cmd: &str, cwd: Option<&str>) -> Result<u32, String> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Err("Empty command".to_string());
    }
    let mut registry = JOBS.lock().unwrap();
    let jobs = registry.get_or_insert_with(HashMap::new);
    let id = {
        let mut next = NEXT_ID.lock().unwrap();
        let id = *next;
        *next += 1;
        id
    };

    let log = log_path(id);
    let stdout = File::create(&log).map_err(|e| format!("Could not open the job log: {e}"))?;
    let stderr = OpenOptions::new()
        .append(true)
        .open(&log)
        .map_err(|e| format!("Could not open the job log: {e}"))?;

    let child = command_for(cmd, cwd)?
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| format!("Could not start the command: {e}"))?;

    let _ = fs::write(pid_path(id), child.id().to_string());
    jobs.insert(
        id,
        Job {
            command: cmd.to_string(),
            child,
        },
    );
    Ok(id)
}

/// Runs a shell command in a background thread and reports when it ends.
fn status_of(job: &mut Job) -> String {
    match job.child.try_wait() {
        Ok(Some(status)) => match status.code() {
            Some(code) => format!("finished (exit {code})"),
            None => "finished (killed)".to_string(),
        },
        Ok(None) => "running".to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// One line per job, newest last.
pub fn jobs() -> String {
    let mut registry = JOBS.lock().unwrap();
    let jobs = match registry.as_mut() {
        Some(jobs) if !jobs.is_empty() => jobs,
        _ => return "No background jobs.".to_string(),
    };
    let mut ids: Vec<u32> = jobs.keys().copied().collect();
    ids.sort();
    let mut lines = Vec::new();
    for id in ids {
        let job = jobs.get_mut(&id).unwrap();
        let status = status_of(job);
        lines.push(format!("#{id} [{status}] {}", job.command));
    }
    lines.join("\n")
}

/// The tail of a job's log, what the model reads to see how it is going.
pub fn output(id: u32, max: usize) -> Result<String, String> {
    let status = {
        let mut registry = JOBS.lock().unwrap();
        match registry.as_mut().and_then(|jobs| jobs.get_mut(&id)) {
            Some(job) => status_of(job),
            None => return Err(format!("No job #{id}")),
        }
    };
    let path = log_path(id);
    let text = fs::read_to_string(&path).unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(format!("#{id} [{status}]: no output yet"));
    }
    let tail = clip_tail(&text, max);
    Ok(format!("#{id} [{status}]: {tail}"))
}

fn clip_tail(text: &str, max: usize) -> String {
    let text = text.trim_end();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let tail: String = text.chars().skip(text.chars().count() - max).collect();
    format!("…(earlier output clipped)\n{tail}")
}

/// Kills a job. False when it had already finished.
pub fn kill(id: u32) -> Result<bool, String> {
    let mut registry = JOBS.lock().unwrap();
    let jobs = registry.as_mut().ok_or_else(|| format!("No job #{id}"))?;
    let job = jobs.get_mut(&id).ok_or_else(|| format!("No job #{id}"))?;
    match job.child.try_wait() {
        Ok(Some(_)) => Ok(false),
        _ => Ok(job.child.kill().is_ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `JOBS` and the job folder are shared by the whole test binary, so the
    /// tests that touch them take turns.
    static JOBS_TESTS: Mutex<()> = Mutex::new(());

    fn jobs_tests() -> std::sync::MutexGuard<'static, ()> {
        JOBS_TESTS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn runs_a_command_and_reads_stdout() {
        let out = run("echo hello", None, None).expect("run");
        assert_eq!(out.code, Some(0));
        assert!(out.stdout.contains("hello"), "stdout was {:?}", out.stdout);
        assert!(!out.timed_out);
    }

    #[test]
    fn reports_a_nonzero_exit() {
        let out = run("exit 3", None, None).expect("run");
        assert_eq!(out.code, Some(3));
    }

    #[test]
    fn captures_stderr() {
        let out = run("echo problem 1>&2", None, None).expect("run");
        assert!(out.stderr.contains("problem"), "stderr was {:?}", out.stderr);
    }

    #[test]
    fn kills_a_command_that_overruns() {
        let out = run("ping -n 6 127.0.0.1", None, Some(1)).expect("run");
        assert!(out.timed_out, "should have timed out");
        assert!(out.format().contains("timed out"));
    }

    #[test]
    fn refuses_an_empty_command() {
        assert!(run("   ", None, None).is_err());
    }

    #[test]
    fn refuses_a_missing_working_directory() {
        assert!(run("echo hi", Some("C:/definitely/not/here"), None).is_err());
    }

    #[test]
    fn runs_in_a_working_directory() {
        let out = run("cd", Some("C:/Windows"), None).expect("run");
        assert!(out.stdout.to_lowercase().contains("windows"));
    }

    #[test]
    fn a_job_reports_its_output() {
        let _guard = jobs_tests();
        let id = spawn("echo job-output && exit 0", None).expect("spawn");
        let line = wait_for_job(id, |line| !line.contains("running"));
        assert!(line.contains(&format!("#{id}")), "jobs said {line}");
        assert!(line.contains("finished (exit 0)"), "jobs said {line}");
        let text = output(id, 4000).expect("output");
        assert!(text.contains("job-output"), "output was {text}");
    }

    #[test]
    fn a_long_job_can_be_killed() {
        let _guard = jobs_tests();
        let id = spawn("ping -n 10 127.0.0.1", None).expect("spawn");
        let line = || {
            jobs()
                .lines()
                .find(|l| l.starts_with(&format!("#{id} ")))
                .unwrap_or("")
                .to_string()
        };
        // Wait for `running` instead of sleeping: a loaded runner may not have
        // the job registered yet.
        let running = wait_for_job(id, |l| l.contains("running"));
        assert!(running.contains("running"), "job line was {:?}", line());
        assert!(kill(id).expect("kill"));
        let after = wait_for_job(id, |line| !line.contains("running"));
        assert!(!after.contains("running"), "job line was {:?}", after);
    }

    #[test]
    fn keeps_the_quotes_in_a_command() {
        // The whole reason `raw_arg` is used: a quoted path must arrive intact.
        let out = run(
            r#"if exist "C:\Windows\System32\cmd.exe" (echo FOUND) else (echo MISSING)"#,
            None,
            None,
        )
        .expect("run");
        assert!(out.stdout.contains("FOUND"), "stdout was {:?}", out.stdout);
    }

    #[test]
    fn lists_a_quoted_directory_with_a_space_in_its_name() {
        let out = run(r#"dir "C:\Program Files" /b"#, None, None).expect("run");
        assert_eq!(out.code, Some(0), "stderr was {:?}", out.stderr);
        assert!(
            out.stdout.to_lowercase().contains("windows"),
            "stdout was {:?}",
            out.stdout
        );
    }

    #[test]
    fn a_command_that_needs_a_double_quoted_subcommand_still_works() {
        let out = run(
            r#"powershell -NoProfile -Command "Write-Output 'quoted ok'""#,
            None,
            None,
        )
        .expect("run");
        assert!(out.stdout.contains("quoted ok"), "stdout was {:?}", out.stdout);
    }

    #[test]
    fn finds_a_program_by_name_before_its_longer_relative() {
        let dir = std::env::temp_dir().join("oczi-shell-test");
        let _ = fs::create_dir_all(&dir);
        let exact = dir.join("PZUpdater.exe");
        let other = dir.join("PZUpdater Helper.exe");
        for file in [&exact, &other] {
            File::create(file).expect("create");
        }
        let found = pick_app(&[dir.clone()], "pzupdater").expect("found");
        assert_eq!(found, exact);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_foreground_command_is_killed_at_its_deadline() {
        // The cap is what keeps a command that hunts a whole disk from holding the turn for
        // minutes: it has to come back, with `timed_out` set and the output it produced.
        let started = Instant::now();
        let out = run("ping -n 30 127.0.0.1", None, Some(1)).expect("run");
        assert!(out.timed_out, "the command was not killed at its deadline");
        assert_eq!(out.code, None, "a killed command has no exit code");
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "returned after {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_timeout_above_the_cap_is_clamped() {
        // Asking for ten minutes does not buy ten minutes: it is the clamp that bounds it.
        assert_eq!(Some(600u64).unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT), 60);
    }

    #[test]
    fn finds_a_program_spelled_the_way_it_was_spoken() {
        assert_eq!(spoken_stems("Steama"), vec!["steam"]);
        assert_eq!(spoken_stems("Discorda"), vec!["discord"]);
        assert_eq!(spoken_stems("Steam"), Vec::<String>::new());
        // Too short to be a name: nothing to look for.
        assert_eq!(spoken_stems("ma"), Vec::<String>::new());
        // A real name that happens to end the same way is only a *second* candidate, so the
        // program itself still wins — see `find_app_tolerant`.
        assert_eq!(spoken_stems("Opera"), vec!["oper"]);
    }

    #[test]
    fn launching_something_that_is_not_there_says_so() {
        let err = launch("no-such-program-xyz").unwrap_err();
        assert!(err.contains("Could not find"), "err was {err}");
        assert!(launch("   ").is_err());
    }

    #[test]
    fn kills_a_job_left_over_from_a_crash() {
        let _guard = jobs_tests();
        // Started outside the registry, exactly as if the app had been killed.
        let mut orphan = command_for("ping -n 30 127.0.0.1", None)
            .expect("command")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let id = 900;
        fs::write(pid_path(id), orphan.id().to_string()).expect("pid file");

        kill_orphans();
        std::thread::sleep(Duration::from_millis(400));
        assert!(
            orphan.try_wait().expect("wait").is_some(),
            "the orphan should have been killed"
        );
        assert!(!pid_path(id).exists(), "the pid file should be gone");
    }

    /// The one line about a job, once `done` says so, or after five seconds.
    fn wait_for_job(id: u32, done: impl Fn(&str) -> bool) -> String {
        let prefix = format!("#{id} ");
        let mut line = String::new();
        for _ in 0..100 {
            line = jobs()
                .lines()
                .find(|l| l.starts_with(&prefix))
                .unwrap_or("")
                .to_string();
            if done(&line) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        line
    }

    #[test]
    fn clipping_keeps_the_tail() {
        let long = "x".repeat(MAX_OUTPUT + 50) + "THE-END";
        let clipped = clip(&long);
        assert!(clipped.contains("THE-END"));
        assert!(clipped.contains("clipped"));
    }

    #[test]
    fn a_process_name_is_a_name_and_nothing_else() {
        assert_eq!(image_name("steam").unwrap(), "steam.exe");
        assert_eq!(image_name(" Discord ").unwrap(), "discord.exe");
        // A path is accepted for its file name, which is how a model quotes a game.
        assert_eq!(
            image_name("C:\\Program Files (x86)\\Steam\\steam.exe").unwrap(),
            "steam.exe"
        );

        let long = "x".repeat(70);
        // A wildcard closes half the machine, a switch and a quote are not a name, and a
        // drive letter on its own is not a program.
        for bad in ["*", "", "   ", "steam /F", "steam.exe\" /IM discord.exe", "C:", long.as_str()] {
            assert!(image_name(bad).is_err(), "{bad:?} must not be a process name");
        }

        // Windows, its shell, the security software, and Oczi's own window.
        for held in [
            "explorer",
            "svchost",
            "lsass",
            "winlogon",
            "dwm",
            "msmpeng",
            "oczi",
            "msedgewebview2",
        ] {
            assert!(image_name(held).is_err(), "{held} must not be closable");
        }
    }

    #[test]
    fn reads_pids_out_of_the_process_list() {
        let csv = "\"steam.exe\",\"1234\",\"Console\",\"1\",\"300,000 K\"\r\n\
                   \"steamwebhelper.exe\",\"5678\",\"Console\",\"1\",\"10,000 K\"\r\n";
        assert_eq!(pids_in(csv, "steam.exe"), vec![1234]);
        assert_eq!(pids_in(csv, "Steam.exe"), vec![1234], "the filter is case-blind");
        assert_eq!(pids_in(csv, "discord.exe"), Vec::<u32>::new());
        // tasklist says so in words when nothing matches: no process, and no crash.
        let none = "INFO: No tasks are running which match the specified criteria.\r\n";
        assert_eq!(pids_in(none, "steam.exe"), Vec::<u32>::new());
        // A row whose pid cannot be read is dropped rather than guessed at.
        assert_eq!(pids_in("\"steam.exe\",\"oops\"\r\n", "steam.exe"), Vec::<u32>::new());
    }

    #[test]
    fn only_an_http_address_is_opened() {
        // Anything else would be a way to run a file or a script through a "link".
        for bad in ["", "   ", "file:///C:/Windows/System32/calc.exe", "javascript:alert(1)", "C:\\x.html"] {
            assert!(open_url(bad).is_err(), "{bad:?} must not be opened");
        }
    }
}
