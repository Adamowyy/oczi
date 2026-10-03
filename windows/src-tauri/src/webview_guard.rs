
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::webview::PlatformWebview;
use tauri::{AppHandle, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2ProcessFailedEventArgs, ICoreWebView2ProcessFailedEventArgs3,
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
    COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_REASON,
    COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
    COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED, COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED,
    COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE,
};
use webview2_com::{take_pwstr, ProcessFailedEventHandler};
use windows_core_062::{Interface, PWSTR};

use crate::{island, log, settings};

/// The island is the window the user is looking at; the other two share its
/// environment and go blank together with it.
const WATCHED: [&str; 3] = [island::WINDOW_LABEL, "settings", "snip"];

/// How often the app may restart itself before it stops trying, and over how
/// long. Three failures in ten minutes mean the runtime is broken rather than
/// unlucky, and a fourth restart would only flicker the screen again.
const LOOP_WINDOW_SECS: u64 = 600;
const LOOP_LIMIT: usize = 3;

/// Guards against one failure being reported three times: every webview in the
/// environment raises its own ProcessFailed for the same dead browser process.
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// Attach the guard to every webview. Called once, after the windows exist.
pub fn watch(app: &AppHandle) {
    for label in WATCHED {
        attach(app, label);
    }
}

fn attach(app: &AppHandle, label: &str) {
    let Some(window) = app.get_webview_window(label) else {
        log::line(format!("webview  {label}: no window, not watched"));
        return;
    };
    let owner = app.clone();
    let name = label.to_string();
    // A second, owned copy: the closure below has to be 'static, and `label` is a
    // borrow of the caller's string.
    let in_closure = name.clone();
    // Runs on the main thread, inside the COM apartment the webview was created in.
    let result = window.with_webview(move |platform| {
        let controller = platform.controller();
        let core = match unsafe { controller.CoreWebView2() } {
            Ok(core) => core,
            Err(err) => {
                log::line(format!("webview  {in_closure}: not watched — {err}"));
                return;
            }
        };
        if name == island::WINDOW_LABEL {
            log::line(format!(
                "webview  {name}: runtime {}",
                runtime_version(&platform)
            ));
        }

        let handler = ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
            if let Some(args) = args {
                failed(&owner, &name, &args);
            }
            Ok(())
        }));

        let mut token = 0i64;
        match unsafe { core.add_ProcessFailed(&handler, &mut token) } {
            Ok(()) => std::mem::forget(handler),
            Err(err) => log::line(format!("webview  {in_closure}: not watched — {err}")),
        }
    });
    if let Err(err) = result {
        log::line(format!("webview  {label}: not watched — {err}"));
    }
}

/// The version of the WebView2 runtime serving this app: worth a log line,
/// because a crash on a runtime that arrived minutes earlier is a different
/// story from a crash on one that has been in place for months.
fn runtime_version(platform: &PlatformWebview) -> String {
    let environment = platform.environment();
    let mut version = PWSTR::null();
    match unsafe { environment.BrowserVersionString(&mut version) } {
        Ok(()) => take_pwstr(version),
        Err(_) => "unknown".to_string(),
    }
}

/// One ProcessFailed notification: record it, then decide whether the island can
/// carry on as it is.
fn failed(app: &AppHandle, label: &str, args: &ICoreWebView2ProcessFailedEventArgs) {
    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
    let _ = unsafe { args.ProcessFailedKind(&mut kind) };

    // The reason and the exit code arrived with the second revision of the args
    // interface, the failed module's name with the third; on a runtime too old
    // for them the cast fails and the line simply carries less.
    let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
    let mut exit_code = 0i32;
    let mut module = String::new();
    if let Ok(details) = args.cast::<ICoreWebView2ProcessFailedEventArgs3>() {
        let _ = unsafe { details.Reason(&mut reason) };
        let _ = unsafe { details.ExitCode(&mut exit_code) };
        let mut path = PWSTR::null();
        if unsafe { details.FailureSourceModulePath(&mut path) }.is_ok() {
            module = take_pwstr(path);
        }
    }

    let mut line = format!(
        "webview  {label}: {} — {}, exit {exit_code:#010x}",
        kind_name(kind),
        reason_name(reason)
    );
    if !module.is_empty() {
        line.push_str(&format!(", in {module}"));
    }
    log::line(line);

    if webview2_recovers(kind) {
        log::line(format!(
            "webview  {label}: the runtime replaces that process itself"
        ));
        return;
    }
    recover(app);
}

fn webview2_recovers(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> bool {
    matches!(
        kind,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED
            | COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED
    )
}

/// Start Oczi again, unless it has already been tried too often.
fn recover(app: &AppHandle) {
    if RESTARTING.swap(true, Ordering::SeqCst) {
        return;
    }
    if recent_restarts() >= LOOP_LIMIT {
        log::line(format!(
            "webview  not restarting: {LOOP_LIMIT} failures in {} minutes",
            LOOP_WINDOW_SECS / 60
        ));
        // The one window that could still show a message is the dead one, so the
        // only place left to say this is the tray tooltip.
        if let Some(tray) = app.tray_by_id("oczi") {
            let _ = tray.set_tooltip(Some("Oczi — WebView2 ciągle zawodzi, zobacz oczi.log"));
        }
        return;
    }
    record_restart();
    log::line("webview  restarting Oczi — a WebView2 environment cannot be rebuilt in place");

    tauri_plugin_single_instance::destroy(app);
    let env = app.env();
    tauri::process::restart(&env);
}

/// Restarts are counted in a file rather than in memory: they throw away the
/// process that would have been holding the count.
fn restart_log_path() -> PathBuf {
    settings::local_dir().join("webview-restarts")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

fn recent_restarts() -> usize {
    let cutoff = now_secs().saturating_sub(LOOP_WINDOW_SECS);
    match std::fs::read_to_string(restart_log_path()) {
        Ok(text) => text
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .filter(|stamp| *stamp >= cutoff)
            .count(),
        Err(_) => 0,
    }
}

fn record_restart() {
    let cutoff = now_secs().saturating_sub(LOOP_WINDOW_SECS);
    let mut kept: Vec<String> = std::fs::read_to_string(restart_log_path())
        .map(|text| {
            text.lines()
                .filter_map(|line| line.trim().parse::<u64>().ok())
                .filter(|stamp| *stamp >= cutoff)
                .map(|stamp| stamp.to_string())
                .collect()
        })
        .unwrap_or_default();
    kept.push(now_secs().to_string());

    let dir = settings::local_dir();
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("webview-restarts"), kept.join("\n") + "\n");
    }
}

/// What the runtime calls each failure kind, so the log reads as a sentence.
fn kind_name(kind: COREWEBVIEW2_PROCESS_FAILED_KIND) -> &'static str {
    use COREWEBVIEW2_PROCESS_FAILED_KIND as Kind;
    const KINDS: &[(Kind, &str)] = &[
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            "browser process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
            "render process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
            "render process unresponsive",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
            "frame render process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_UTILITY_PROCESS_EXITED,
            "utility process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_SANDBOX_HELPER_PROCESS_EXITED,
            "sandbox helper process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
            "gpu process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_PLUGIN_PROCESS_EXITED,
            "ppapi plugin process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_PPAPI_BROKER_PROCESS_EXITED,
            "ppapi broker process exited",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_KIND_UNKNOWN_PROCESS_EXITED,
            "unknown process exited",
        ),
    ];
    KINDS
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, name)| *name)
        .unwrap_or("process failed for another reason")
}

fn reason_name(reason: COREWEBVIEW2_PROCESS_FAILED_REASON) -> &'static str {
    use COREWEBVIEW2_PROCESS_FAILED_REASON as Reason;
    const REASONS: &[(Reason, &str)] = &[
        (COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED, "unexpected"),
        (
            COREWEBVIEW2_PROCESS_FAILED_REASON_UNRESPONSIVE,
            "unresponsive",
        ),
        (COREWEBVIEW2_PROCESS_FAILED_REASON_TERMINATED, "terminated"),
        (COREWEBVIEW2_PROCESS_FAILED_REASON_CRASHED, "crashed"),
        (
            COREWEBVIEW2_PROCESS_FAILED_REASON_LAUNCH_FAILED,
            "launch failed",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_REASON_OUT_OF_MEMORY,
            "out of memory",
        ),
        (
            COREWEBVIEW2_PROCESS_FAILED_REASON_PROFILE_DELETED,
            "profile deleted",
        ),
    ];
    REASONS
        .iter()
        .find(|(known, _)| *known == reason)
        .map(|(_, name)| *name)
        .unwrap_or("reason not reported")
}
