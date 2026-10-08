// Reminders, the one thing Oczi has to remember for the user.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

/// How long the scheduler may sleep between looks. A reminder that comes due
/// sooner than this cuts the sleep short, so it is an upper bound, not a delay.
const TICK: Duration = Duration::from_secs(20);

/// Nothing is scheduled further out than this: far enough for "next year", short
/// enough that a mis-parsed date cannot sit in the file unnoticed.
const MAX_AHEAD_DAYS: i64 = 366;

const FORMS: &str =
    "Podaj czas jako \"+20m\", \"18:30\", \"jutro 08:00\" albo \"2026-10-12 09:00\".";

/// How a repeating reminder comes back. Anything else is a one-off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    /// Every day at the same clock time.
    Daily,
    /// Monday to Friday at the same clock time.
    Weekdays,
    /// The same weekday, every week.
    Weekly,
}

impl Repeat {
    /// The next occurrence after `at`, always the same clock time.
    fn step(self, at: i64) -> i64 {
        match self {
            Repeat::Daily => at + 86_400,
            Repeat::Weekly => at + 7 * 86_400,
            Repeat::Weekdays => {
                let mut next = at + 86_400;
                while is_weekend(next) {
                    next += 86_400;
                }
                next
            }
        }
    }
}

/// Saturday or Sunday, in local wall-clock seconds.
fn is_weekend(secs: i64) -> bool {
    // 1970-01-01 was a Thursday, so this is Sunday = 0 … Saturday = 6.
    matches!((secs.div_euclid(86_400) + 4).rem_euclid(7), 0 | 6)
}

/// The next time this should come up, never in the past: a daily reminder answered
/// after a week away lands on tomorrow, not on a day that has already gone.
fn next_after(at: i64, repeat: Repeat, now: i64) -> i64 {
    let mut next = at;
    // Bounded so a nonsense stamp in the file cannot spin here.
    for _ in 0..4_096 {
        next = repeat.step(next);
        if next > now {
            break;
        }
    }
    next
}

/// The `repeat` a tool call asked for, in either language.
pub fn parse_repeat(spec: &str) -> Result<Option<Repeat>, String> {
    let word = spec.trim().to_lowercase();
    Ok(match word.as_str() {
        "" | "none" | "once" | "off" | "no" | "false" | "jednorazowo" | "raz" | "nigdy" => None,
        "daily" | "day" | "every day" | "everyday" | "codziennie" | "dziennie" | "dzien" => {
            Some(Repeat::Daily)
        }
        "weekdays" | "working days" | "workdays" | "mon-fri" | "dni robocze" | "robocze"
        | "w dni robocze" => Some(Repeat::Weekdays),
        "weekly" | "week" | "every week" | "co tydzien" | "co tydzień" | "tygodniowo" => {
            Some(Repeat::Weekly)
        }
        other => {
            return Err(format!(
                "Nie znam powtarzania „{other}” — użyj daily, weekdays albo weekly."
            ))
        }
    })
}

/// The word for a repeat, for messages the model reads and writes on.
pub fn repeat_word(repeat: Repeat) -> &'static str {
    match repeat {
        Repeat::Daily => "codziennie",
        Repeat::Weekdays => "w dni robocze",
        Repeat::Weekly => "co tydzień",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: u64,
    pub text: String,
    /// Local wall-clock seconds when it is due.
    pub at: i64,
    /// When it was asked for, the list is ordered by `at`, this is for the log.
    pub created: i64,
    /// It has been put on screen already, but nobody has answered it. Such an entry
    /// stays on the list: a card lost to a crash, a restart or a shutdown has to be
    /// the card that comes back, not the reminder that quietly never returned.
    #[serde(default)]
    pub fired: bool,
    /// Absent for a one-off. A repeating reminder is answered like any other, and
    /// answering it moves it to its next occurrence instead of ending it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
}

// ── The file ──────────────────────────────────────────────────────────────────

fn path() -> PathBuf {
    crate::settings::config_dir().join("reminders.json")
}

/// An unreadable file is treated as an empty one: a reminder must never be the
/// reason the app cannot start, and the bad file is left on the disk untouched.
fn load() -> Vec<Reminder> {
    match std::fs::read(path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            crate::log::line(format!("reminders unreadable ({e}), starting empty"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save(list: &[Reminder]) -> Result<(), String> {
    let dir = crate::settings::config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path(), json).map_err(|e| e.to_string())
}

/// Everything scheduled, soonest first.
pub fn all() -> Vec<Reminder> {
    let mut list = load();
    list.sort_by_key(|r| r.at);
    list
}

pub fn add(text: &str, at: i64, repeat: Option<Repeat>) -> Result<Reminder, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Puste przypomnienie — podaj, o czym przypomnieć.".to_string());
    }
    let mut list = load();
    let id = list.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    let reminder = Reminder {
        id,
        // A card the island has to draw: long text would be clipped rather than
        // helpful, and the model can say the rest in the answer.
        text: text.chars().take(300).collect(),
        at,
        created: now_local(),
        fired: false,
        repeat,
    };
    list.push(reminder.clone());
    save(&list)?;
    Ok(reminder)
}

/// Takes one off the list for good. This is the model's `cancel_reminder`, and the
/// only way a one-off leaves.
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

pub fn answered(id: u64) -> Result<Option<i64>, String> {
    let mut list = load();
    let now = now_local();
    let Some(r) = list.iter_mut().find(|r| r.id == id) else {
        return Ok(None);
    };
    let Some(repeat) = r.repeat else {
        list.retain(|r| r.id != id);
        save(&list)?;
        return Ok(None);
    };
    r.at = next_after(r.at, repeat, now);
    r.fired = false;
    let next = r.at;
    save(&list)?;
    Ok(Some(next))
}

fn select_due(
    list: Vec<Reminder>,
    now: i64,
    leftovers: bool,
) -> (Vec<Reminder>, Vec<Reminder>) {
    list.into_iter().partition(|r| {
        if leftovers {
            r.fired || r.at <= now
        } else {
            r.at <= now && !r.fired
        }
    })
}

/// What is due now, marked as shown in the same breath so it is not handed over
/// twice in one run.
fn take_due(now: i64, leftovers: bool) -> Vec<Reminder> {
    let (mut due, rest) = select_due(load(), now, leftovers);
    if due.is_empty() {
        return due;
    }
    due.sort_by_key(|r| r.at);
    let mut keep = rest;
    for r in &mut due {
        r.fired = true;
    }
    keep.extend(due.iter().cloned());
    // Shown is worth more than tidy: a failed write is logged, and the reminder is
    // put on screen anyway rather than being silently swallowed.
    if let Err(e) = save(&keep) {
        crate::log::line(format!("reminders could not be saved: {e}"));
    }
    due
}

/// The list as the model reads it back. A reminder that was already shown but not
/// answered is still on the list, and says so: that is the one the user is looking
/// at right now, and the model must not set a second copy of it.
pub fn describe() -> String {
    let list = all();
    if list.is_empty() {
        return "Brak zaplanowanych przypomnień.".to_string();
    }
    let mut out = format!("Zaplanowane przypomnienia ({}):", list.len());
    for r in list {
        let mut line = format!("\n#{} {} — {}", r.id, format_local(r.at), r.text);
        if let Some(repeat) = r.repeat {
            line.push_str(&format!(" [{}]", repeat_word(repeat)));
        }
        if r.fired {
            line.push_str(" (pokazane, czeka na potwierdzenie)");
        }
        out.push_str(&line);
    }
    out
}

// ── The clock ─────────────────────────────────────────────────────────────────

/// Windows' own local time, so the time zone and DST are already applied.
fn local_fields() -> (i64, i64, i64, i64, i64, i64) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    (
        t.wYear as i64,
        t.wMonth as i64,
        t.wDay as i64,
        t.wHour as i64,
        t.wMinute as i64,
        t.wSecond as i64,
    )
}

/// Now, as local wall-clock seconds since the epoch.
pub fn now_local() -> i64 {
    let (y, mo, d, h, mi, s) = local_fields();
    days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s
}

/// Local wall-clock seconds for a date and time.
fn stamp(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
    days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60
}

/// Days since 1970-01-01 for a proleptic Gregorian date, Howard Hinnant's
/// `days_from_civil`. No calendar tables, and correct for every date we can store.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn add_days(y: i64, m: i64, d: i64, days: i64) -> (i64, i64, i64) {
    civil_from_days(days_from_civil(y, m, d) + days)
}

/// `2026-10-09 08:00`, the form the card and the list both show.
pub fn format_local(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        y,
        mo,
        d,
        rem / 3600,
        (rem % 3600) / 60
    )
}

// ── Reading `when` ────────────────────────────────────────────────────────────

/// `+20m`, `+1h30m`, `in 2 hours`, `za 90 minut`, and a bare number, read as
/// minutes. Returns seconds from now, or `None` when this is not a duration.
fn relative(text: &str) -> Option<i64> {
    let mut t = text.trim();
    if let Some(rest) = t.strip_prefix('+') {
        t = rest.trim();
    }
    for word in ["in ", "za "] {
        if let Some(rest) = t.strip_prefix(word) {
            t = rest.trim();
        }
    }
    if t.is_empty() {
        return None;
    }

    let mut total = 0i64;
    let mut seen_unit = false;
    let mut rest = t;
    loop {
        rest = rest.trim_start();
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            break;
        }
        let n: i64 = digits.parse().ok()?;
        rest = rest[digits.len()..].trim_start();
        let unit: String = rest.chars().take_while(|c| c.is_alphabetic()).collect();
        rest = &rest[unit.len()..];

        let seconds = match unit.as_str() {
            "s" | "sec" | "secs" | "sek" | "sekunda" | "sekundy" | "sekund" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" | "minut" | "minuta" | "minuty" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" | "godz" | "godzin" | "godzina" | "godziny" => {
                3600
            }
            "d" | "day" | "days" | "dzien" | "dzień" | "dni" | "doba" | "doby" => 86_400,
            // A number with no unit is minutes, but only on its own: "20 30" is not
            // something to guess at.
            "" => {
                if rest.trim().is_empty() {
                    return if n > 0 { Some(n * 60) } else { None };
                }
                return None;
            }
            _ => return None,
        };
        total += n * seconds;
        seen_unit = true;
        if rest.trim().is_empty() {
            break;
        }
    }
    if !seen_unit || total <= 0 {
        return None;
    }
    Some(total)
}

/// `18:30` / `8:5`, hours and minutes, minutes optional.
fn parse_clock(tok: &str) -> Option<(i64, i64)> {
    let (h, m) = tok.split_once(':')?;
    let h: i64 = h.trim().parse().ok()?;
    let m: i64 = if m.trim().is_empty() {
        0
    } else {
        m.trim().parse().ok()?
    };
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// `2026-10-12` or `12.10` / `12.10.2026` (day first, as it is written here).
fn parse_date(tok: &str, today: (i64, i64, i64)) -> Option<(i64, i64, i64)> {
    let parts: Vec<&str> = tok.split(['-', '.']).collect();
    let n = |s: &str| -> Option<i64> { s.trim().parse().ok() };
    match parts.as_slice() {
        [y, mo, d] if tok.contains('-') => {
            let (y, mo, d) = (n(y)?, n(mo)?, n(d)?);
            if mo == 0 || mo > 12 || d == 0 || d > 31 {
                return None;
            }
            Some((y, mo, d))
        }
        [y, mo, d] => {
            // 12.10.2026, day, month, year.
            let (d, mo, y) = (n(y)?, n(mo)?, n(d)?);
            if mo == 0 || mo > 12 || d == 0 || d > 31 {
                return None;
            }
            Some((y, mo, d))
        }
        [d, mo] if tok.contains('.') => {
            let (d, mo) = (n(d)?, n(mo)?);
            if mo == 0 || mo > 12 || d == 0 || d > 31 {
                return None;
            }
            Some((today.0, mo, d))
        }
        _ => None,
    }
}

/// Turns the `when` the model was given into local wall-clock seconds: `+20m`,
/// `90`, `17:30`, `tomorrow 08:00`, `2026-10-12 09:00`, `12.10.2026 09:00`.
/// The error text is written for the model to act on, not for the user.
pub fn parse_when(spec: &str) -> Result<i64, String> {
    // `T` before lowercasing: `2026-10-12T09:00` is what a model often produces.
    let raw = spec.trim().replace('T', " ");
    let text = raw.to_lowercase();
    if text.is_empty() {
        return Err(format!("Brak czasu. {FORMS}"));
    }

    let now = now_local();
    let (ty, tmo, td, _, _, _) = local_fields();

    if let Some(secs) = relative(&text) {
        let at = now + secs;
        return check_range(at, now, spec);
    }

    // "tomorrow"/"today" in both languages, before the date and the clock.
    let mut rest = text.as_str();
    let mut day_offset: Option<i64> = None;
    for (word, off) in [
        ("tomorrow", 1i64),
        ("jutro", 1),
        ("today", 0),
        ("dzisiaj", 0),
        ("dzis", 0),
    ] {
        if let Some(tail) = rest.strip_prefix(word) {
            if tail.is_empty() || tail.starts_with(' ') || tail.starts_with(',') {
                day_offset = Some(off);
                rest = tail.trim_start_matches([',', ' ']);
                break;
            }
        }
    }

    let mut day: Option<(i64, i64, i64)> = None;
    let mut clock: Option<(i64, i64)> = None;
    for tok in rest.split_whitespace() {
        let tok = tok.trim_end_matches(',');
        if tok.is_empty() {
            continue;
        }
        if clock.is_none() {
            if let Some(c) = parse_clock(tok) {
                clock = Some(c);
                continue;
            }
        }
        if day.is_none() {
            if let Some(d) = parse_date(tok, (ty, tmo, td)) {
                day = Some(d);
                continue;
            }
        }
        return Err(format!("Nie rozumiem czasu „{spec}”. {FORMS}"));
    }

    let at = match (day, clock) {
        (Some((dy, dmo, dd)), c) => {
            let (h, mi) = c.unwrap_or((9, 0));
            stamp(dy, dmo, dd, h, mi)
        }
        (None, Some((h, mi))) => {
            let (dy, dmo, dd) = add_days(ty, tmo, td, day_offset.unwrap_or(0));
            let mut s = stamp(dy, dmo, dd, h, mi);
            if s <= now {
                // A bare "18:30" that has already gone means tomorrow's; an explicit
                // "today 18:30" is a mistake the model should hear about.
                if day_offset.is_some() {
                    return Err(format!("Ta godzina już dziś minęła. {FORMS}"));
                }
                let (dy, dmo, dd) = add_days(ty, tmo, td, 1);
                s = stamp(dy, dmo, dd, h, mi);
            }
            s
        }
        (None, None) => return Err(format!("Nie rozumiem czasu „{spec}”. {FORMS}")),
    };

    check_range(at, now, spec)
}

fn check_range(at: i64, now: i64, spec: &str) -> Result<i64, String> {
    if at <= now {
        return Err(format!("„{spec}” już minęło. {FORMS}"));
    }
    if at > now + MAX_AHEAD_DAYS * 86_400 {
        return Err("Tak odległego terminu nie ustawię — maksymalnie rok naprzód.".to_string());
    }
    Ok(at)
}

// ── Firing ────────────────────────────────────────────────────────────────────

/// The card Rust hands to the island when a reminder comes due.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Fired {
    id: u64,
    text: String,
    at: i64,
    /// The due time, already formatted: the card has no business doing calendar
    /// arithmetic, and the two sides must agree on what "now" is.
    at_text: String,
    /// Seconds past due at the moment it was shown, for the log.
    late_seconds: i64,
    missed: bool,
    /// `daily`, `weekdays` or `weekly`, or absent for a one-off, the card says
    /// which, because "codziennie" and "raz" look identical otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat: Option<Repeat>,
}

static LISTENING: AtomicBool = AtomicBool::new(false);

/// When this run of Oczi started, local wall clock. Everything already due at this
/// moment was missed, and the card says so.
static STARTED_AT: OnceLock<i64> = OnceLock::new();

/// Seconds until the next reminder is due, if there is one.
fn next_in(now: i64) -> Option<i64> {
    all().first().map(|r| (r.at - now).max(0))
}

/// The island has its listener wired: hand it whatever came due while Oczi was
/// off, and let the loop start looking. Called from the `reminders_ready` command.
pub fn ready(app: &AppHandle) {
    LISTENING.store(true, Ordering::Relaxed);
    // The one look that also picks up cards a previous run never saw answered.
    fire(app, true);
}

/// Watches the file and shows whatever is due. Started from `lib.rs` next to the
/// other pollers: one place to look is enough for a list this small, and the sleep
/// is cut short when something is due sooner than a tick.
pub fn start(app: AppHandle) {
    let _ = STARTED_AT.set(now_local());
    tauri::async_runtime::spawn(async move {
        loop {
            let wait = match next_in(now_local()) {
                // Due now, or already: look again in a second.
                Some(0) => 1,
                Some(secs) => secs.min(TICK.as_secs() as i64),
                None => TICK.as_secs() as i64,
            };
            tokio::time::sleep(Duration::from_secs(wait.max(1) as u64)).await;
            if LISTENING.load(Ordering::Relaxed) {
                fire(&app, false);
            }
        }
    });
}

fn fire(app: &AppHandle, leftovers: bool) {
    let now = now_local();
    let started = *STARTED_AT.get_or_init(now_local);
    for r in take_due(now, leftovers) {
        crate::log::line(format!(
            "reminder {} fired (due {}, {}s late{})",
            r.id,
            format_local(r.at),
            now - r.at,
            if r.at <= started { ", missed" } else { "" }
        ));
        let payload = Fired {
            id: r.id,
            text: r.text,
            at: r.at,
            at_text: format_local(r.at),
            late_seconds: now - r.at,
            missed: r.at <= started,
            repeat: r.repeat,
        };
        // Nobody hears an event that fails, and a reminder is already off the list
        // by the time this runs: a silent failure would be the end of it.
        if let Err(e) = app.emit_to(crate::island::WINDOW_LABEL, "reminder", payload) {
            crate::log::line(format!("reminder {} could not be shown: {e}", r.id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip() {
        for days in [-20_000i64, -1, 0, 1, 719_468, 20_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "{y}-{m}-{d}");
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // 2026-10-08, checked against the proleptic Gregorian day number.
        assert_eq!(days_from_civil(2026, 10, 8), 20_734);
    }

    #[test]
    fn formats_a_local_stamp() {
        let s = stamp(2026, 10, 9, 8, 0);
        assert_eq!(format_local(s), "2026-10-09 08:00");
        assert_eq!(format_local(s + 59), "2026-10-09 08:00");
        assert_eq!(format_local(s + 60), "2026-10-09 08:01");
        assert_eq!(format_local(s - 1), "2026-10-09 07:59");
    }

    #[test]
    fn relative_durations() {
        assert_eq!(relative("+20m"), Some(1200));
        assert_eq!(relative("90"), Some(5400));
        assert_eq!(relative("1h30m"), Some(5400));
        assert_eq!(relative("in 2 hours"), Some(7200));
        assert_eq!(relative("za 90 minut"), Some(5400));
        assert_eq!(relative("+45s"), Some(45));
        assert_eq!(relative("jutro"), None);
        assert_eq!(relative("18:30"), None);
        assert_eq!(relative("2026-10-12"), None);
        assert_eq!(relative("gdy bede w domu"), None);
    }

    #[test]
    fn a_clock_rolls_to_tomorrow_once_it_has_passed() {
        let now = now_local();
        let at = parse_when("23:59").expect("a clock");
        assert!(at > now && at <= now + 86_400);
    }

    #[test]
    fn reads_the_forms_the_model_is_told_about() {
        for spec in [
            "+20m",
            "90",
            "8:30",
            "jutro 08:00",
            "tomorrow 8:00",
            "2026-12-24 18:00",
            "2026-12-24T18:00",
            "24.12.2026 18:00",
            "24.12 18:00",
            "2026-12-24",
        ] {
            let at = parse_when(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
            assert!(at > now_local(), "{spec} is not in the future");
        }
    }

    #[test]
    fn turns_down_what_it_cannot_read() {
        for spec in ["", "   ", "kiedyś", "25:00", "2026-13-01 10:00", "wczoraj"] {
            assert!(parse_when(spec).is_err(), "{spec} should not parse");
        }
        // A day that has already gone is a mistake, not tomorrow.
        assert!(parse_when("2020-01-01 08:00").is_err());
    }

    #[test]
    fn a_time_is_capped_at_a_year() {
        let now = now_local();
        let (y, m, d) = civil_from_days(now.div_euclid(86_400) + 400);
        let spec = format!("{y:04}-{m:02}-{d:02} 09:00");
        assert!(parse_when(&spec).is_err());
    }

    #[test]
    fn due_and_pending_split_apart() {
        let mk = |id: u64, at: i64| Reminder {
            id,
            text: format!("r{id}"),
            at,
            created: 0,
            fired: false,
            repeat: None,
        };
        // `at <= now` is due, so a reminder set exactly for `now` fires: the two
        // that have come due, the one that has not.
        let (due, rest) = select_due(vec![mk(1, 100), mk(2, 300), mk(3, 200)], 250, false);
        assert_eq!(due.len(), 2);
        assert_eq!(due[0].id, 1);
        assert_eq!(due[1].id, 3);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, 2);

        let (due, _rest) = select_due(vec![mk(1, 200)], 200, false);
        assert_eq!(due.len(), 1, "a reminder due exactly now is due");

        let (due, rest) = select_due(vec![mk(1, 100)], 50, false);
        assert!(due.is_empty());
        assert_eq!(rest.len(), 1);
    }

    #[test]
    fn an_unanswered_card_comes_back_on_the_next_run_and_only_then() {
        let mut shown = Reminder {
            id: 4,
            text: "r4".to_string(),
            at: 100,
            created: 0,
            fired: false,
            repeat: None,
        };
        // It was put on screen and never answered.
        shown.fired = true;

        // Mid-run it must not arrive again on every tick…
        let (due, rest) = select_due(vec![shown.clone()], 400, false);
        assert!(due.is_empty(), "an answered-later card is not re-shown every tick");
        assert_eq!(rest.len(), 1);

        // …but the first look of the next run hands it back.
        let (due, rest) = select_due(vec![shown.clone()], 400, true);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, 4);
        assert!(rest.is_empty());

        // And one that has been answered is simply not in the list any more: the
        // file is the only place it lived.
        assert!(select_due(Vec::new(), 400, true).0.is_empty());
    }

    #[test]
    fn a_daily_reminder_lands_on_the_next_future_one() {
        let day = 86_400;
        let base = days_from_civil(2026, 10, 8) * day + 8 * 3600; // 08:00, a Thursday
        // Answered on the day itself: tomorrow, same clock time.
        assert_eq!(next_after(base, Repeat::Daily, base + 3600), base + day);
        // Answered a week late: not a day that has already gone.
        let now = base + 7 * day + 60;
        let next = next_after(base, Repeat::Daily, now);
        assert!(next > now);
        assert_eq!(next % day, base % day, "the clock time never moves");

        // Weekly keeps the weekday.
        assert_eq!(next_after(base, Repeat::Weekly, base), base + 7 * day);

        // Weekdays skip the weekend: Friday goes to Monday, not Saturday.
        let friday = days_from_civil(2026, 10, 9) * day + 8 * 3600;
        assert!(is_weekend(friday + day), "2026-10-10 is a Saturday");
        assert_eq!(
            next_after(friday, Repeat::Weekdays, friday),
            friday + 3 * day,
            "Friday 08:00 comes back on Monday 08:00"
        );
        let monday = days_from_civil(2026, 10, 12) * day + 8 * 3600;
        assert_eq!(next_after(monday, Repeat::Weekdays, monday), monday + day);
    }

    #[test]
    fn repeat_words_both_languages() {
        assert_eq!(parse_repeat(""), Ok(None));
        assert_eq!(parse_repeat("jednorazowo"), Ok(None));
        assert_eq!(parse_repeat("daily"), Ok(Some(Repeat::Daily)));
        assert_eq!(parse_repeat("codziennie"), Ok(Some(Repeat::Daily)));
        assert_eq!(parse_repeat("dni robocze"), Ok(Some(Repeat::Weekdays)));
        assert_eq!(parse_repeat("weekly"), Ok(Some(Repeat::Weekly)));
        assert!(parse_repeat("co drugi dzień").is_err());
    }
}
