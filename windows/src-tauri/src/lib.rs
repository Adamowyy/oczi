// Oczi, app wiring and the commands the island calls.

mod clean;
mod clipboard;
mod debloat;
mod deepseek;
mod elevate;
mod files;
mod gamebar;
mod health;
mod integrations;
mod island;
mod log;
mod media;
mod ocr;
mod pc;
mod pin;
mod prices;
mod providers;
mod registry;
mod reminders;
mod routines;
mod secrets;
mod settings;
mod shell;
mod snip;
mod startup;
mod steam;
mod text_tools;
mod tray;
mod tweaks;
mod update;
mod util;
#[cfg(feature = "voice")]
mod voice;
mod web;
mod webview_guard;
mod youtube;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use deepseek::{Chat, ChatContext, ChatReply, Options};
use files::DroppedFile;
use island::{PollGate, ScreenInfo};
use settings::Settings;
use snip::Snip;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
}

/// The window handle, for the modules that reach the island from a place that has none — a tool
/// call runs with nothing but its arguments, and the price card is sent from exactly there.
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// The app handle, once `setup` has run. Before that there is no island to send anything to.
pub fn app() -> Option<AppHandle> {
    APP.get().cloned()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootInfo {
    settings: Settings,
    screen: ScreenInfo,
    version: String,
}

#[tauri::command]
fn boot(app: AppHandle, shared: State<Shared>) -> BootInfo {
    let settings = shared.settings.lock().unwrap().clone();
    let screen = island::screen_info(&app, &settings.screen);
    BootInfo {
        settings,
        screen,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// The displays the settings window offers to pin the island to. Read live, so a
/// display plugged in after launch shows up the next time settings is opened.
#[tauri::command]
fn list_monitors(app: AppHandle) -> Vec<island::MonitorInfo> {
    island::list_monitors(&app)
}

/// The price pill's own lookup: Steam finds the game, gg.deals prices it, the card is the answer.
#[tauri::command]
async fn prices_lookup(title: String) -> Result<prices::Offer, String> {
    prices::lookup(&title).await
}

/// Is there a newer release than the one running? Called once per launch, see
/// the module comment for why nothing else ever touches the network.
#[tauri::command]
async fn check_update() -> Option<update::Release> {
    update::latest(env!("CARGO_PKG_VERSION")).await
}

// Remembers that this version's card has been shown, so it appears once.
#[tauri::command]
fn mark_version_seen(shared: State<Shared>, version: String) {
    let snapshot = {
        let mut current = shared.settings.lock().unwrap();
        if current.last_seen_version == version {
            return;
        }
        current.last_seen_version = version;
        current.clone()
    };
    if let Err(err) = settings::save(&snapshot) {
        log::line(format!("could not save the seen version: {err}"));
    }
}

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, mut settings: Settings) {
    // The provider and the model have to agree, and the island reads them straight out of
    // this struct: a mismatched pair saved once would 400 on every chat until the next load.
    settings::sanitise(&mut settings);
    let (
        screen_changed,
        autostart_changed,
        hotkey_changed,
        snip_hotkey_changed,
        voice_hotkey_changed,
        pin_hotkey_changed,
        language_changed,
    ) = {
        let mut current = shared.settings.lock().unwrap();
        // The island owns this one: the settings window's copy is as old as the
        // window is, and writing it back would re-show the card on the next launch.
        settings.last_seen_version = current.last_seen_version.clone();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        let hotkey_changed = current.hotkey != settings.hotkey;
        let snip_hotkey_changed = current.snip_hotkey != settings.snip_hotkey;
        let voice_hotkey_changed = current.voice_hotkey != settings.voice_hotkey;
        let pin_hotkey_changed = current.pin_hotkey != settings.pin_hotkey;
        let language_changed = current.language != settings.language;
        *current = settings.clone();
        (
            screen_changed,
            autostart_changed,
            hotkey_changed,
            snip_hotkey_changed,
            voice_hotkey_changed,
            pin_hotkey_changed,
            language_changed,
        )
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[oczi] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart {
            manager.enable()
        } else {
            manager.disable()
        };
        if let Err(err) = result {
            eprintln!("[oczi] autostart: {err}");
        }
    }
    if screen_changed {
        island::apply_geometry(&app, &settings.screen, settings.island_anchor);
    }
    if hotkey_changed {
        island::update_hotkey(island::HotkeySlot::Summon, &settings.hotkey);
    }
    if snip_hotkey_changed {
        island::update_hotkey(island::HotkeySlot::Snip, &settings.snip_hotkey);
    }
    if pin_hotkey_changed {
        island::update_hotkey(island::HotkeySlot::Pin, &settings.pin_hotkey);
    }
    // Only the voice build has anything to start, and only it registers the chord: the
    // ordinary build stores the setting and leaves the keys to whoever had them.
    #[cfg(feature = "voice")]
    if voice_hotkey_changed {
        island::update_hotkey(island::HotkeySlot::Voice, &settings.voice_hotkey);
    }
    #[cfg(not(feature = "voice"))]
    let _ = voice_hotkey_changed;
    // The tray and the window frame are drawn by the system, so they take their
    // language from here rather than from the front end.
    if language_changed {
        tray::set_language(&app, &settings.language);
        if let Some(win) = app.get_webview_window("settings") {
            let _ = win.set_title(settings_title(&settings.language));
        }
    }
    // Keep the other window in step (island ⇄ settings window).
    let _ = app.emit("settings-changed", settings);
}

/// True while the island is retracted into the top edge: tells the cursor poll to
/// idle and starts answering drags and hovers on the wake band.
#[tauri::command]
fn set_collapsed(app: AppHandle, shared: State<Shared>, collapsed: bool) {
    shared.gate.collapsed.store(collapsed, Ordering::Relaxed);
    shared.gate.forget_ignore_state();
    let _ = app;
}

/// Whether a fullscreen app owns the island's display right now. The cursor poll pushes
/// a change as an event; this answers for a page that has just booted and missed it.
#[tauri::command]
fn island_fullscreen(app: AppHandle) -> bool {
    island::detect_fullscreen(&app)
}

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect {
        x,
        y,
        w: width,
        h: height,
    });
}

/// Files are only taken while the upload flow (the plus tab) is on screen.
#[tauri::command]
fn set_accept_drops(shared: State<Shared>, accept: bool) {
    shared.gate.accept_drops.store(accept, Ordering::Relaxed);
}

/// Gives the island window keyboard focus so its chat field can be typed in, and
/// takes the flag away again when the user leaves the chat.
#[tauri::command]
fn focus_window(app: AppHandle, focused: bool) {
    let Some(win) = island::window(&app) else {
        return;
    };
    island::set_activating(&win, focused);
    if focused {
        // island::focus, not a bare set_focus: Windows only grants the foreground
        // to a process that already has it, so a summon sent while another app was
        // in front would take the caret and never give it the keyboard.
        island::focus(&app);
    }
}

#[tauri::command]
fn reposition(app: AppHandle, shared: State<Shared>) {
    let (pref, anchor) = {
        let s = shared.settings.lock().unwrap();
        (s.screen.clone(), s.island_anchor)
    };
    island::apply_geometry(&app, &pref, anchor);
}

/// Dragging the island: it slides, and the place it was left in is written down so
/// it comes back there after a restart.
#[tauri::command]
fn set_island_anchor(app: AppHandle, shared: State<Shared>, anchor: f64, persist: bool) {
    let anchor = anchor.clamp(0.0, 1.0);
    let (pref, mut settings) = {
        let s = shared.settings.lock().unwrap();
        (s.screen.clone(), s.clone())
    };
    island::set_anchor(&app, &pref, anchor);
    if persist {
        settings.island_anchor = anchor;
        *shared.settings.lock().unwrap() = settings.clone();
        if let Err(err) = settings::save(&settings) {
            log::line(format!("could not save the island position: {err}"));
        }
    }
}

#[tauri::command]
fn set_intervals(shared: State<Shared>, auto_close: Option<f64>, notch_hide: Option<f64>) {
    let mut settings = shared.settings.lock().unwrap();
    if let Some(seconds) = auto_close {
        settings.auto_close_interval = seconds;
    }
    if let Some(seconds) = notch_hide {
        settings.notch_hide_interval = seconds;
    }
    let snapshot = settings.clone();
    drop(settings);
    if let Err(err) = settings::save(&snapshot) {
        log::line(format!("could not save the island intervals: {err}"));
    }
}

#[tauri::command]
fn open_url(url: String) {
    // One route for both sides — a link on a card and the model's own open_url tool go
    // through the same call, so they open in the same browser the same way. The error is
    // ignored here on purpose: a link that cannot be opened must not raise a dialog.
    let _ = shell::open_url(&url);
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Tray → Pause. Paused means paused: the pollers stop talking to the network,
/// not just the island stopping showing things.
#[tauri::command]
fn set_paused(paused: bool) {
    integrations::set_paused(paused);
}

// ── Chat, files and secrets ───────────────────────────────────────────────────

/// One chat turn. The API key and any file bytes stay on the Rust side.
#[tauri::command]
async fn chat_send(
    shared: State<'_, Shared>,
    chat: State<'_, Chat>,
    snip: State<'_, Snip>,
    query: String,
    context: Option<ChatContext>,
) -> Result<ChatReply, String> {
    let (model, provider_id, thinking, web_search, search_provider, terminal, language) = {
        let settings = shared.settings.lock().unwrap();
        (
            settings.model.clone(),
            settings.provider.clone(),
            settings.thinking,
            settings.web_search,
            settings.search_provider.clone(),
            settings.terminal_enabled,
            settings.language.clone(),
        )
    };
    // A pending screenshot is read off disk here, once, and only for the turn it was
    // taken for: the front end sends the image context exactly while a shot is pinned,
    // so a shot taken mid-conversation still travels with the question it was for.
    let screenshot = match &context {
        Some(ChatContext::Image { .. }) => {
            snip.take().and_then(|path| match snip::data_url(&path) {
                Ok(url) => Some(url),
                Err(err) => {
                    log::line(format!("snip  could not attach the screenshot: {err}"));
                    None
                }
            })
        }
        _ => None,
    };
    deepseek::send(
        &chat,
        Options {
            model: &model,
            thinking,
            llm: &provider_id,
            web: web_search,
            provider: &search_provider,
            terminal,
            language: &language,
        },
        query,
        context,
        screenshot,
    )
    .await
}

/// Puts the overlay away and brings the island back. Every exit from the snip flow
/// goes through here, cancelled or not, so the island can never be left hidden.
fn dismiss_snip(app: &AppHandle) {
    if let Some(overlay) = app.get_webview_window("snip") {
        let _ = overlay.hide();
    }
    island::show(app);
}

/// Where the overlay sits and how big its CSS pixels are. The frame is physical;
/// the overlay reports CSS pixels, so both are needed to crop the right box.
fn overlay_metrics(app: &AppHandle) -> (f64, f64, f64) {
    let Some(overlay) = app.get_webview_window("snip") else {
        return (1.0, 0.0, 0.0);
    };
    let scale = overlay.scale_factor().unwrap_or(1.0);
    let (x, y) = overlay
        .outer_position()
        .map(|p| (p.x as f64, p.y as f64))
        .unwrap_or((0.0, 0.0));
    (scale, x, y)
}

// The eye, step one: freeze the desktop, then put the overlay on screen. `pin_mode` is what the pin
// chord passes; `finish_snip` reads and clears it.
#[tauri::command]
async fn begin_snip(app: AppHandle, pin_mode: Option<bool>) -> Result<(), String> {
    if pin_mode.unwrap_or(false) {
        pin::start_pin_mode();
    }
    if let Some(win) = island::window(&app) {
        let _ = win.hide();
    }
    let grabbing = app.clone();
    let frozen = tauri::async_runtime::spawn_blocking(move || {
        // The pause gives the compositor time to really drop the island before the
        // pixels are read.
        std::thread::sleep(std::time::Duration::from_millis(220));
        let snip = grabbing.state::<Snip>();
        snip::grab(&snip)
    })
    .await
    .map_err(|e| format!("the screen grab crashed: {e}"))?;

    if let Err(err) = frozen {
        log::line(format!("snip  could not grab the screen: {err}"));
        island::show(&app);
        return Err(err);
    }

    let Some(overlay) = app.get_webview_window("snip") else {
        log::line("snip  overlay window is missing".to_string());
        island::show(&app);
        return Err("the selection overlay is missing".into());
    };
    let _ = overlay.show();
    let _ = overlay.set_focus();
    Ok(())
}

/// The eye, step two: crop what was dragged and tell the island about it.
#[tauri::command]
fn finish_snip(
    app: AppHandle,
    snip: State<Snip>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let (scale, origin_x, origin_y) = overlay_metrics(&app);
    // The overlay's rectangles are relative to the overlay, the frame's are relative
    // to the whole desktop, and the desktop does not start at 0 when a second
    // monitor sits to the left: without this the crop would be off by its width.
    let (vx, vy, ..) = snip::desktop_bounds();
    let cx = (origin_x - vx as f64 + x * scale).round() as i32;
    let cy = (origin_y - vy as f64 + y * scale).round() as i32;
    let cw = (width * scale).round() as i32;
    let ch = (height * scale).round() as i32;

    // A pin and a question leave the same overlay, and the difference is the whole of the
    // flow: a pin keeps its crop on screen, a question hands its crop to the chat.
    if pin::take_mode() {
        // The window is picked before the crop is written: the file is named after the slot, and
        // each pin keeps its own picture, so a new one never eats the pin already on screen.
        let index = pin::reserve();
        let (info, _path) = snip::finish_pin(&snip, cx, cy, cw, ch, index)?;
        log::line(format!(
            "pin  {} cut {}x{} ({} bytes)",
            pin::label(index),
            info.width,
            info.height,
            info.bytes
        ));
        // The pin goes back exactly over the pixels it was cut from, in desktop
        // coordinates — the space both the crop and a window position speak.
        if let Err(err) = pin::show(&app, index, vx + cx, vy + cy, cw, ch) {
            log::line(format!("pin  could not show: {err}"));
        }
        dismiss_snip(&app);
        return Ok(());
    }

    let info = snip::finish(&snip, cx, cy, cw, ch)?;
    log::line(format!(
        "snip  {}x{} ({} bytes)",
        info.width, info.height, info.bytes
    ));
    let _ = app.emit_to(island::WINDOW_LABEL, "snip-done", info);
    dismiss_snip(&app);
    Ok(())
}

/// Esc, right-click, or a click with no drag.
#[tauri::command]
fn cancel_snip(app: AppHandle, snip: State<Snip>) {
    snip.discard_frame();
    log::line("snip  cancelled".to_string());
    let _ = app.emit_to(island::WINDOW_LABEL, "snip-done", snip::SnipInfo::CANCELLED);
    dismiss_snip(&app);
}

/// Which pinned window a command is about; an unknown label is an error.
fn pin_slot(label: &str) -> Result<usize, String> {
    pin::index_of(label).ok_or_else(|| format!("there is no pin called {label}"))
}

/// The press that starts moving a pin; the loop ends with the button.
#[tauri::command]
fn pin_drag_start(app: AppHandle, label: String) {
    if let Some(index) = pin::index_of(&label) {
        pin::start_drag(&app, index);
    }
}

/// The text a pin shows, through OCR, copied. Async: the engine works on a worker thread.
#[tauri::command]
async fn pin_copy_text(label: String) -> Result<String, String> {
    let index = pin_slot(&label)?;
    tauri::async_runtime::spawn_blocking(move || pin::copy_text(index))
        .await
        .map_err(|e| format!("the text reader crashed: {e}"))?
}

#[tauri::command]
fn pin_copy_image(label: String) -> Result<(), String> {
    pin::copy_image(pin_slot(&label)?)
}

/// Windows' own save dialog, then the file is written where the user pointed.
#[tauri::command]
fn pin_save_as(label: String) -> Result<Option<String>, String> {
    pin::save_as(pin_slot(&label)?)
}

/// Ask the chat about the pin without taking it off the screen.
#[tauri::command]
fn pin_ask(app: AppHandle, snip: State<Snip>, label: String) -> Result<(), String> {
    pin::ask(&app, &snip, pin_slot(&label)?)
}

/// A wheel step over the pin: bigger or smaller, around its own middle.
#[tauri::command]
fn pin_scale(app: AppHandle, label: String, step: i32) -> Result<(), String> {
    pin::scale(&app, pin_slot(&label)?, step)
}

/// Makes room for the menu; replies with the picture's offset in CSS pixels.
#[tauri::command]
fn pin_menu(
    app: AppHandle,
    label: String,
    open: bool,
    need_w: f64,
    need_h: f64,
) -> Result<(f64, f64), String> {
    pin::menu(&app, pin_slot(&label)?, open, need_w, need_h)
}

/// The annotated picture, or nothing when the drawing was cleared. The page composites it, since
/// the strokes live on its canvas; Rust only stores what it is given.
#[tauri::command]
fn pin_mark(label: String, data: String) -> Result<(), String> {
    let index = pin_slot(&label)?;
    if data.trim().is_empty() {
        return pin::set_marked(index, None);
    }
    let encoded = data
        .split_once("base64,")
        .map(|(_, rest)| rest)
        .unwrap_or(&data);
    let bytes = crate::util::base64_from(encoded)
        .ok_or_else(|| "the drawing came back unreadable".to_string())?;
    pin::set_marked(index, Some(bytes))
}

/// Above everything, or an ordinary window — the toggle beside the ×.
#[tauri::command]
fn pin_topmost(app: AppHandle, label: String, on: bool) -> Result<(), String> {
    pin::set_topmost(&app, pin_slot(&label)?, on)
}

/// Closing every pin at once, from the menu.
#[tauri::command]
fn pin_close_all(app: AppHandle) {
    pin::close_all(&app);
}

/// Closing a pin. The file goes with the window.
#[tauri::command]
fn pin_close(app: AppHandle, label: String) {
    if let Some(index) = pin::index_of(&label) {
        pin::close(&app, index);
    }
}

#[tauri::command]
fn chat_reset(chat: State<Chat>) {
    chat.reset();
}

/// Copies a dropped file into the inbox and reports its name back.
#[tauri::command]
fn ingest_file(path: String) -> Result<DroppedFile, String> {
    files::ingest(&path)
}

/// The island may only ask whether a key exists, never read it.
#[tauri::command]
fn secret_present(key: String) -> bool {
    secrets::present(&key)
}

#[tauri::command]
fn secret_set(key: String, value: String) -> Result<(), String> {
    secrets::set(&key, &value)
}

/// A setting, not a secret, is shown back to the window: a masked region code tells nobody
/// anything. `secrets::readable` decides which keys those are.
#[tauri::command]
fn secret_value(key: String) -> Option<String> {
    secrets::readable(&key)
}

#[tauri::command]
fn secret_clear(key: String) -> Result<(), String> {
    secrets::clear(&key)
}

/// Opens the configured n8n instance, the URL lives in the Credential Manager.
#[tauri::command]
fn open_n8n() {
    if let Some(url) = secrets::get("n8n-url") {
        open_url(url);
    }
}

// ── Reminders ─────────────────────────────────────────────────────────────────

/// What is still pending. Read from the file every time, so the list is right even
/// when the file was edited by hand, there is no copy of it held in memory.
#[tauri::command]
fn reminders_list() -> Vec<reminders::Reminder> {
    reminders::all()
}

/// Set one without a chat turn, the model has its own tools for this; the island
/// uses it for the "+10 min" button on a card, which is just a new reminder.
#[tauri::command]
fn reminder_add(
    text: String,
    at: String,
    repeat: Option<String>,
) -> Result<reminders::Reminder, String> {
    let when = reminders::parse_when(&at)?;
    let repeat = reminders::parse_repeat(repeat.as_deref().unwrap_or(""))?;
    reminders::add(&text, when, repeat)
}

#[tauri::command]
fn reminder_cancel(id: u64) -> Result<bool, String> {
    reminders::remove(id)
}

/// A card on screen has been answered. For a one-off that is the end of it; for a
/// repeating one this moves it to its next occurrence and returns when that is.
/// Until this arrives, the entry stays and is handed back on the next launch.
#[tauri::command]
fn reminder_done(id: u64) -> Result<Option<i64>, String> {
    reminders::answered(id)
}

/// The island has wired its listener. Nothing is shown before this arrives: an
/// event emitted while the page was still loading had nobody to hear it, and the
/// reminder was already gone from the list by then.
#[tauri::command]
fn reminders_ready(app: AppHandle) {
    reminders::ready(&app);
}

// ── Routines ──────────────────────────────────────────────────────────────────

/// What Oczi will do by itself, read from the file every time, so a routine edited by
/// hand is still right when the window opens.
#[tauri::command]
fn routines_list() -> Vec<routines::Routine> {
    routines::all()
}

#[tauri::command]
fn routine_cancel(id: u64) -> Result<bool, String> {
    routines::remove(id)
}

/// The report card has been read. A one-off is finished and leaves the list; a repeating
/// routine keeps its place, and only its report is dropped.
#[tauri::command]
fn routine_done(id: u64) -> Result<bool, String> {
    routines::answered(id)
}

/// The island has wired its listener, so a report may be pushed from now on. Anything
/// reported while Oczi was not listening — a run whose card was lost to a crash, a
/// restart or a shutdown — is handed over in this same call.
#[tauri::command]
fn routines_ready(app: AppHandle) {
    routines::ready(&app);
}

/// Run one on the spot, without waiting for its time. The card reports it like any other
/// run, so nothing happens silently.
#[tauri::command]
fn routine_run_now(app: AppHandle, id: u64) -> Result<(), String> {
    routines::run_now(&app, id)
}

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
}

/// Transport buttons on the now-playing card. Queued to the media thread, so the
/// window's own thread never waits on a WinRT call.
#[tauri::command]
fn media_control(action: String) {
    media::control(&action);
}

/// Lets the island write to the same log as the Rust side.
#[tauri::command]
fn log_line(message: String) {
    log::line(format!("ui  {message}"));
}

// ── Settings window ───────────────────────────────────────────────────────────

/// WebView2 allows one browser environment per app with fixed options, so every
/// window must pass the same args as the island (tauri.conf.json) or come up blank.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// In a dev build the pages are served by Vite, so the second window needs the
/// absolute dev URL; a bundled build resolves it inside the app bundle.
fn settings_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/settings.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("settings.html".into())
}

fn settings_title(language: &str) -> &'static str {
    if language == "pl" {
        // i18n-ok: the Polish branch, picked at run time from the UI language.
        "Ustawienia — Oczi"
    } else {
        "Settings — Oczi"
    }
}

/// A page inside the bundled frontend, or the dev server while running with `dev`.
fn page_url(app: &AppHandle, page: &str) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path(&format!("/{page}"));
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App(page.into())
}

/// The health report and the debloat list: two windows of their own, created hidden at launch and
/// only ever shown — the same reason as the settings window, opening one has to feel instant.
fn create_pc_windows(app: &AppHandle) {
    for (label, page) in [("health", "health.html"), ("debloat", "debloat.html")] {
        match WebviewWindowBuilder::new(app, label, page_url(app, page))
            .additional_browser_args(BROWSER_ARGS)
            .title("Oczi")
            .inner_size(600.0, 720.0)
            .min_inner_size(460.0, 420.0)
            .resizable(true)
            .focused(false)
            .visible(false)
            .center()
            .build()
        {
            Ok(win) => {
                // Closing must only hide it, or it could never be reopened.
                let hidden = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hidden.hide();
                    }
                });
            }
            Err(err) => log::line(format!("{label} window failed: {err}")),
        }
    }
}

fn create_settings_window(app: &AppHandle, settings: &Settings) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title(settings_title(&settings.language))
        .inner_size(560.0, 680.0)
        .min_inner_size(460.0, 480.0)
        .resizable(true)
        // Created hidden and unfocused: showing it is a user action, and until then it
        // must not take the keyboard away from whatever they were doing at launch.
        .focused(false)
        .visible(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("settings") else {
        log::line("settings window missing");
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    // The page lives on between visits, so tell it to re-read the monitors: one
    // plugged in since launch would otherwise never appear in the picker.
    let _ = win.emit("settings-shown", ());
}

fn snip_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/snip.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("snip.html".into())
}

/// The selection overlay: created at launch and only ever shown and hidden, for the
/// same reason as the settings window. It covers the whole virtual desktop, fetched
/// from Windows in physical pixels and handed to the builder in logical ones.
fn create_snip_window(app: &AppHandle) {
    let url = snip_page_url(app);
    let (x, y, w, h) = snip::desktop_bounds();
    let scale = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);
    match WebviewWindowBuilder::new(app, "snip", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Snip — Oczi")
        .decorations(false)
        .resizable(false)
        .transparent(true)
        .shadow(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .position(x as f64 / scale, y as f64 / scale)
        .inner_size(w as f64 / scale, h as f64 / scale)
        // Hidden and unfocused at launch: a window that is never shown must not take
        // the foreground away from whatever the user was typing in.
        .focused(false)
        .visible(false)
        .build()
    {
        Ok(_) => log::line(format!(
            "snip overlay ready — desktop {x},{y} {w}x{h} @{scale}x"
        )),
        Err(err) => log::line(format!("snip overlay failed: {err}")),
    }
}

fn pin_page_url(app: &AppHandle) -> WebviewUrl {
    #[cfg(dev)]
    if let Some(mut base) = app.config().build.dev_url.clone() {
        base.set_path("/pin.html");
        return WebviewUrl::External(base);
    }
    let _ = app;
    WebviewUrl::App("pin.html".into())
}

/// The pins' windows: created at launch and only ever shown and hidden, one per slot.
fn create_pin_windows(app: &AppHandle) {
    for index in 0..pin::SLOTS {
        let label = pin::label(index);
        match WebviewWindowBuilder::new(app, label.clone(), pin_page_url(app))
            .additional_browser_args(BROWSER_ARGS)
            .title("Pin — Oczi")
            .decorations(false)
            .resizable(false)
            .transparent(true)
            .shadow(false)
            .skip_taskbar(true)
            .always_on_top(true)
            .inner_size(320.0, 200.0)
            // Hidden and unfocused at launch: nothing may take the foreground from what the user
            // was typing in, and a pin only appears when one is asked for.
            .focused(false)
            .visible(false)
            .build()
        {
            Ok(_) => log::line(format!("pin window {label} ready")),
            Err(err) => log::line(format!("pin window {label} failed: {err}")),
        }
    }
}

/// A health scan. One pass, on demand: a few seconds of load samples plus the event log, which is
/// the slow half. It runs on a worker thread because a couple of seconds of reading the log on the
/// main thread is a frozen window — and nothing of it survives the call.
#[tauri::command]
async fn health_scan(deep: bool) -> health::Report {
    let mut report = tauri::async_runtime::spawn_blocking(move || health::scan(deep))
        .await
        .unwrap_or_default();
    // A deep scan can be read, not just measured: the model is shown the findings and says which
    // one matters. It costs a call, so it happens for the pass the user asked to be deeper — and a
    // missing key or a failed call simply leaves the report without the extra paragraph.
    if deep {
        match deepseek::advice(&health::describe(&report)).await {
            Ok(text) => report.advice = text,
            Err(e) => log::line(format!("health advice skipped: {e}")),
        }
    }
    health::show(&report);
    report
}

/// One watch for a freeze: samples the machine once a second for `minutes` and reports the moments
/// it stopped. Bounded by design — the loop ends by itself and leaves nothing running.
#[tauri::command]
async fn health_watch(minutes: u32) -> health::Watch {
    tauri::async_runtime::spawn_blocking(move || health::watch(minutes))
        .await
        .unwrap_or_default()
}

/// Stops the watch that is running. The loop ends between samples and returns what it saw.
#[tauri::command]
fn health_watch_cancel() {
    health::cancel();
}

/// The last watch ever run, so a freeze report survives a restart.
#[tauri::command]
fn health_last_watch() -> Option<health::Watch> {
    health::last_watch()
}

/// The verdict from the last scan, for the hub's stats line. `None` until the first one ever.
#[tauri::command]
fn health_last() -> Option<health::Report> {
    health::last()
}

/// The one place a scan can change something, and only for the action the user picked after
/// reading the finding.
#[tauri::command]
fn health_fix(action: String, arg: String) -> Result<String, String> {
    health::fix(&action, &arg)
}

/// The packages Windows ships that the catalogue is willing to offer, as installed right now.
/// The Appx cmdlets take a second or two, so this runs off the main thread like the scan.
#[tauri::command]
async fn debloat_list() -> Result<Vec<debloat::AppItem>, String> {
    tauri::async_runtime::spawn_blocking(debloat::list)
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Removes the packages the user ticked, then reports what Windows says is actually gone.
#[tauri::command]
async fn debloat_remove(names: Vec<String>) -> Result<debloat::Removal, String> {
    tauri::async_runtime::spawn_blocking(move || debloat::remove(&names))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// What this app has taken away, newest first — the window's history.
#[tauri::command]
fn debloat_history() -> Vec<debloat::Removed> {
    debloat::removed_history()
}

/// Game Bar and the capture switches, as Windows has them right now. Two registry reads and one
/// Appx call, none of which this app has to have an opinion about first.
#[tauri::command]
async fn gamebar_status() -> Result<gamebar::GameBar, String> {
    tauri::async_runtime::spawn_blocking(gamebar::status)
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Turns Game DVR capture on or off at both levels and answers with a fresh reading, so the window
/// shows what Windows has rather than what was asked for.
#[tauri::command]
async fn gamebar_capture(on: bool) -> Result<gamebar::GameBar, String> {
    tauri::async_runtime::spawn_blocking(move || gamebar::set_capture(on))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// The same, for the machine level, through the one UAC prompt Windows insists on. Blocks until
/// the registry shows the change, so a refusal comes back as a refusal.
#[tauri::command]
async fn gamebar_policy_elevated(on: bool) -> Result<gamebar::GameBar, String> {
    tauri::async_runtime::spawn_blocking(move || gamebar::policy_elevated(on))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Puts Game Bar back from the Microsoft Store. `winget` can take a minute, so it runs off the
/// main thread like the removal does.
#[tauri::command]
async fn gamebar_install() -> Result<gamebar::GameBar, String> {
    tauri::async_runtime::spawn_blocking(gamebar::install)
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

// ── The tweaks tab ────────────────────────────────────────────────────────────
// Thirteen registry values, one table, and the same four commands in every language: read them,
// set one, reset one, put back what was there. Each answers with the whole table re-read, so the
// window replaces its list instead of merging an answer into it.

/// Every tweak as Windows has it now.
#[tauri::command]
async fn tweaks_list() -> Vec<tweaks::TweakState> {
    tauri::async_runtime::spawn_blocking(tweaks::list)
        .await
        .unwrap_or_default()
}

/// One switch, both directions: `on` is the quiet state whatever number Windows spells it with.
#[tauri::command]
async fn tweak_set(id: String, on: bool) -> Result<Vec<tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(move || tweaks::set(&id, on))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// A row with more than two states: the number is the one the table lists.
#[tauri::command]
async fn tweak_choose(id: String, value: u32) -> Result<Vec<tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(move || tweaks::set_value(&id, value))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Back to Windows' own default — the value is deleted, not guessed.
#[tauri::command]
async fn tweak_reset(id: String) -> Result<Vec<tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(move || tweaks::reset(&id))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// The same write through one UAC prompt, for a row that lives in the machine hive. Blocks until
/// the registry shows the change, so a refused prompt comes back as a refusal.
#[tauri::command]
async fn tweak_elevated(id: String, on: bool) -> Result<Vec<tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(move || tweaks::set_elevated(&id, on))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Puts back the value the last change replaced, and forgets that change.
#[tauri::command]
async fn tweak_undo(id: String) -> Result<Vec<tweaks::TweakState>, String> {
    tauri::async_runtime::spawn_blocking(move || tweaks::undo(&id))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

/// Everything this app changed, newest first.
#[tauri::command]
fn tweaks_history() -> Vec<tweaks::Change> {
    tweaks::changes()
}

/// Whether this process can write the machine hive — the window asks before promising a change.
#[tauri::command]
fn tweaks_elevated_probe() -> bool {
    tweaks::is_elevated()
}

/// Restarts Explorer, which half of these settings only read at startup. Only ever on request.
#[tauri::command]
async fn restart_explorer() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(tweaks::restart_explorer)
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
}

// ── The autostart tab ─────────────────────────────────────────────────────────

/// Everything Windows would start at logon, switched or not.
#[tauri::command]
async fn startup_list() -> Vec<startup::Entry> {
    tauri::async_runtime::spawn_blocking(startup::list)
        .await
        .unwrap_or_default()
}

/// Flips one entry the way Task Manager does, then re-reads the whole list.
#[tauri::command]
async fn startup_set(name: String, on: bool) -> Result<Vec<startup::Entry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        startup::set(&name, on)?;
        Ok(startup::list())
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()))
}

// ── The cleaning tab ──────────────────────────────────────────────────────────

/// How much is sitting in the temp folder and the Recycle Bin right now.
#[tauri::command]
async fn clean_scan() -> clean::Sizes {
    tauri::async_runtime::spawn_blocking(clean::scan)
        .await
        .unwrap_or_default()
}

/// Empties both, then measures again: the report is the difference, not the intention.
#[tauri::command]
async fn clean_run() -> clean::Freed {
    tauri::async_runtime::spawn_blocking(clean::run)
        .await
        .unwrap_or_default()
}

/// Shows one of the two PC windows. Both exist from launch and are only ever hidden, so opening
/// one is instant and the page keeps whatever the last scan left in it.
#[tauri::command]
fn open_pc_window(app: AppHandle, which: String) {
    if !matches!(which.as_str(), "health" | "debloat") {
        log::line(format!("open_pc_window: unknown window {which}"));
        return;
    }
    let Some(win) = app.get_webview_window(&which) else {
        log::line(format!("{which} window missing"));
        return;
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    // The page lives on between visits; tell it to re-read what it shows.
    let _ = win.emit("pc-shown", ());
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

/// The chord the dictation slot should register, or `None` in a build without the engine:
/// a key combination nobody can act on is worse than no key at all — it would be taken
/// away from whatever else the user meant it for.
#[cfg(feature = "voice")]
fn voice_chord(settings: &Settings) -> Option<String> {
    Some(settings.voice_hotkey.clone())
}

#[cfg(not(feature = "voice"))]
fn voice_chord(_settings: &Settings) -> Option<String> {
    None
}

/// One dictation: 16 kHz mono floats as raw bytes in, the transcript out. The island
/// records and knows what silence means; this only turns a recording into text. The error
/// is one of `voice::Fault`'s codes — the island has both language tables for the wording.
///
/// `async` on purpose. Tauri runs a synchronous command on the main thread, so four seconds
/// of Whisper in one stopped the whole window: every event, every repaint and every other
/// call waited behind it, which is what froze Oczi in the middle of a spoken conversation.
/// The work is blocking by nature, so it goes to a thread of its own.
#[cfg(feature = "voice")]
#[tauri::command]
async fn voice_transcribe(
    request: tauri::ipc::Request<'_>,
    shared: State<'_, Shared>,
) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("bad-audio".to_string());
    };
    // Copied out of the request before the wait: the payload borrows, and the thread that
    // does the work outlives this call.
    let samples = bytes.clone();
    let language = shared.settings.lock().unwrap().voice_language.clone();
    tauri::async_runtime::spawn_blocking(move || {
        voice::transcribe_pcm(&samples, &language).map_err(|fault| fault.code().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Without the engine the command still has to exist — the island calls it from a build
/// that has one — so it says what is missing instead of failing to compile.
#[cfg(not(feature = "voice"))]
#[tauri::command]
async fn voice_transcribe(_request: tauri::ipc::Request<'_>) -> Result<String, String> {
    Err("engine".to_string())
}

/// The value after `--flag` on the command line — for the headless checks that take one.
fn flag_value(flag: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

pub fn run() {
    // The one part of play_video that needs the network, on its own: `oczi.exe --play "lofi do
    // programowania"` resolves it exactly as the tool does, opens it, and prints what it
    // picked. Without a window, so whether the ids the parser reads are real is answerable on
    // a real machine — and the tab that opens is the other half of the proof.
    if let Some(query) = flag_value("--play") {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("async runtime");
        match runtime.block_on(youtube::play(&query)) {
            Ok(started) => println!(
                "{} — {} ({})",
                if started.opened {
                    "playing"
                } else {
                    "already playing, not reopened"
                },
                started.hit.title,
                started.hit.id
            ),
            Err(e) => println!("failed: {e}"),
        }
        return;
    }

    // The OCR a pin's "copy text" uses, printed for a picture on disk: `oczi.exe --pin-ocr
    // <file.png>` reads the file and stops. The recognition needs no window and no clipboard, so a
    // wrong answer on a known picture is answerable without clicking anything.
    if let Some(file) = flag_value("--pin-ocr") {
        match ocr::text_from_png(std::path::Path::new(&file)) {
            Ok(text) if text.is_empty() => println!("(no text)"),
            Ok(text) => println!("{text}"),
            Err(err) => println!("failed: {err}"),
        }
        return;
    }

    // A read-only look at what starts with this PC, the same text the model gets from
    // startup_items. It is here for support and for checking the reader on a real
    // machine without opening a window: `oczi.exe --startup` prints it and stops.
    if std::env::args().any(|arg| arg == "--startup") {
        println!("{}", startup::text());
        return;
    }

    // The same shape for the Steam library, which is read, not walked: `oczi.exe --steam`
    // prints the games it can start and stops. A game's name is the whole of what a launch
    // has to go on, so being able to see the list without opening a window is what tells a
    // "could not find it" apart from "it is not installed".
    if std::env::args().any(|arg| arg == "--steam") {
        let games = steam::all();
        if games.is_empty() {
            println!("no steam library found");
        }
        for game in games {
            println!("{}\t{}", game.appid, game.name);
        }
        return;
    }

    // The same idea for dictation, and the only way to check the engine without speaking
    // into the app: `oczi.exe --transcribe <file.wav>` prints the text for a 16 kHz mono
    // recording and stops. A windowed build writes stdout when a terminal started it or a
    // file is redirected into it, which is how the model and the language setting get
    // compared on real speech.
    #[cfg(feature = "voice")]
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(at) = args.iter().position(|arg| arg == "--transcribe") {
            let file = args.get(at + 1).cloned().unwrap_or_default();
            let language = args.get(at + 2).cloned().unwrap_or_else(|| "auto".into());
            match std::fs::read(&file)
                .map_err(|_| voice::Fault::BadAudio)
                .and_then(|bytes| voice::wav_16k_mono(&bytes))
                .and_then(|samples| voice::transcribe(&samples, &language))
            {
                Ok(text) => println!("{text}"),
                Err(fault) => println!("voice:{}", fault.code()),
            }
            return;
        }
    }

    // How long each Gemini model actually takes, at each reasoning level the app may send:
    // `oczi.exe --chat-latency` times them with the stored key and prints the table. The free
    // tier publishes no latency figure, and a model that thinks for twenty seconds in front of
    // the notch reads as a hung app — so the choice of model is measured, not guessed. It builds
    // its own runtime because Tauri's does not exist this early.
    if std::env::args().any(|arg| arg == "--chat-latency") {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(deepseek::latency_probe()),
            Err(err) => println!("failed: {err}"),
        }
        return;
    }

    // What the model is actually told, in full: `oczi.exe --prompt` prints the system prompt
    // and stops. It is the only way to read the assembled text (every paragraph, both
    // language settings, the escaped newlines) without a live conversation, and a prompt is
    // exactly the kind of thing that reads one way in the source and another once assembled.
    if std::env::args().any(|arg| arg == "--prompt") {
        println!("{}", deepseek::system_prompt(true, true, "pl"));
        return;
    }

    // The debloat list, without a window: what the catalogue is willing to offer, read from
    // Windows right now — the same enumeration the window draws.
    if std::env::args().any(|arg| arg == "--debloat") {
        match debloat::list() {
            Ok(items) => println!("{}", debloat::describe(&items)),
            Err(e) => println!("debloat: {e}"),
        }
        return;
    }

    // The Windows settings table, printed: every row's value as this machine holds it.
    if std::env::args().any(|arg| arg == "--tweaks") {
        println!("{}", tweaks::describe(&tweaks::list()));
        return;
    }

    // Game Bar and the capture switches, without a window: the same reading the Debloat window
    // draws, printed, so the registry values behind one toggle can be checked against the machine.
    if std::env::args().any(|arg| arg == "--gamebar") {
        match gamebar::status() {
            Ok(state) => println!("{}", gamebar::describe(&state)),
            Err(e) => println!("gamebar: {e}"),
        }
        return;
    }

    // The health scan, without a window: `oczi.exe --health` prints exactly what the tool hands
    // the model, so a verdict can be read and compared against the machine it is about.
    // A watch without a window: `oczi.exe --watch 15` samples for fifteen minutes and prints the
    // report, which is the only way to test the sampler on a real freeze without the island.
    if let Some(at) = std::env::args().position(|arg| arg == "--watch") {
        let minutes = std::env::args()
            .nth(at + 1)
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .unwrap_or(15);
        let watch = health::watch(minutes);
        println!("{}", health::describe_watch(&watch));
        return;
    }

    if std::env::args().any(|arg| arg == "--health") {
        let report = health::scan(std::env::args().any(|arg| arg == "--deep"));
        println!("{}", health::describe(&report));
        return;
    }

    // A panic in the release build aborts the process, so this hook is the only
    // chance to leave a trace of it. Without it a crash was a log that simply
    // stopped, and the reason had to be guessed at.
    std::panic::set_hook(Box::new(|info| {
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        log::line(format!("PANIC at {at}: {info}"));
    }));

    let loaded = settings::load();
    let gate = Arc::new(PollGate::new());

    let moved = secrets::migrate();
    if moved > 0 {
        log::line(format!("secrets: moved {moved} keys to the oczi service"));
    }

    // Logged because a stale model name in settings.json is the one failure that
    // is invisible from the UI until every chat turn comes back a 400.
    log::line(format!(
        "chat model {} (thinking {})",
        loaded.model,
        if loaded.thinking { "on" } else { "off" }
    ));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            let _ = app.emit_to(island::WINDOW_LABEL, "tray", "open".to_string());
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Chat::default())
        .manage(Snip::default())
        // The microphone is the only permission the island ever asks for, and only when
        // dictation is used. Everything else keeps whatever WebView2 would have done
        // without this handler, so nothing else about the app changes.
        .on_permission_request(|_webview, kind| match kind {
            PermissionKind::Microphone => PermissionResponse::Allow,
            _ => PermissionResponse::Default,
        })
        .invoke_handler(tauri::generate_handler![
            boot,
            list_monitors,
            check_update,
            mark_version_seen,
            save_settings,
            set_intervals,
            set_collapsed,
            island_fullscreen,
            set_island_rect,
            set_accept_drops,
            focus_window,
            reposition,
            open_url,
            quit_app,
            log_line,
            chat_send,
            chat_reset,
            begin_snip,
            finish_snip,
            cancel_snip,
            pin_close,
            pin_drag_start,
            pin_ask,
            pin_copy_text,
            pin_copy_image,
            pin_save_as,
            pin_scale,
            pin_menu,
            pin_topmost,
            pin_mark,
            pin_close_all,
            ingest_file,
            secret_present,
            secret_value,
            secret_set,
            secret_clear,
            refresh_integration,
            media_control,
            open_n8n,
            reminders_list,
            reminder_add,
            reminder_cancel,
            reminder_done,
            reminders_ready,
            routines_list,
            routine_cancel,
            routine_done,
            routines_ready,
            routine_run_now,
            open_settings_window,
            set_paused,
            set_island_anchor,
            voice_transcribe,
            prices_lookup,
            health_scan,
            health_watch,
            health_watch_cancel,
            health_last_watch,
            health_last,
            health_fix,
            debloat_list,
            debloat_remove,
            debloat_history,
            gamebar_status,
            gamebar_capture,
            gamebar_policy_elevated,
            gamebar_install,
            startup_list,
            startup_set,
            clean_scan,
            clean_run,
            tweaks_list,
            tweak_set,
            tweak_choose,
            tweak_reset,
            tweak_elevated,
            tweak_undo,
            tweaks_history,
            tweaks_elevated_probe,
            restart_explorer,
            open_pc_window,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            // The price card is emitted from a tool call, which has no window handle of its own.
            let _ = APP.set(handle.clone());
            tray::build(&handle, &loaded.language)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle, &loaded);
            // The two PC windows, for the same reason: created once, then shown.
            create_pc_windows(&handle);
            // Same reason, and the overlay must exist before anything can ask for it.
            create_snip_window(&handle);
            if let Some(win) = island::window(&handle) {
                island::make_non_activating(&win);
                island::apply_geometry(&handle, &loaded.screen, loaded.island_anchor);
                let _ = win.show();
                // Files are handed to the window itself, not to the webview: see
                // own_file_drops for why the COM route cannot work here.
                island::own_file_drops(&handle);
            }
            // After the island has its real size: four windows built earlier delay `apply_geometry`
            // and the greeting starts on a 240-pixel-wide window.
            create_pin_windows(&handle);
            // Last, with every window created: a crash inside WebView2 must not
            // leave the island showing the runtime's own error page. See webview_guard.
            webview_guard::watch(&handle);
            gate.collapsed.store(false, Ordering::Relaxed);
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());
            island::spawn_hotkey(
                handle.clone(),
                loaded.hotkey.clone(),
                loaded.snip_hotkey.clone(),
                voice_chord(&loaded),
                loaded.pin_hotkey.clone(),
            );

            // A previous run may have been killed rather than quit: stop whatever
            // it left running, and clear the logs that no longer mean anything.
            shell::kill_orphans();
            shell::clear_logs();
            log::line(format!(
                "--- Oczi {} started ---",
                env!("CARGO_PKG_VERSION")
            ));
            integrations::start(handle.clone());
            media::start(handle.clone());
            // Reminders are the app's own promise, so they are watched here rather
            // than by anything the user has to keep alive separately.
            reminders::start(handle.clone());
            // Routines are the same promise with the doing attached: the same clock, the
            // same file under %APPDATA%\Oczi, nothing left on the machine.
            routines::start(handle.clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Oczi")
        .run(|_app, event| {
            // Nothing of ours may outlive the app: background jobs are killed and
            // their logs cleared, so quitting Oczi really stops everything.
            if let tauri::RunEvent::Exit = event {
                shell::shutdown();
            }
        });
}
