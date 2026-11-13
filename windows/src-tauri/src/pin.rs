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

/// How many crops can hang at once. Every pin keeps its window for as long as Oczi runs and only
/// closing it frees the slot, so a pinned reference stays on the screen for days; a crop that finds
/// every slot busy replaces the oldest.
pub const SLOTS: usize = 12;

/// The label of the window that carries pin `index`, and the number back out of it.
pub fn label(index: usize) -> String {
    format!("pin{}", index + 1)
}

pub fn index_of(label: &str) -> Option<usize> {
    let n: usize = label.strip_prefix("pin")?.parse().ok()?;
    // The number in the label is the slot, and `pin0` and `pin13` are not slots: a label Windows
    // could hand back is still a label, so it is refused rather than made to fit.
    (1..=SLOTS).contains(&n).then(|| n - 1)
}

/// Where pin `index` is written. One file per window, deleted when the pin closes.
pub fn file(index: usize) -> PathBuf {
    settings::local_dir().join(format!("pin-{}.png", index + 1))
}

/// The picture with its annotations drawn on, written by the page when it has any. Absent means
/// the pin is the crop as it was cut.
pub fn marked(index: usize) -> PathBuf {
    settings::local_dir().join(format!("pin-{}-marked.png", index + 1))
}

/// The file the actions should read: the annotated one when the user has drawn on the pin.
pub fn active(index: usize) -> PathBuf {
    let marked = marked(index);
    if marked.exists() {
        marked
    } else {
        file(index)
    }
}

/// Stores the annotated picture, or clears it when the page has nothing to add.
pub fn set_marked(index: usize, bytes: Option<Vec<u8>>) -> Result<(), String> {
    let path = marked(index);
    match bytes {
        Some(bytes) => {
            std::fs::write(&path, bytes).map_err(|e| format!("could not save the drawing: {e}"))?;
            log::line(format!(
                "pin  {} annotated ({} bytes)",
                label(index),
                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            ));
        }
        None => {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            log::line(format!("pin  {} drawing cleared", label(index)));
        }
    }
    Ok(())
}

// ── What survives a restart ───────────────────────────────────────────────────
//
// The pictures already live on the disk, so a pin that was on the screen when Oczi was last closed
// is the same file plus where it sat. That is written down here on every move, zoom and toggle, and
// read back by the page itself when it loads — a pin a person pinned and did not close comes back
// where they left it, even after a reboot.

/// One pin as a previous run left it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Kept {
    /// Zero-based slot, the number `label()` counts from.
    slot: usize,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    /// Above everything, or an ordinary window.
    #[serde(default = "floating")]
    topmost: bool,
    /// How faded the picture was, in percent. Shift and the wheel set it, and a pin that comes back
    /// solid covers whatever it was pinned beside.
    #[serde(default = "opaque")]
    opacity: u32,
    /// How old the crop is, so "the oldest" survives a restart too.
    #[serde(default)]
    order: u64,
}

fn floating() -> bool {
    true
}

/// No fade at all, which is how a pin starts.
fn opaque() -> u32 {
    100
}

/// The least a pin may be faded to: past this the picture cannot be read or grabbed any more.
pub const MIN_OPACITY: u32 = 15;

fn kept_path() -> PathBuf {
    settings::config_dir().join("pins.json")
}

/// An unreadable list is treated as an empty one: a pin must never be the reason the app cannot
/// start, and the bad file is left on the disk untouched.
fn kept_list() -> Vec<Kept> {
    match std::fs::read(kept_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            log::line(format!("pins unreadable ({e}), starting with none"));
            Vec::new()
        }),
        Err(_) => Vec::new(),
    }
}

fn save_kept(list: &[Kept]) {
    let write = || -> Result<(), String> {
        let dir = settings::config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
        std::fs::write(kept_path(), json).map_err(|e| e.to_string())
    };
    if let Err(err) = write() {
        // A pin that cannot be remembered is still a pin on the screen: the failure is logged and
        // the window is left alone.
        log::line(format!("pin  could not write the pin list: {err}"));
    }
}

/// Writes one window down as it is now. Called after every change to where or how it hangs, so a
/// restart never has to guess and nothing has to be flushed on the way out.
pub fn remember(index: usize) {
    let slot = match slots().lock() {
        Ok(guard) => guard[index]
            .as_ref()
            .map(|slot| (slot.base_size, slot.base_pos, slot.topmost, slot.opacity, slot.order)),
        Err(_) => None,
    };
    let Some((base_size, base_pos, topmost, opacity, order)) = slot else {
        return;
    };
    // No picture, no pin: a slot that was closed or reused has nothing worth restoring.
    if !file(index).exists() {
        return;
    }
    let entry = Kept {
        slot: index,
        x: base_pos.0,
        y: base_pos.1,
        w: base_size.0,
        h: base_size.1,
        topmost,
        opacity,
        order,
    };
    let mut list = kept_list();
    match list.iter_mut().find(|kept| kept.slot == index) {
        Some(existing) => *existing = entry,
        None => list.push(entry),
    }
    save_kept(&list);
}

/// Takes one window off the list, so a closed pin is not brought back by the next start.
fn forget(index: usize) {
    let mut list = kept_list();
    let before = list.len();
    list.retain(|kept| kept.slot != index);
    if list.len() != before {
        save_kept(&list);
    }
}

/// The picture a previous run left in this slot, shown again where the user had it. The pin page
/// asks for it when it loads rather than the app pushing it at launch: the page that has to draw
/// the picture is listening by the time this is answered, which an emit during start-up could not
/// count on.
pub fn restore(app: &AppHandle, index: usize) -> Option<Restored> {
    let kept = kept_list().into_iter().find(|kept| kept.slot == index)?;
    let path = active(index);
    let Ok(bytes) = std::fs::read(&path) else {
        // The picture went away with something else: drop the record instead of leaving a slot that
        // every later crop has to step over.
        log::line(format!(
            "pin  {} was remembered but its picture is gone",
            label(index)
        ));
        forget(index);
        return None;
    };
    let Some(win) = app.get_webview_window(&label(index)) else {
        return None;
    };
    let (w, h) = (kept.w.max(1), kept.h.max(1));
    // A monitor that is no longer there must not take the pin with it: a remembered place outside
    // the desktop is pulled back to the nearest edge, where it can still be seen and dragged.
    let (vx, vy, vw, vh) = crate::snip::desktop_bounds();
    let x = kept.x.clamp(vx, (vx + vw - w as i32).max(vx));
    let y = kept.y.clamp(vy, (vy + vh - h as i32).max(vy));
    let _ = win.set_always_on_top(kept.topmost);
    let _ = win.set_size(PhysicalSize::new(w, h));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    let _ = win.show();
    let opacity = kept.opacity.clamp(MIN_OPACITY, 100);
    if let Ok(mut guard) = slots().lock() {
        guard[index] = Some(Slot {
            base_size: (w, h),
            base_pos: (x, y),
            expanded: false,
            expanded_at: (0, 0),
            topmost: kept.topmost,
            opacity,
            order: kept.order,
        });
    }
    log::line(format!(
        "pin  {} back at {x},{y} {w}x{h} at {opacity}%",
        label(index)
    ));
    // Where it really went, not where it asked to be: a clamped place is the new truth.
    remember(index);
    Some(Restored {
        url: format!("data:image/png;base64,{}", util::base64_for(&bytes)),
        opacity,
    })
}

/// What the page gets when it asks for the pin it left here: the picture and how faded it was.
#[derive(Clone, serde::Serialize)]
pub struct Restored {
    pub url: String,
    pub opacity: u32,
}

/// The fade the user picked with Shift and the wheel. Kept here as well as in the page, because
/// only this side is written to the disk — a pin that comes back solid covers what it was beside.
pub fn set_opacity(index: usize, percent: u32) -> Result<(), String> {
    let value = percent.clamp(MIN_OPACITY, 100);
    let known = match slots().lock() {
        Ok(mut guard) => match guard[index].as_mut() {
            Some(slot) => {
                slot.opacity = value;
                true
            }
            None => false,
        },
        Err(_) => false,
    };
    if !known {
        return Err("this pin is gone".into());
    }
    log::line(format!("pin  {} at {value}%", label(index)));
    remember(index);
    Ok(())
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
    /// The fade the picture was left at, in percent: the page draws it, so this is only what gets
    /// written down and restored.
    opacity: u32,
    order: u64,
}

fn slots() -> &'static Mutex<[Option<Slot>; SLOTS]> {
    static CELL: OnceLock<Mutex<[Option<Slot>; SLOTS]>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(std::array::from_fn(|_| None)))
}

/// The reason a crop was refused after the drag. The overlay turns this into a sentence in the
/// user's own language: every pin is somebody's reference, so the app never drops one to make room.
pub const PINS_FULL: &str = "pins-full";

/// Takes a window for a new crop, before the crop is written: the file is named after the slot.
/// Full is an error, not a reason to replace the oldest — what is pinned stays until it is closed.
pub fn reserve() -> Result<usize, String> {
    let kept = kept_list();
    // A run that starts with pins on the screen numbers its new crops after them, so "the oldest"
    // is still the oldest after a restart.
    let base = kept.iter().map(|kept| kept.order + 1).max().unwrap_or(0);
    if NEXT_ORDER.load(Ordering::Relaxed) < base {
        NEXT_ORDER.store(base, Ordering::Relaxed);
    }
    let order = NEXT_ORDER.fetch_add(1, Ordering::Relaxed);
    let mut guard = slots().lock().unwrap();
    // A slot a previous run left occupied is not free either: its page has not asked for the
    // picture yet when a new crop is cut, and taking it would lose something that was pinned.
    let free = (0..SLOTS).find(|index| {
        guard[*index].is_none() && !kept.iter().any(|kept| kept.slot == *index && file(*index).exists())
    });
    let Some(index) = free else {
        log::line(format!(
            "pin  all {SLOTS} pins are open — the new crop was refused"
        ));
        return Err(PINS_FULL.into());
    };
    // Whatever the slot held belongs to it: a new crop starts with no drawing of its own either.
    for path in [file(index), marked(index)] {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    forget(index);
    guard[index] = Some(Slot {
        base_size: (1, 1),
        base_pos: (0, 0),
        expanded: false,
        expanded_at: (0, 0),
        topmost: true,
        opacity: opaque(),
        order,
    });
    Ok(index)
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
            slot.opacity = opaque();
        }
    }
    log::line(format!("pin  {name} {w}x{h} at {x},{y}"));
    // Written down now, not at exit: the app may be killed, and a pin that came back at a stale
    // place would be worse than one that came back at all.
    remember(index);
    Ok(())
}

/// Hides one pin and drops its file. Every exit goes through here — and so does the record that a
/// restart reads: a closed pin is the one thing that must not come back.
pub fn close(app: &AppHandle, index: usize) {
    if let Some(win) = app.get_webview_window(&label(index)) {
        let _ = win.hide();
    }
    for path in [file(index), marked(index)] {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    if let Ok(mut guard) = slots().lock() {
        guard[index] = None;
    }
    forget(index);
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
            // Where the pin was let go is where the next start has to put it back.
            if let Ok(mut guard) = slots().lock() {
                if let Some(slot) = guard[index].as_mut() {
                    slot.base_pos = (pos.x, pos.y);
                }
            }
            log::line(format!("pin  moved to {},{}", pos.x, pos.y));
            remember(index);
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
    remember(index);
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
    remember(index);
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
    let path = active(index);
    if !path.exists() {
        return Err("there is no pin to copy".into());
    }
    crate::clipboard::set_image(&path)?;
    log::line(format!("pin  {} picture on the clipboard", label(index)));
    Ok(())
}

/// Hands the pin's picture to the chat and leaves the pin where it is.
pub fn ask(app: &AppHandle, snip: &crate::snip::Snip, index: usize) -> Result<(), String> {
    let path = active(index);
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

    let source = active(index);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pin_mode_belongs_to_exactly_one_capture() {
        // The flag is read once and cleared, which is what makes an abandoned pin harmless: the
        // capture that follows it must be an ordinary snip, not a second pin.
        take_mode();
        start_pin_mode();
        assert!(take_mode());
        assert!(!take_mode());
    }

    #[test]
    fn every_slot_has_a_name_and_only_its_own() {
        // The number in the label is the slot, and every one of them needs a window named exactly
        // that in the capability list: a slot without one is a pin page that cannot call anything.
        assert_eq!(SLOTS, 12);
        for index in 0..SLOTS {
            assert_eq!(index_of(&label(index)), Some(index));
        }
        assert_eq!(index_of("pin0"), None);
        assert_eq!(index_of("pin13"), None);
        assert_eq!(index_of("island"), None);
    }
}
