// Now playing: whatever Windows itself says is playing.

use std::sync::mpsc::{RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::AppHandle;
use windows::Foundation::TimeSpan;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::integrations;
use crate::log;

/// How often the player is read. It answers instantly, so this only has to be
/// quick enough to catch a track change; nothing here touches the network.
const EVERY: Duration = Duration::from_secs(2);

/// Transport buttons, from the card, land here and are applied by the media
/// thread, the command itself must never block the window's own thread.
static ACTIONS: OnceLock<Mutex<Option<Sender<String>>>> = OnceLock::new();

/// Called by the `media_control` command. Returns at once; the media thread
/// picks the action up, applies it, and samples again straight away.
pub fn control(action: &str) {
    if let Some(tx) = ACTIONS.get() {
        if let Some(tx) = tx.lock().unwrap().as_ref() {
            let _ = tx.send(action.to_string());
        }
    }
}

/// Starts the sampler. Its own thread: WinRT needs an apartment on the thread
/// that calls it, and this one runs for the life of the app.
pub fn start(app: AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    ACTIONS.get_or_init(|| Mutex::new(Some(tx)));

    std::thread::spawn(move || {
        // MTA: this thread has no message pump, and an STA without one can block.
        if let Err(err) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            log::line(format!("media: WinRT unavailable ({err})"));
            return;
        }

        let mut manager: Option<GlobalSystemMediaTransportControlsSessionManager> = None;
        let mut last: Option<String> = None;
        let mut last_title = String::new();

        loop {
            // Sleeping *on the channel*: a transport button wakes the thread
            // immediately instead of waiting out the interval.
            let action = match rx.recv_timeout(EVERY) {
                Ok(action) => Some(action),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };

            if manager.is_none() {
                manager = connect();
            }
            let Some(player) = manager.as_ref() else {
                continue;
            };

            // Nothing to poll while the pill is off in settings: the memory is
            // dropped so switching it back on reports state at once.
            if !integrations::active(&app, "integration_music") {
                last = None;
                last_title.clear();
                continue;
            }

            if let Some(action) = action {
                apply(player, &action);
            }

            let data = sample(player);
            let playing = data.get("playing").and_then(Value::as_bool).unwrap_or(false);
            // The position is left out of the key on purpose: it moves every poll,
            // and the card runs its own clock between updates rather than having the
            // whole player redrawn twice a second.
            let key = [
                "playing", "status", "title", "artist", "album", "app", "durationSecs",
                "canPrev", "canNext", "canToggle",
            ]
            .iter()
            .map(|field| data.get(*field).map(Value::to_string).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("|");
            if last.as_deref() == Some(key.as_str()) {
                continue;
            }
            last = Some(key);

            let title = data.get("title").and_then(Value::as_str).unwrap_or("");
            let event = (playing && !title.is_empty() && title != last_title).then(|| {
                integrations::IntegrationEvent {
                    success: true,
                    label: title.to_string(),
                    detail: data.get("artist").and_then(Value::as_str).map(str::to_string),
                }
            });
            if !title.is_empty() {
                last_title = title.to_string();
            }

            // Every change, playing or not: a flickering player shows up here as a
            // row of status changes, which is how the stuck celebration was found.
            log::line(format!(
                "media: {} [{}] — {}",
                data.get("app").and_then(Value::as_str).unwrap_or(""),
                data.get("status").and_then(Value::as_str).unwrap_or(""),
                title
            ));

            integrations::emit_update(&app, "integration_music", data, event);
        }
    });
}

/// The session manager. Asking for it is the one call that can fail for good, 
/// no SMTC on the machine at all, so a failure is only logged once.
fn connect() -> Option<GlobalSystemMediaTransportControlsSessionManager> {
    match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()) {
        Ok(manager) => {
            // One line, once: this is the call that proves WinRT came up on this
            // machine, and the only one that can fail for good.
            log::line("media: session manager ready");
            Some(manager)
        }
        Err(err) => {
            log::line(format!("media: no session manager ({err})"));
            None
        }
    }
}

/// The current session, if any player has one. An error here means "no player",
/// which is the ordinary state of a quiet machine.
fn session(player: &GlobalSystemMediaTransportControlsSessionManager) -> Option<GlobalSystemMediaTransportControlsSession> {
    player.GetCurrentSession().ok()
}

fn apply(player: &GlobalSystemMediaTransportControlsSessionManager, action: &str) {
    let Some(session) = session(player) else { return };
    let op = match action {
        "next" => session.TrySkipNextAsync(),
        "prev" => session.TrySkipPreviousAsync(),
        "pause" if is_playing(&session) => session.TryTogglePlayPauseAsync(),
        "play" if !is_playing(&session) => session.TryTogglePlayPauseAsync(),
        "toggle" => session.TryTogglePlayPauseAsync(),
        _ => return,
    };
    if let Ok(op) = op {
        let _ = op.get();
    }
}

fn is_playing(session: &GlobalSystemMediaTransportControlsSession) -> bool {
    status_of(session) == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing
}

fn status_of(session: &GlobalSystemMediaTransportControlsSession) -> GlobalSystemMediaTransportControlsSessionPlaybackStatus {
    session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .unwrap_or(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed)
}

/// Seconds from a 100-nanosecond TimeSpan.
fn secs(t: TimeSpan) -> f64 {
    (t.Duration as f64 / 10_000_000.0).max(0.0)
}

/// Everything the card shows, in one blob. Missing pieces are left out rather
/// than filled with zeros: a stopped player has no title, and saying so beats
/// inventing one.
fn sample(player: &GlobalSystemMediaTransportControlsSessionManager) -> Value {
    let Some(session) = session(player) else {
        return json!({ "available": true, "playing": false, "status": "none" });
    };

    let status = status_of(&session);
    let playing = status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;
    let name = match status.0 {
        1 => "opened",
        2 => "changing",
        3 => "stopped",
        4 => "playing",
        5 => "paused",
        _ => "none",
    };

    let props = session.TryGetMediaPropertiesAsync().ok().and_then(|op| op.get().ok());
    let text = |value: Option<windows::core::HSTRING>| value.map(|h| h.to_string()).unwrap_or_default();
    let title = text(props.as_ref().and_then(|p| p.Title().ok()));
    let artist = text(props.as_ref().and_then(|p| p.Artist().ok()));
    let album = text(props.as_ref().and_then(|p| p.AlbumTitle().ok()));

    let controls = session.GetPlaybackInfo().and_then(|i| i.Controls()).ok();
    let can = |value: Option<bool>| value.unwrap_or(false);
    let (mut position, mut duration) = (0.0, 0.0);
    if let Ok(timeline) = session.GetTimelineProperties() {
        position = timeline.Position().map(secs).unwrap_or(0.0);
        duration = timeline.EndTime().map(secs).unwrap_or(0.0);
    }

    json!({
        "available": true,
        "playing": playing,
        "status": name,
        "title": title,
        "artist": artist,
        "album": album,
        "app": session.SourceAppUserModelId().map(|h| player_name(&h.to_string())).unwrap_or_default(),
        "positionSecs": position,
        "durationSecs": duration,
        "canPrev": can(controls.as_ref().and_then(|c| c.IsPreviousEnabled().ok())),
        "canNext": can(controls.as_ref().and_then(|c| c.IsNextEnabled().ok())),
        "canToggle": can(controls.as_ref().and_then(|c| c.IsPlayPauseToggleEnabled().ok())),
    })
}

/// The OS hands over an app user model id or a process name, `Spotify.exe`, but
/// also `Music.youtube.com-5929F88E_…!App` for a packaged app. The card needs
/// something a person recognises.
fn player_name(source: &str) -> String {
    let head = source.split('!').next().unwrap_or(source);
    let head = head.rsplit(['\\', '/']).next().unwrap_or(head);
    let stem = head.trim_end_matches(".exe").trim_end_matches(".EXE");
    let lower = stem.to_lowercase();

    let known = [
        ("youtube", "YouTube Music"),
        ("spotify", "Spotify"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("vlc", "VLC"),
        ("foobar", "foobar2000"),
        ("musicbee", "MusicBee"),
        ("aimp", "AIMP"),
        ("winamp", "Winamp"),
        ("zune", "Groove"),
        ("groove", "Groove"),
    ];
    for (needle, label) in known {
        if lower.contains(needle) {
            return label.to_string();
        }
    }

    // "<name>-<hash>" is a package family name: the hash is not something to read.
    let name = match stem.rsplit_once('-') {
        Some((before, tail))
            if tail.len() > 6 && tail.chars().any(|c| c == '_' || c.is_ascii_digit()) =>
        {
            before
        }
        _ => stem,
    };
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}
