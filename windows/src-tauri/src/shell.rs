// Running commands on this machine, for the chat's terminal tools.

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
pub const MAX_TIMEOUT: u64 = 300;

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

pub fn launch(what: &str) -> Result<String, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;

    let what = what.trim();
    if what.is_empty() {
        return Err("Nothing to open".to_string());
    }
    let target = if Path::new(what).exists() {
        PathBuf::from(what)
    } else {
        find_app(what).ok_or_else(|| format!("Could not find \"{what}\" on the desktop or in the Start menu."))?
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

/// Shortcuts beat executables, and the desktop beats the Start menu.
fn find_app(name: &str) -> Option<PathBuf> {
    pick_app(&search_dirs(), name)
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

pub fn kill_orphans() {
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

/// Runs a command and waits for it, up to `timeout_s`.
pub fn run(cmd: &str, cwd: Option<&str>, timeout_s: Option<u64>) -> Result<Output, String> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Err("Empty command".to_string());
    }
    let timeout = timeout_s.unwrap_or(DEFAULT_TIMEOUT).clamp(1, MAX_TIMEOUT);
    let started = Instant::now();

    let mut child = command_for(cmd, cwd)?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start the command: {e}"))?;

    // Read both pipes on their own threads: a child that fills a pipe while we
    // wait on the other one would deadlock.
    let out_handle = child.stdout.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            buf
        })
    });
    let err_handle = child.stderr.take().map(|mut s| {
        std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            buf
        })
    });

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

    let stdout = out_handle.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();

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
        let id = spawn("echo job-output && exit 0", None).expect("spawn");
        let line = wait_for_job(id, |line| !line.contains("running"));
        assert!(line.contains(&format!("#{id}")), "jobs said {line}");
        assert!(line.contains("finished (exit 0)"), "jobs said {line}");
        let text = output(id, 4000).expect("output");
        assert!(text.contains("job-output"), "output was {text}");
    }

    #[test]
    fn a_long_job_can_be_killed() {
        let id = spawn("ping -n 10 127.0.0.1", None).expect("spawn");
        std::thread::sleep(Duration::from_millis(300));
        let line = || {
            jobs()
                .lines()
                .find(|l| l.starts_with(&format!("#{id} ")))
                .unwrap_or("")
                .to_string()
        };
        assert!(line().contains("running"), "job line was {:?}", line());
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
    fn launching_something_that_is_not_there_says_so() {
        let err = launch("no-such-program-xyz").unwrap_err();
        assert!(err.contains("Could not find"), "err was {err}");
        assert!(launch("   ").is_err());
    }

    #[test]
    fn kills_a_job_left_over_from_a_crash() {
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
}
