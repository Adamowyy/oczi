// Notification-area icon: Open Oczi, Settings, Pause, Quit.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter};

use crate::island::WINDOW_LABEL;

/// The system draws the tray, so these four labels never pass through the front
/// end's tables: they are translated here, from the UI language.
fn labels(language: &str) -> [&'static str; 4] {
    if language == "pl" {
        // i18n-ok: the Polish branch, picked at run time from the UI language.
        ["Otwórz Oczi", "Ustawienia…", "Wstrzymaj", "Zakończ"]
    } else {
        ["Open Oczi", "Settings…", "Pause", "Quit"]
    }
}

fn menu(app: &AppHandle, language: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let [open, settings, pause, quit] = labels(language);
    let open = MenuItem::with_id(app, "open", open, true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", settings, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", pause, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit, true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&open, &sep1, &settings, &pause, &sep2, &quit])
}

pub fn build(app: &AppHandle, language: &str) -> tauri::Result<()> {
    let items = menu(app, language)?;

    let mut builder = TrayIconBuilder::with_id("oczi")
        .tooltip("Oczi")
        .menu(&items)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => crate::show_settings_window(app),
            id => {
                let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// Retitles the tray after a language change. Four items are cheaper to build
/// again than to walk and retitle.
pub fn set_language(app: &AppHandle, language: &str) {
    let Some(tray) = app.tray_by_id("oczi") else {
        return;
    };
    match menu(app, language) {
        Ok(items) => {
            if let Err(err) = tray.set_menu(Some(items)) {
                crate::log::line(format!("tray menu: {err}"));
            }
        }
        Err(err) => crate::log::line(format!("tray menu: {err}")),
    }
}
