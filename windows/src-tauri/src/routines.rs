// Routines: things Oczi does by itself at a time the user named.
//
// A routine is a stored *intent*: a short list of tool calls the model composed when
// it was asked ("in 10 minutes open YouTube and search for urbex history"), plus the
// app's own clock. There is no catalogue of known routines and no script, no scheduled
// task and no file left anywhere: the model plans the steps out of the tools it has at
// that moment, this module keeps them and runs exactly those when the moment arrives.
// That is what makes an arbitrary request work — "open my music for programming in an
// hour" is only a plan the model can express, and a plan it cannot express is refused
// while the user is still talking rather than at the hour it should have happened.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::reminders as rem;

/// How long the scheduler may sleep between looks. A routine due sooner cuts the
/// sleep short, so this is an upper bound, not a delay.
const TICK: Duration = Duration::from_secs(20);

/// A routine that came due while Oczi was off is *not* replayed — opening a page or
/// launching a game three hours late is a surprise, not a promise kept. It is reported
/// as missed instead. Anything inside this window still runs: a routine due at 20:00
/// and an app that starts at 20:00:40 are the same thing to the user.
const MISSED_GRACE_SECS: i64 = 120;

/// A routine is a short sequence. More belongs in the chat, as work the model does now.
const MAX_STEPS: usize = 4;

/// The tools a step may be: the machine actions, and nothing else. A lookup is not a
/// routine — searching the web at 20:00 and doing nothing with the result means nothing.
pub const STEP_TOOLS: [&str; 5] = [
    "launch_app",
    "open_url",
    "play_video",
    "run_terminal",
    "kill_process",
];

// ── What a routine is ─────────────────────────────────────────────────────────

/// One tool call, as the model planned it. `args` is passed to the tool untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub tool: String,
    pub args: Value,
}

/// What one step did, in the shape the card needs: the tool, the *name* it acted on and
/// whether it worked. The island translates the verb and prints the name after it, so
/// nothing here may be a sentence in any language.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub tool: String,
    pub arg: String,
    pub ok: bool,
}

/// The result of a run, kept until the user has seen it. A report nobody read is handed
/// over again on the next launch, which is why it lives in the file and not in the event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    /// Seconds past its time when the run started, for the log and the card.
    pub late_seconds: i64,
    /// It never ran: it came due while Oczi was off, or a previous run was interrupted.
    pub missed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: u64,
    /// A short name the model wrote in the user's language, shown as the card's title.
    pub label: String,
    /// Local wall-clock seconds when it is next due.
    pub at: i64,
    pub created: i64,
    /// Absent for a one-off. A repeating routine runs again at the same clock time,
    /// and only its report is answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<rem::Repeat>,
    pub steps: Vec<Step>,
    /// The moment the current run was due. Kept separately because a repeating routine's
    /// `at` has already moved to the next occurrence by the time anything is reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<i64>,
    /// It is being run right now. Persisted *before* the first step, so a crash or a kill
    /// in the middle of a run cannot make the same routine run twice on the next launch.
    #[serde(default)]
    pub running: bool,
    /// The last result, while nobody has clicked OK on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
    /// That report has been put on screen at least once, for the model's own reading of
    /// the list. A report that was never shown is shown by `ready()` instead.
    #[serde(default)]
    pub shown: bool,
}

// ── The file ──────────────────────────────────────────────────────────────────

fn path() -> PathBuf {
    crate::settings::config_dir().join("routines.json")
}

/// An unreadable file is treated as an empty one: a routine must never be the reason
/// the app cannot start, and the bad file is left on the disk untouched.
fn load() -> Vec<Routine> {
    match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            crate::log::line(format!("routines unreadable ({e}), starting empty"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save(list: &[Routine]) -> Result<(), String> {
    let dir = crate::settings::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path(), json).map_err(|e| e.to_string())
}

/// Everything scheduled, soonest first.
pub fn all() -> Vec<Routine> {
    let mut list = load();
    list.sort_by_key(|r| r.at);
    list
}

pub fn add(
    label: &str,
    at: i64,
    repeat: Option<rem::Repeat>,
    steps: Vec<Step>,
) -> Result<Routine, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Pusta rutyna — podaj, co ma się stać.".to_string());
    }
    if steps.is_empty() {
        return Err("Rutyna bez kroków nic nie robi.".to_string());
    }
    let mut list = load();
    let id = list.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    let routine = Routine {
        id,
        // The card's title: long text is clipped rather than helpful.
        label: label.chars().take(120).collect(),
        at,
        created: rem::now_local(),
        repeat,
        steps,
        due: None,
        running: false,
        report: None,
        shown: false,
    };
    list.push(routine.clone());
    save(&list)?;
    Ok(routine)
}

/// Takes a routine off the list for good. This is the model's `cancel_routine`.
pub fn remove(id: u64) -> Result<bool, String> {
    let mut list = load();
    let before = list.len();
    list.retain(|r| r.id != id);
    if list.len() == before {
        return Ok(false);
    }
    save(&list)?;
    Ok(true)
}

/// The report has been read. A one-off is finished and leaves the list; a repeating one
/// keeps its place and loses only the report — its next occurrence is already set.
pub fn answered(id: u64) -> Result<bool, String> {
    let mut list = load();
    let Some(pos) = list.iter().position(|r| r.id == id) else {
        return Ok(false);
    };
    if list[pos].repeat.is_none() {
        list.remove(pos);
    } else {
        list[pos].report = None;
        list[pos].due = None;
        list[pos].shown = false;
    }
    save(&list)?;
    Ok(true)
}

/// The list as the model reads it back: what is set, what the steps are, and how the
/// last run went — a routine that opened nothing is a question the user will ask.
pub fn describe() -> String {
    let list = all();
    if list.is_empty() {
        return "Brak zaplanowanych rutyn.".to_string();
    }
    let mut out = format!("Zaplanowane rutyny ({}):", list.len());
    for r in list {
        let mut line = format!("\n#{} {} — {}", r.id, rem::format_local(r.at), r.label);
        if let Some(repeat) = r.repeat {
            line.push_str(&format!(" [{}]", rem::repeat_word(repeat)));
        }
        line.push_str(&format!(" — {} kroków", r.steps.len()));
        for (i, s) in r.steps.iter().enumerate() {
            line.push_str(&format!("\n    {}. {} {}", i + 1, s.tool, short_arg(&s.tool, &s.args)));
        }
        if let Some(rep) = &r.report {
            let done = rep.outcomes.iter().filter(|o| o.ok).count();
            line.push_str(&format!("\n    ostatni wynik: {}/{} kroków", done, rep.outcomes.len()));
            if rep.missed {
                line.push_str(" — nie odpaliła (Oczi nie działał)");
            } else if r.shown {
                line.push_str(" — karta pokazana, czeka na zamknięcie");
            }
        }
        out.push_str(&line);
    }
    out
}

// ── Reading a plan ────────────────────────────────────────────────────────────

/// The `steps` the model sent, checked before anything is stored. A plan that cannot run
/// is refused here, while the model can still fix it or say so — never at the moment the
/// user is waiting for it to happen.
pub fn parse_steps(raw: Option<&Value>, terminal: bool) -> Result<Vec<Step>, String> {
    let Some(raw) = raw else {
        return Err("Brak kroków — podaj „steps”.".to_string());
    };
    let steps: Vec<Step> = serde_json::from_value(raw.clone())
        .map_err(|_| "Każdy krok to {\"tool\": \"…\", \"args\": {…}}.".to_string())?;
    if steps.is_empty() {
        return Err("Rutyna bez kroków nic nie robi.".to_string());
    }
    if steps.len() > MAX_STEPS {
        return Err(format!(
            "Maksymalnie {MAX_STEPS} kroki w jednej rutynie — więcej zrób w rozmowie teraz."
        ));
    }
    // Every step is a machine action, so with terminal access off there is nothing this
    // could be made of. Saying it as an error is what makes the model tell the user
    // instead of promising a routine that can never run.
    if !terminal {
        return Err(
            "Dostęp do terminala jest wyłączony w ustawieniach, więc nie mogę zaplanować żadnego kroku — powiedz użytkownikowi, żeby go włączył."
                .to_string(),
        );
    }
    for step in &steps {
        check_step(step)?;
    }
    Ok(steps)
}

/// One step, against the tool it names. The arguments are the ones that tool documents,
/// so a plan the model wrote half-heartedly is caught here and not at the hour.
fn check_step(step: &Step) -> Result<(), String> {
    if !STEP_TOOLS.contains(&step.tool.as_str()) {
        return Err(format!(
            "Nie znam kroku „{}” — rutyna może się składać tylko z: {}.",
            step.tool,
            STEP_TOOLS.join(", ")
        ));
    }
    let field = |key: &str| {
        step.args
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    match step.tool.as_str() {
        "launch_app" => {
            let target = match field("target") {
                t if t.is_empty() => field("path"),
                t => t,
            };
            if target.is_empty() {
                return Err("Krok launch_app potrzebuje „target” — nazwy albo ścieżki do otwarcia.".to_string());
            }
        }
        "open_url" => {
            let url = field("url");
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(format!(
                    "Krok open_url potrzebuje „url” z http:// albo https:// — dostałem „{url}”."
                ));
            }
        }
        "play_video" => {
            if field("query").is_empty() {
                return Err("Krok play_video potrzebuje „query” — czego poszukać i puścić.".to_string());
            }
        }
        "run_terminal" => {
            if field("command").is_empty() {
                return Err("Krok run_terminal potrzebuje „command”.".to_string());
            }
        }
        "kill_process" => {
            // The same check the tool itself makes, so a name that would be refused at
            // the hour is refused while the user is still talking.
            crate::shell::image_name(&field("name"))?;
        }
        _ => {}
    }
    Ok(())
}

/// What a step acted on, short enough for one line of the card: a program, a host, the
/// first words of a command, a process. The card prints it after a translated verb, so
/// this has to be a name and never a sentence.
fn short_arg(tool: &str, args: &Value) -> String {
    let text = match tool {
        "launch_app" => args
            .get("target")
            .or_else(|| args.get("path"))
            .and_then(Value::as_str)
            .unwrap_or(""),
        "open_url" => args.get("url").and_then(Value::as_str).unwrap_or(""),
        // What was asked for, not what it turned out to be: the title is only known once the
        // search is done, and this is read before the step runs.
        "play_video" => args.get("query").and_then(Value::as_str).unwrap_or(""),
        "run_terminal" => args.get("command").and_then(Value::as_str).unwrap_or(""),
        "kill_process" => args.get("name").and_then(Value::as_str).unwrap_or(""),
        _ => "",
    };
    let text = text.trim();
    // An address is a query string nobody reads on a card: the host is the name.
    let text = if tool == "open_url" {
        host_of(text)
    } else {
        text.to_string()
    };
    let clipped: String = text.chars().take(48).collect();
    if text.chars().count() > 48 {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

// ── Firing ────────────────────────────────────────────────────────────────────

/// The card Rust hands to the island when a routine has run.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Fired {
    id: u64,
    label: String,
    /// The time it was due, formatted: the card does no calendar arithmetic.
    at_text: String,
    late_seconds: i64,
    missed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat: Option<rem::Repeat>,
    outcomes: Vec<Outcome>,
}

static LISTENING: AtomicBool = AtomicBool::new(false);

/// What the clock says about one routine.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Due, and close enough to its time to run.
    Run,
    /// Due long ago, or interrupted by a crash: reported, never replayed.
    Missed,
    NotYet,
}

fn decide(r: &Routine, now: i64) -> Decision {
    // Already run, and its report is still waiting to be read. Without this a one-off
    // would run again on every pass, because its `at` stays in the past while it waits.
    if r.report.is_some() {
        return Decision::NotYet;
    }
    if r.running {
        return Decision::Missed;
    }
    if r.at > now {
        return Decision::NotYet;
    }
    if now - r.at > MISSED_GRACE_SECS {
        return Decision::Missed;
    }
    Decision::Run
}

/// Seconds until the next routine is due, if there is one.
fn next_in(now: i64) -> Option<i64> {
    all()
        .into_iter()
        .filter(|r| r.report.is_none() && !r.running)
        .map(|r| (r.at - now).max(0))
        .min()
}

/// What happens to the reports already in the file at the start of a run. Pure, so the rule is
/// testable without an app: returns whether the list changed.
///
/// A report the user never answered is handed over again **only if it never reached the
/// screen** — a crash or a kill between writing it and showing it. One that *was* shown has
/// been read or put away, like a notification the user folded, and must not come back on every
/// launch: that is how an old routine ends up looking like Oczi remembering something nobody
/// asked for. A one-off is then finished for good and leaves the list; a repeating one keeps
/// its place and loses only the report.
fn settle_reports(list: &mut Vec<Routine>, log: &mut dyn FnMut(String)) -> bool {
    let before = list.len();
    list.retain(|r| !(r.report.is_some() && r.shown && r.repeat.is_none()));
    let mut changed = list.len() != before;
    if changed {
        log("one-off routines whose report was put away are gone".to_string());
    }
    for r in list.iter_mut() {
        if r.report.is_some() && r.shown && r.repeat.is_some() {
            r.report = None;
            r.due = None;
            r.shown = false;
            changed = true;
            log(format!("routine {}: report put away, next run kept", r.id));
        }
    }
    changed
}

/// The island has its listener wired: hand it every report that never reached the screen — a
/// run interrupted by a crash, or a card lost to a restart — and let the loop start looking.
pub fn ready(app: &AppHandle) {
    LISTENING.store(true, Ordering::Relaxed);
    let now = rem::now_local();
    let mut list = load();
    let mut changed = false;
    // A run a crash interrupted is decided here: never replayed, because half of it may
    // already have happened.
    for r in list.iter_mut() {
        if r.running {
            r.running = false;
            r.report = Some(Report {
                outcomes: Vec::new(),
                late_seconds: r.due.map(|due| (now - due).max(0)).unwrap_or(0),
                missed: true,
            });
            r.shown = false;
            changed = true;
        }
    }
    let mut log = |line: String| crate::log::line(line);
    changed |= settle_reports(&mut list, &mut log);
    if changed {
        if let Err(e) = save(&list) {
            crate::log::line(format!("routines could not be saved: {e}"));
        }
    }
    for r in load() {
        if r.report.is_some() && !r.shown {
            show(app, &r);
        }
    }
}

/// Watches the file and runs whatever is due. Started from `lib.rs` next to the other
/// pollers. The steps themselves run whether or not the island is listening — the work is
/// the promise — while the card waits for the listener: see `show`.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let wait = match next_in(rem::now_local()) {
                // Due now, or already: look again in a second.
                Some(0) => 1,
                Some(secs) => secs.min(TICK.as_secs() as i64),
                None => TICK.as_secs() as i64,
            };
            tokio::time::sleep(Duration::from_secs(wait.max(1) as u64)).await;
            fire(&app);
        }
    });
}

fn fire(app: &AppHandle) {
    let now = rem::now_local();
    let mut list = load();
    let mut to_run: Vec<(Routine, i64)> = Vec::new();
    let mut to_report: Vec<Routine> = Vec::new();
    let mut changed = false;

    for r in list.iter_mut() {
        match decide(r, now) {
            Decision::NotYet => continue,
            Decision::Missed => {
                r.report = Some(Report {
                    outcomes: Vec::new(),
                    late_seconds: (now - r.at).max(0),
                    missed: true,
                });
                r.due = Some(r.at);
                r.shown = false;
                // A repeating routine keeps its place: the next occurrence is tomorrow,
                // not a moment that has already gone.
                if let Some(repeat) = r.repeat {
                    r.at = rem::next_after(r.at, repeat, now);
                }
                to_report.push(r.clone());
                changed = true;
            }
            Decision::Run => {
                let due = r.at;
                r.due = Some(due);
                r.running = true;
                if let Some(repeat) = r.repeat {
                    r.at = rem::next_after(r.at, repeat, now);
                }
                to_run.push((r.clone(), due));
                changed = true;
            }
        }
    }
    if changed {
        if let Err(e) = save(&list) {
            crate::log::line(format!("routines could not be saved: {e}"));
            return;
        }
    }
    for r in to_report {
        show(app, &r);
    }
    for (r, due) in to_run {
        execute(app.clone(), r, due, now);
    }
}

/// Runs a routine on the spot, without waiting for its time. The card reports it like any
/// other run, so nothing happens silently.
pub fn run_now(app: &AppHandle, id: u64) -> Result<(), String> {
    let mut list = load();
    let Some(pos) = list.iter().position(|r| r.id == id) else {
        return Err(format!("Nie ma rutyny #{id}."));
    };
    if list[pos].running {
        return Err(format!("Rutyna #{id} właśnie się wykonuje."));
    }
    let due = rem::now_local();
    list[pos].due = Some(due);
    list[pos].running = true;
    list[pos].report = None;
    list[pos].shown = false;
    let routine = list[pos].clone();
    save(&list)?;
    execute(app.clone(), routine, due, due);
    Ok(())
}

/// The steps, off the runtime's own threads: opening a program and running a command is
/// real work that would otherwise hold a worker for as long as the command takes.
fn execute(app: AppHandle, r: Routine, due: i64, now: i64) {
    crate::log::line(format!(
        "routine {} running: {} ({} steps, due {}, {}s late)",
        r.id,
        r.label,
        r.steps.len(),
        rem::format_local(due),
        (now - due).max(0)
    ));
    let late = (now - due).max(0);
    tauri::async_runtime::spawn(async move {
        let steps = r.steps.clone();
        let outcomes = tauri::async_runtime::spawn_blocking(move || run_steps(steps))
            .await
            .unwrap_or_default();
        let done = outcomes.iter().filter(|o| o.ok).count();
        crate::log::line(format!(
            "routine {} ran: {done}/{} steps ok",
            r.id,
            outcomes.len()
        ));
        finish(
            &app,
            r.id,
            Report {
                outcomes,
                late_seconds: late,
                missed: false,
            },
        );
    });
}

/// One step after another; a step that fails does not stop the rest, because a routine
/// that opened two of three things is a result the user wants to hear about. Every step
/// is honoured whatever the switches say *now*: it was planned with them on and the user
/// agreed to the plan by asking for it.
fn run_steps(steps: Vec<Step>) -> Vec<Outcome> {
    let mut out = Vec::new();
    for step in steps {
        let arg = short_arg(&step.tool, &step.args);
        let result = tauri::async_runtime::block_on(crate::deepseek::run_step(&step.tool, &step.args));
        match &result {
            Ok(_) => crate::log::line(format!("routine step ok: {} {arg}", step.tool)),
            Err(e) => crate::log::line(format!("routine step failed: {} {arg}: {e}", step.tool)),
        }
        out.push(Outcome {
            tool: step.tool,
            arg,
            ok: result.is_ok(),
        });
    }
    out
}

/// Writes the report into the file and puts it on screen. The file first: a card that
/// cannot be shown is still there on the next launch.
fn finish(app: &AppHandle, id: u64, report: Report) {
    let mut list = load();
    let Some(rec) = list.iter_mut().find(|r| r.id == id) else {
        // Cancelled while it was running. The work happened; there is nobody to tell.
        crate::log::line(format!("routine {id} finished but is no longer on the list"));
        return;
    };
    rec.running = false;
    rec.report = Some(report);
    rec.shown = false;
    let routine = rec.clone();
    if let Err(e) = save(&list) {
        crate::log::line(format!("routines could not be saved: {e}"));
    }
    show(app, &routine);
}

/// Puts a report on the island, if the island can hear it. A report that cannot be shown
/// stays unread in the file and is handed over by `ready()` on the next launch — the one
/// failure nobody would ever notice otherwise.
fn show(app: &AppHandle, r: &Routine) {
    let Some(rep) = &r.report else {
        return;
    };
    let due = r.due.unwrap_or(r.at);
    let payload = Fired {
        id: r.id,
        label: r.label.clone(),
        at_text: rem::format_local(due),
        late_seconds: rep.late_seconds,
        missed: rep.missed,
        repeat: r.repeat,
        outcomes: rep.outcomes.clone(),
    };
    let done = rep.outcomes.iter().filter(|o| o.ok).count();
    crate::log::line(format!(
        "routine {} report ready: {done}/{} steps, due {}{}",
        r.id,
        rep.outcomes.len(),
        payload.at_text,
        if rep.missed { ", missed" } else { "" }
    ));
    if !LISTENING.load(Ordering::Relaxed) {
        crate::log::line(format!(
            "routine {} held: the island is not listening yet",
            r.id
        ));
        return;
    }
    match app.emit_to(crate::island::WINDOW_LABEL, "routine", payload) {
        Ok(()) => {
            let mut list = load();
            if let Some(rec) = list.iter_mut().find(|x| x.id == r.id) {
                rec.shown = true;
                if let Err(e) = save(&list) {
                    crate::log::line(format!("routines could not be saved: {e}"));
                }
            }
        }
        Err(e) => crate::log::line(format!("routine {} could not be shown: {e}", r.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn step(tool: &str, args: Value) -> Step {
        Step {
            tool: tool.to_string(),
            args,
        }
    }

    fn plan(steps: Vec<Step>) -> Value {
        serde_json::to_value(steps).unwrap()
    }

    #[test]
    fn a_plan_of_real_tool_calls_is_taken() {
        let steps = plan(vec![
            step("launch_app", json!({ "target": "spotify" })),
            step("open_url", json!({ "url": "https://www.youtube.com/results?search_query=urbex+history" })),
            step("kill_process", json!({ "name": "Steam" })),
            step("run_terminal", json!({ "command": "C:\\Users\\me\\Desktop\\FiveM\\start.cmd" })),
        ]);
        let parsed = parse_steps(Some(&steps), true).expect("a plan");
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[2].tool, "kill_process");
    }

    #[test]
    fn a_step_the_app_cannot_run_is_turned_down_while_the_user_is_still_there() {
        // A tool that is not a machine action: a routine that only looks something up
        // would do nothing the user could see.
        let bad = plan(vec![step("web_search", json!({ "query": "urbex" }))]);
        let err = parse_steps(Some(&bad), true).unwrap_err();
        assert!(err.contains("web_search"), "{err}");

        // Also the tools that belong to another card entirely.
        for tool in ["remind", "list_routines", "cancel_routine", "startup_items"] {
            let bad = plan(vec![step(tool, json!({}))]);
            assert!(parse_steps(Some(&bad), true).is_err(), "{tool} must be refused");
        }
    }

    #[test]
    fn a_step_without_its_argument_is_refused() {
        for (tool, args) in [
            ("launch_app", json!({})),
            ("launch_app", json!({ "target": "   " })),
            ("open_url", json!({ "url": "youtube.com" })),
            ("open_url", json!({ "url": "file:///C:/x.html" })),
            ("run_terminal", json!({})),
            ("kill_process", json!({ "name": "" })),
            // A wildcard is how one mistake closes half the machine.
            ("kill_process", json!({ "name": "*" })),
            ("kill_process", json!({ "name": "explorer" })),
            ("kill_process", json!({ "name": "msedgewebview2" })),
        ] {
            let bad = plan(vec![step(tool, args.clone())]);
            assert!(
                parse_steps(Some(&bad), true).is_err(),
                "{tool} {args} should not be a step"
            );
        }
    }

    #[test]
    fn launch_app_takes_the_old_path_key_too() {
        let ok = plan(vec![step("launch_app", json!({ "path": "C:\\x\\y.exe" }))]);
        assert!(parse_steps(Some(&ok), true).is_ok());
    }

    #[test]
    fn an_empty_or_overlong_plan_is_refused() {
        assert!(parse_steps(Some(&json!([])), true).is_err());
        assert!(parse_steps(None, true).is_err());
        let many = plan((0..MAX_STEPS + 1)
            .map(|i| step("launch_app", json!({ "target": format!("app{i}") })))
            .collect());
        assert!(parse_steps(Some(&many), true).is_err());
    }

    #[test]
    fn nothing_is_planned_without_terminal_access() {
        let ok = plan(vec![step("launch_app", json!({ "target": "spotify" }))]);
        assert!(parse_steps(Some(&ok), true).is_ok());
        assert!(parse_steps(Some(&ok), false).is_err());
    }

    #[test]
    fn a_name_on_the_card_is_a_name_not_a_query_string() {
        assert_eq!(
            short_arg(
                "open_url",
                &json!({ "url": "https://www.youtube.com/results?search_query=urbex+history" })
            ),
            "www.youtube.com"
        );
        assert_eq!(
            short_arg("launch_app", &json!({ "target": "spotify" })),
            "spotify"
        );
        assert_eq!(
            short_arg("kill_process", &json!({ "name": "Discord" })),
            "Discord"
        );
        let long = "x".repeat(60);
        assert_eq!(short_arg("run_terminal", &json!({ "command": long })).chars().count(), 49);
    }

    #[test]
    fn a_due_routine_runs_and_one_that_has_gone_is_only_reported() {
        let base = Routine {
            id: 1,
            label: "youtube".to_string(),
            at: 1_000,
            created: 0,
            repeat: None,
            steps: vec![step("launch_app", json!({ "target": "spotify" }))],
            due: None,
            running: false,
            report: None,
            shown: false,
        };
        // Five seconds late: it still runs, the user just asked for it.
        assert_eq!(decide(&base, 1_005), Decision::Run);
        // Not yet.
        assert_eq!(decide(&base, 999), Decision::NotYet);
        // Ten minutes late: Oczi was off, and opening a page now is a surprise.
        assert_eq!(decide(&base, 1_600), Decision::Missed);

        // A report nobody has read holds the routine where it is: a one-off's `at` stays
        // in the past while it waits, and it must not run again on the next pass.
        let waiting = Routine {
            report: Some(Report {
                outcomes: Vec::new(),
                late_seconds: 0,
                missed: false,
            }),
            ..base.clone()
        };
        assert_eq!(decide(&waiting, 1_600), Decision::NotYet);

        // A run a crash interrupted is reported, never replayed.
        let interrupted = Routine {
            running: true,
            ..base
        };
        assert_eq!(decide(&interrupted, 1_005), Decision::Missed);
    }

    #[test]
    fn a_repeating_routine_keeps_its_place_after_it_has_run() {
        let daily = rem::Repeat::Daily;
        let base = 20_000i64;
        // The next occurrence is a day later, at the same clock time — the same rule the
        // reminders use, which is why it is called rather than written again here.
        assert_eq!(rem::next_after(base, daily, base), base + 86_400);
        assert!(rem::next_after(base, daily, base + 3 * 86_400) > base + 3 * 86_400);
    }

    #[test]
    fn reads_the_forms_the_tool_documents() {
        assert!(rem::parse_when("+10m").is_ok());
        assert_eq!(rem::parse_repeat("codziennie"), Ok(Some(rem::Repeat::Daily)));
    }

    #[test]
    fn a_report_that_was_shown_does_not_come_back_forever() {
        let base = |id: u64, repeat: Option<rem::Repeat>, shown: bool| Routine {
            id,
            label: format!("r{id}"),
            at: 1_000,
            created: 0,
            repeat,
            steps: vec![step("launch_app", json!({ "target": "spotify" }))],
            due: Some(1_000),
            running: false,
            report: Some(Report {
                outcomes: Vec::new(),
                late_seconds: 0,
                missed: false,
            }),
            shown,
        };

        // A one-off whose report reached the card and was put away is finished: off the list,
        // so it cannot look like a routine remembered from a previous session.
        let mut list = vec![base(1, None, true)];
        assert!(settle_reports(&mut list, &mut |_| {}));
        assert!(list.is_empty());

        // One whose report never reached the screen (a crash between the two, or a shutdown
        // before the island listened) is kept, so its card still gets shown.
        let mut list = vec![base(2, None, false)];
        assert!(!settle_reports(&mut list, &mut |_| {}));
        assert_eq!(list.len(), 1);
        assert!(list[0].report.is_some());

        // A repeating routine keeps its place and loses only the report: its next occurrence
        // is what matters, and the old result is news that has been read.
        let mut list = vec![base(3, Some(rem::Repeat::Daily), true)];
        assert!(settle_reports(&mut list, &mut |_| {}));
        assert_eq!(list.len(), 1);
        assert!(list[0].report.is_none());
        assert_eq!(list[0].repeat, Some(rem::Repeat::Daily));

        // A routine with no report at all is not touched, and nothing is written back.
        let mut list = vec![Routine {
            report: None,
            shown: false,
            ..base(4, None, false)
        }];
        assert!(!settle_reports(&mut list, &mut |_| {}));
        assert_eq!(list.len(), 1);
    }
}
