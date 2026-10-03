// Oczi, app wiring and the commands the island calls.

mod deepseek;
mod files;
mod integrations;
mod island;
mod log;
mod secrets;
mod settings;
mod shell;
mod snip;
mod text_tools;
mod tray;
mod util;
mod web;
mod webview_guard;

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

use deepseek::{Chat, ChatContext, ChatReply, Options};
use files::DroppedFile;
use island::{PollGate, ScreenInfo};
use settings::Settings;
use snip::Snip;

/// Keeps spawned helpers from flashing a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Shared {
    pub settings: Mutex<Settings>,
    pub gate: Arc<PollGate>,
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

#[tauri::command]
fn save_settings(app: AppHandle, shared: State<Shared>, settings: Settings) {
    let (screen_changed, autostart_changed, hotkey_changed) = {
        let mut current = shared.settings.lock().unwrap();
        let screen_changed = current.screen != settings.screen;
        let autostart_changed = current.autostart != settings.autostart;
        let hotkey_changed = current.hotkey != settings.hotkey;
        *current = settings.clone();
        (screen_changed, autostart_changed, hotkey_changed)
    };
    if let Err(err) = settings::save(&settings) {
        eprintln!("[oczi] could not save settings: {err}");
    }
    if autostart_changed {
        let manager = app.autolaunch();
        let result = if settings.autostart { manager.enable() } else { manager.disable() };
        if let Err(err) = result {
            eprintln!("[oczi] autostart: {err}");
        }
    }
    if screen_changed {
        island::apply_geometry(&app, &settings.screen);
    }
    if hotkey_changed {
        island::update_hotkey(&settings.hotkey);
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

/// The front end pushes the island shape; Rust decides click-through from it.
#[tauri::command]
fn set_island_rect(shared: State<Shared>, x: f64, y: f64, width: f64, height: f64) {
    shared.gate.set_rect(island::IslandRect { x, y, w: width, h: height });
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
    let Some(win) = island::window(&app) else { return };
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
    let pref = shared.settings.lock().unwrap().screen.clone();
    island::apply_geometry(&app, &pref);
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return;
    }
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", &url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
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
    let (model, thinking, web_search, provider, terminal) = {
        let settings = shared.settings.lock().unwrap();
        (
            settings.model.clone(),
            settings.thinking,
            settings.web_search,
            settings.search_provider.clone(),
            settings.terminal_enabled,
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
            web: web_search,
            provider: &provider,
            terminal,
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

/// The eye, step one: freeze the desktop, then put the selection overlay on screen.
/// The island comes down first, it is topmost, so it would otherwise be inside its
/// own screenshot and on top of the overlay.
#[tauri::command]
async fn begin_snip(app: AppHandle) -> Result<(), String> {
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
    let info = snip::finish(
        &snip,
        (origin_x - vx as f64 + x * scale).round() as i32,
        (origin_y - vy as f64 + y * scale).round() as i32,
        (width * scale).round() as i32,
        (height * scale).round() as i32,
    )?;
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

/// Refresh buttons in the integration cards.
#[tauri::command]
async fn refresh_integration(app: AppHandle, id: String) {
    integrations::poll_once(app, &id).await;
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

/// Created hidden at launch and only shown/hidden afterwards: a WebView2 window
/// created later silently comes up blank, so it must exist before the island.
fn create_settings_window(app: &AppHandle) {
    let url = settings_page_url(app);
    match WebviewWindowBuilder::new(app, "settings", url)
        .additional_browser_args(BROWSER_ARGS)
        .title("Settings — Oczi")
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
        Ok(_) => log::line(format!("snip overlay ready — desktop {x},{y} {w}x{h} @{scale}x")),
        Err(err) => log::line(format!("snip overlay failed: {err}")),
    }
}

#[tauri::command]
fn open_settings_window(app: AppHandle) {
    show_settings_window(&app);
}

pub fn run() {
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
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .manage(Shared {
            settings: Mutex::new(loaded.clone()),
            gate: gate.clone(),
        })
        .manage(Chat::default())
        .manage(Snip::default())
        .invoke_handler(tauri::generate_handler![
            boot,
            save_settings,
            set_collapsed,
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
            ingest_file,
            secret_present,
            secret_set,
            secret_clear,
            refresh_integration,
            open_n8n,
            open_settings_window,
            set_paused,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::build(&handle)?;
            // Before the island: see create_settings_window.
            create_settings_window(&handle);
            // Same reason, and the overlay must exist before anything can ask for it.
            create_snip_window(&handle);

            if let Some(win) = island::window(&handle) {
                island::make_non_activating(&win);
                island::apply_geometry(&handle, &loaded.screen);
                let _ = win.show();
                // Files are handed to the window itself, not to the webview: see
                // own_file_drops for why the COM route cannot work here.
                island::own_file_drops(&handle);
            }
            // Last, with every window created: a crash inside WebView2 must not
            // leave the island showing the runtime's own error page. See webview_guard.
            webview_guard::watch(&handle);
            gate.collapsed.store(false, Ordering::Relaxed);
            gate.set_active(true);
            island::spawn_cursor_poll(handle.clone(), gate.clone());
            island::spawn_hotkey(handle.clone(), loaded.hotkey.clone());

            // A previous run may have been killed rather than quit: stop whatever
            // it left running, and clear the logs that no longer mean anything.
            shell::kill_orphans();
            shell::clear_logs();
            log::line(format!("--- Oczi {} started ---", env!("CARGO_PKG_VERSION")));
            integrations::start(handle.clone());
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
