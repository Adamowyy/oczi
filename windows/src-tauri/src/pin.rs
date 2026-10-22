// Pinned crops: regions that stay above other windows and can be dragged anywhere on the desktop,
// including the other monitor. Pixels stay in Rust; the windows are made at launch and only shown
// and hidden, because WebView2 will not build a usable one after start-up.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::{log, settings, util};

/// How many crops can hang at once. A new pin takes the first free window, and only replaces the
/// oldest when they are all busy.
pub const SLOTS: usize = 4;

/// The label of the window that carries pin `index`, and the number back out of it.
pub fn label(index: usize) -> String {
    format!("pin{}", index + 1)
}

pub fn index_of(label: &str) -> Option<usize> {
    let n: usize = label.strip_prefix("pin")?.parse().ok()?;
    (1..=SLOTS).contains(&n).then_some(n - 1)
}

/// Where pin `index` is written. One file per window, deleted when the pin closes.
pub fn file(index: usize) -> PathBuf {
    settings::local_dir().join(format!("pin-{}.png", index + 1))
}

/// Set when a capture is started by the pin chord rather than by the eye, so `finish_snip` knows
/// which of the two things to do with the crop.
static PIN_MODE: AtomicBool = AtomicBool::new(false);
static NEXT_ORDER: AtomicU64 = AtomicU64::new(0);

pub fn start_pin_mode() {
    PIN_MODE.store(true, Ordering::Relaxed);
}

/// Reads the flag once and clears it: a capture belongs to the mode it was started in, and a flag
/// left standing would send the next ordinary snip to the pin.
pub fn take_mode() -> bool {
    PIN_MODE.swap(false, Ordering::Relaxed)
}

/// What one window knows about the crop it holds.
struct Slot {
    base_size: (u32, u32),
    base_pos: (i32, i32),
    expanded: bool,
    /// How far the picture sits from the window's corner while a menu has it bigger. Closing the
    /// menu shrinks the window around this offset, not around the old position.
    expanded_at: (i32, i32),
    topmost: bool,
    order: u64,
}

fn slots() -> &'static Mutex<[Option<Slot>; SLOTS]> {
    static CELL: OnceLock<Mutex<[Option<Slot>; SLOTS]>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(std::array::from_fn(|_| None)))
}

/// Takes a window for a new crop, before the crop is written: the file is named after the slot.
pub fn reserve() -> usize {
    let order = NEXT_ORDER.fetch_add(1, Ordering::Relaxed);
    let mut guard = slots().lock().unwrap();
    let free = guard.iter().position(|slot| slot.is_none());
    let index = match free {
        Some(index) => index,
        None => {
            let oldest = guard
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| slot.as_ref().map(|slot| (slot.order, index)))
                .min();
            let index = oldest.map(|(_, index)| index).unwrap_or(0);
            log::line(format!("pin  all {SLOTS} windows busy — the oldest goes"));
            index
        }
    };
    guard[index] = Some(Slot {
        base_size: (1, 1),
        base_pos: (0, 0),
        expanded: false,
        expanded_at: (0, 0),
        topmost: true,
        order,
    });
    index
}

/// Puts the picture back exactly over the region it was cut from, in physical pixels from the
/// desktop origin, which is what a window position takes.
pub fn show(app: &AppHandle, index: usize, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    let name = label(index);
    let Some(win) = app.get_webview_window(&name) else {
        return Err(format!("the pin window {name} is missing"));
    };
    let bytes = std::fs::read(file(index)).map_err(|e| format!("could not read the pin: {e}"))?;
    let url = format!("data:image/png;base64,{}", util::base64_for(&bytes));
    let (w, h) = (w.max(1), h.max(1));

    // A new crop starts above everything, until the user says otherwise.
    let _ = win.set_always_on_top(true);
    win.set_size(PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| format!("could not size the pin: {e}"))?;
    win.set_position(PhysicalPosition::new(x, y))
        .map_err(|e| format!("could not place the pin: {e}"))?;
    win.show()
        .map_err(|e| format!("could not show the pin: {e}"))?;
    // Named after the window: a JS listener takes every target, so one shared name reached all
    // four pages and each drew the same crop.
    app.emit_to(&name, &format!("pin-image-{name}"), url)
        .map_err(|e| format!("could not draw the pin: {e}"))?;

    if let Ok(mut guard) = slots().lock() {
        if let Some(slot) = guard[index].as_mut() {
            slot.base_size = (w as u32, h as u32);
            slot.base_pos = (x, y);
            slot.expanded = false;
            slot.expanded_at = (0, 0);
            slot.topmost = true;
        }
    }
    log::line(format!("pin  {name} {w}x{h} at {x},{y}"));
    Ok(())
}

/// Hides one pin and drops its file. Every exit goes through here.
pub fn close(app: &AppHandle, index: usize) {
    if let Some(win) = app.get_webview_window(&label(index)) {
        let _ = win.hide();
    }
    let path = file(index);
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut guard) = slots().lock() {
        guard[index] = None;
    }
    log::line(format!("pin  {} closed", label(index)));
}

/// Follows the cursor while the left button is held. The OS drag loop wants a focused caption
/// click and left the window where it was; this works from any window and crosses monitor edges.
pub fn start_drag(app: &AppHandle, index: usize) {
    let Some(win) = app.get_webview_window(&label(index)) else {
        return;
    };
    let (Ok(pos), Some(start)) = (win.outer_position(), cursor()) else {
        return;
    };
    let dx = start.0 - pos.x;
    let dy = start.1 - pos.y;
    std::thread::spawn(move || {
        while left_button_down() {
            if let Some(c) = cursor() {
                let _ = win.set_position(PhysicalPosition::new(c.0 - dx, c.1 - dy));
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        if let Ok(pos) = win.outer_position() {
            log::line(format!("pin  moved to {},{}", pos.x, pos.y));
        }
    });
}

fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x, p.y))
}

/// The picture's own size, read from its header: a zoom makes the window no longer its measure.
pub fn picture_size(path: &Path) -> Option<(u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

/// Grows or shrinks the pin around its middle, keeping the crop's proportions. The limit applies to
/// the smallest side, so two independent minimums cannot squeeze the shape.
pub fn scale(app: &AppHandle, index: usize, step: i32) -> Result<(), String> {
    const MIN_SIDE: f64 = 60.0;
    const MAX_SIDE: f64 = 3000.0;

    let Some(win) = app.get_webview_window(&label(index)) else {
        return Err("the pin window is missing".into());
    };
    let size = win
        .outer_size()
        .map_err(|e| format!("could not measure the pin: {e}"))?;
    let pos = win
        .outer_position()
        .map_err(|e| format!("could not measure the pin: {e}"))?;
    let factor = if step >= 0 { 1.15 } else { 1.0 / 1.15 };
    let mut w = size.width as f64 * factor;
    let mut h = size.height as f64 * factor;
    let smallest = w.min(h);
    if smallest < MIN_SIDE {
        let k = MIN_SIDE / smallest;
        w *= k;
        h *= k;
    }
    let largest = w.max(h);
    if largest > MAX_SIDE {
        let k = MAX_SIDE / largest;
        w *= k;
        h *= k;
    }
    let (w, h) = (w.round() as i32, h.round() as i32);
    let cx = pos.x + size.width as i32 / 2;
    let cy = pos.y + size.height as i32 / 2;

    win.set_size(PhysicalSize::new(w as u32, h as u32))
        .map_err(|e| format!("could not resize the pin: {e}"))?;
    win.set_position(PhysicalPosition::new(cx - w / 2, cy - h / 2))
        .map_err(|e| format!("could not place the pin: {e}"))?;
    if let Ok(mut guard) = slots().lock() {
        if let Some(slot) = guard[index].as_mut() {
            slot.base_size = (w as u32, h as u32);
        }
    }
    log::line(format!("pin  {} zoomed to {w}x{h}", label(index)));
    Ok(())
}

/// The menu needs room a small crop does not have, so the window is made bigger for as long as it
/// is open, towards the side of the monitor with space. Returns the picture's offset inside the
/// window in CSS pixels, which is what the page draws from.
pub fn menu(
    app: &AppHandle,
    index: usize,
    open: bool,
    need_w: f64,
    need_h: f64,
) -> Result<(f64, f64), String> {
    let name = label(index);
    let Some(win) = app.get_webview_window(&name) else {
        return Err(format!("the pin window {name} is missing"));
    };
    let scale = win.scale_factor().map_err(|e| e.to_string())?.max(0.25);
    let (base_size, expanded, expanded_at) = {
        let guard = slots().lock().map_err(|_| "the pin state is poisoned".to_string())?;
        let slot = guard[index]
            .as_ref()
            .ok_or_else(|| "this pin is gone".to_string())?;
        (slot.base_size, slot.expanded, slot.expanded_at)
    };

    if !open {
        if expanded {
            // Shrink back around the offset, not the old position: the pin may have been dragged.
            if let Ok(pos) = win.outer_position() {
                let moved = PhysicalPosition::new(pos.x + expanded_at.0, pos.y + expanded_at.1);
                let _ = win.set_size(PhysicalSize::new(base_size.0, base_size.1));
                let _ = win.set_position(moved);
                if let Ok(mut guard) = slots().lock() {
                    if let Some(slot) = guard[index].as_mut() {
                        slot.base_pos = (moved.x, moved.y);
                    }
                }
            } else {
                let _ = win.set_size(PhysicalSize::new(base_size.0, base_size.1));
            }
            if let Ok(mut guard) = slots().lock() {
                if let Some(slot) = guard[index].as_mut() {
                    slot.expanded = false;
                    slot.expanded_at = (0, 0);
                }
            }
        }
        return Ok((0.0, 0.0));
    }
    if expanded {
        return Ok((0.0, 0.0));
    }

    // Eight physical pixels of breathing room between the picture and the menu.
    const GAP: u32 = 8;
    let want_w = (need_w * scale).round().max(0.0) as u32;
    let want_h = (need_h * scale).round().max(0.0) as u32;
    let extra_w = want_w + GAP;
    let extra_h = want_h + GAP;

    let pos = win
        .outer_position()
        .map_err(|e| format!("could not measure the pin: {e}"))?;
    let mon = win
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "the pin is not on a monitor".to_string())?;
    let mon_left = mon.position().x;
    let mon_top = mon.position().y;
    let mon_right = mon_left + mon.size().width as i32;
    let mon_bottom = mon_top + mon.size().height as i32;
    let room_right = (mon_right - (pos.x + base_size.0 as i32)).max(0) as u32;
    let room_left = (pos.x - mon_left).max(0) as u32;
    let room_down = (mon_bottom - (pos.y + base_size.1 as i32)).max(0) as u32;
    let room_up = (pos.y - mon_top).max(0) as u32;

    let grow_w = extra_w.min(room_right.max(room_left));
    let grow_h = extra_h.min(room_down.max(room_up));
    let (new_x, offset_x) = if grow_w == 0 {
        (pos.x, 0)
    } else if room_right >= grow_w {
        (pos.x, 0)
    } else {
        (pos.x - grow_w as i32, grow_w)
    };
    let (new_y, offset_y) = if grow_h == 0 {
        (pos.y, 0)
    } else if room_down >= grow_h {
        (pos.y, 0)
    } else {
        (pos.y - grow_h as i32, grow_h)
    };

    win.set_size(PhysicalSize::new(base_size.0 + grow_w, base_size.1 + grow_h))
        .map_err(|e| format!("could not make room for the menu: {e}"))?;
    win.set_position(PhysicalPosition::new(new_x, new_y))
        .map_err(|e| format!("could not place the menu: {e}"))?;
    if let Ok(mut guard) = slots().lock() {
        if let Some(slot) = guard[index].as_mut() {
            slot.expanded = true;
            slot.expanded_at = (offset_x as i32, offset_y as i32);
        }
    }
    Ok((offset_x as f64 / scale, offset_y as f64 / scale))
}

/// Above everything, or an ordinary window.
pub fn set_topmost(app: &AppHandle, index: usize, on: bool) -> Result<(), String> {
    let Some(win) = app.get_webview_window(&label(index)) else {
        return Err("the pin window is missing".into());
    };
    win.set_always_on_top(on)
        .map_err(|e| format!("could not change the pin: {e}"))?;
    if let Ok(mut guard) = slots().lock() {
        if let Some(slot) = guard[index].as_mut() {
            slot.topmost = on;
        }
    }
    log::line(format!(
        "pin  {} {}",
        label(index),
        if on { "on top" } else { "normal window" }
    ));
    Ok(())
}

/// Every pin at once.
pub fn close_all(app: &AppHandle) {
    let busy: Vec<usize> = match slots().lock() {
        Ok(guard) => guard
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|_| index))
            .collect(),
        Err(_) => Vec::new(),
    };
    for index in busy {
        close(app, index);
    }
}

/// The text the pin shows, through the system's OCR, put on the clipboard and handed back.
pub fn copy_text(index: usize) -> Result<String, String> {
    let path = file(index);
    if !path.exists() {
        return Err("there is no pin to read".into());
    }
    let text = crate::ocr::text_from_png(&path)?;
    if text.is_empty() {
        return Ok(text);
    }
    crate::clipboard::set_text(&text)?;
    log::line(format!(
        "pin  {} copied {} characters of text",
        label(index),
        text.chars().count()
    ));
    Ok(text)
}

/// The pin's pixels as an image, for anything that takes a pasted picture.
pub fn copy_image(index: usize) -> Result<(), String> {
    let path = file(index);
    if !path.exists() {
        return Err("there is no pin to copy".into());
    }
    crate::clipboard::set_image(&path)?;
    log::line(format!("pin  {} picture on the clipboard", label(index)));
    Ok(())
}

/// Hands the pin's picture to the chat and leaves the pin where it is.
pub fn ask(app: &AppHandle, snip: &crate::snip::Snip, index: usize) -> Result<(), String> {
    let path = file(index);
    if !path.exists() {
        return Err("there is no pin to ask about".into());
    }
    let (width, height) =
        picture_size(&path).ok_or_else(|| "the pin's picture is unreadable".to_string())?;
    snip.park(path);
    let _ = app.emit_to(
        crate::island::WINDOW_LABEL,
        "pin-ask",
        AskPayload { width, height },
    );
    log::line(format!("pin  {} asking about {width}x{height}", label(index)));
    Ok(())
}

#[derive(Clone, serde::Serialize)]
struct AskPayload {
    width: u32,
    height: u32,
}

/// Windows' own save dialog. `Ok(None)` is the user closing it, which is not a failure.
pub fn save_as(index: usize) -> Result<Option<String>, String> {
    use windows::core::{w, Interface};
    use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FileSaveDialog, IFileDialog, IFileSaveDialog, FOS_OVERWRITEPROMPT, SIGDN_FILESYSPATH,
    };

    let source = file(index);
    if !source.exists() {
        return Err("there is no pin to save".into());
    }

    unsafe {
        let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("could not open the save dialog: {e}"))?;
        // The file-name, filter and show methods all live on the parent interface.
        let ui: IFileDialog = dialog
            .cast::<IFileDialog>()
            .map_err(|e| format!("could not prepare the save dialog: {e}"))?;
        let types = [COMDLG_FILTERSPEC {
            pszName: w!("PNG picture"),
            pszSpec: w!("*.png"),
        }];
        ui.SetFileTypes(&types)
            .map_err(|e| format!("could not set the file type: {e}"))?;
        ui.SetDefaultExtension(w!("png"))
            .map_err(|e| format!("could not set the extension: {e}"))?;
        ui.SetFileName(w!("pin.png"))
            .map_err(|e| format!("could not name the file: {e}"))?;
        if let Ok(options) = ui.GetOptions() {
            let _ = ui.SetOptions(options | FOS_OVERWRITEPROMPT);
        }
        if let Err(err) = ui.Show(None) {
            // 0x800704C7 is the user closing the dialog: a choice, not a failure.
            if err.code().0 as u32 == 0x8007_04C7 {
                return Ok(None);
            }
            return Err(format!("could not show the save dialog: {err}"));
        }
        let item = ui
            .GetResult()
            .map_err(|e| format!("could not read the choice: {e}"))?;
        let shown = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|e| format!("could not read the path: {e}"))?;
        let target = shown
            .to_string()
            .map_err(|e| format!("could not read the path: {e}"))?;
        CoTaskMemFree(Some(shown.0 as *const _));
        std::fs::copy(&source, &target).map_err(|e| format!("could not write {target}: {e}"))?;
        log::line(format!("pin  {} saved to {target}", label(index)));
        Ok(Some(target))
    }
}
