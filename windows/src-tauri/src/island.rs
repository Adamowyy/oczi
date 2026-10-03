
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use std::cell::UnsafeCell;

use windows::core::{implement, BOOL, Interface};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    OleInitialize, RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use windows::Win32::UI::Shell::{DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

pub const PANEL_W: f64 = 720.0;
pub const PANEL_H: f64 = 320.0;

pub const WINDOW_LABEL: &str = "island";

/// Spin rate of the cursor poll. While the island is hidden there is nothing to
/// animate, so it drops to a slow tick that is only there to notice a hover or a
/// file being dragged in, a GetCursorPos every 125 ms is below measurement.
const POLL_HZ_ACTIVE: u64 = 16;
const POLL_HZ_IDLE: u64 = 125;

/// Id of our global hotkey (Ctrl+Alt+C), and the mods it is registered with.
const HOTKEY_ID: i32 = 0xC0CC;
/// Ctrl+Alt+S, the same thing as clicking the eye.
const HOTKEY_SNIP_ID: i32 = 0xC0CD;

/// Margin around the island that still counts as "on the island", in logical px.
/// Wider than the macOS 6 pt because a click must never be swallowed.
const HIT_MARGIN: f64 = 14.0;

#[derive(Serialize, Clone)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Clone)]
pub struct ScreenInfo {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The island shape in window-logical coordinates, pushed by the front end.
/// The poll thread owns the click-through decision so it lands in the same 16 ms
/// tick as the cursor read, an IPC round trip here loses clicks.
#[derive(Clone, Copy, Default)]
pub struct IslandRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Wakes / parks the cursor poll thread so a hidden island costs literally nothing.
pub struct PollGate {
    active: Mutex<bool>,
    cv: Condvar,
    pub collapsed: AtomicBool,
    pub rect: Mutex<IslandRect>,
    /// The island only takes a file drop while the user is in the explicit
    /// upload flow (the plus tab). Outside it, DragEnter answers NONE so the
    /// drag passes through to whatever window is underneath.
    pub accept_drops: AtomicBool,
    /// Mirrors the window flag so we only call into Win32 when it changes.
    ignoring: AtomicBool,
}

impl PollGate {
    pub fn new() -> Self {
        Self {
            active: Mutex::new(false),
            cv: Condvar::new(),
            collapsed: AtomicBool::new(true),
            rect: Mutex::new(IslandRect::default()),
            accept_drops: AtomicBool::new(false),
            ignoring: AtomicBool::new(false),
        }
    }

    pub fn set_rect(&self, rect: IslandRect) {
        *self.rect.lock().unwrap() = rect;
    }

    /// Forces the next poll tick to re-apply the flag (after a window resize).
    pub fn forget_ignore_state(&self) {
        self.ignoring.store(false, Ordering::Relaxed);
    }

    pub fn set_active(&self, on: bool) {
        let mut guard = self.active.lock().unwrap();
        *guard = on;
        self.cv.notify_all();
    }

    fn is_active(&self) -> bool {
        *self.active.lock().unwrap()
    }
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

/// Puts the island back on screen after the snip overlay had the stage, and hands it
/// the keyboard: the very next thing the user does is type their question about the
/// shot they just took, so the caret has to be there without another click.
pub fn show(app: &AppHandle) {
    if let Some(win) = window(app) {
        let _ = win.show();
    }
    focus(app);
}

// Takes the foreground, for the one moment where the user is expected to type.
pub fn focus(app: &AppHandle) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let Some(win) = window(app) else { return };
    let already_front = match hwnd_of(&win) {
        Some(hwnd) => unsafe { GetForegroundWindow() == hwnd },
        None => false,
    };
    if !already_front {
        unsafe {
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        }
    }
    let _ = win.set_focus();
}

fn cursor_physical() -> Option<(f64, f64)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x as f64, p.y as f64))
}

// Makes the island a place Windows hands dropped files to.
pub fn own_file_drops(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(win) = window(app) else { return };
    let Some(hwnd) = hwnd_of(&win) else { return };
    // RegisterDragDrop needs the thread in an OLE apartment. WebView2 only initialises
    // plain COM (STA), so OLE must be turned on here, the call is idempotent and the
    // result is deliberately ignored either way.
    unsafe {
        let _ = OleInitialize(None);
    }
    let target = drop_target();
    unsafe {
        install_target(hwnd, target);
        let _ = EnumChildWindows(Some(hwnd), Some(install_target_each), LPARAM(0));
    }
}

/// Revokes whatever target a window had and hands it ours.
unsafe fn install_target(hwnd: HWND, target_raw: *mut core::ffi::c_void) {
    unsafe {
        let _ = RevokeDragDrop(hwnd);
        let target = IDropTarget::from_raw(target_raw);
        let _ = RegisterDragDrop(hwnd, &target);
        std::mem::forget(target);
    }
}

unsafe extern "system" fn install_target_each(hwnd: HWND, _: LPARAM) -> BOOL {
    unsafe { install_target(hwnd, drop_target()) };
    true.into()
}

/// The one drop target, created on first use and intentionally leaked for the app's
/// lifetime. `RegisterDragDrop` keeps its own references per window, so the pointer
/// only has to stay valid until the windows go away, which is when the process does.
fn drop_target() -> *mut core::ffi::c_void {
    let mut raw = DROP_TARGET_PTR.load(Ordering::Relaxed) as *mut core::ffi::c_void;
    if raw.is_null() {
        let target: IDropTarget = DropTarget::new().into();
        raw = target.into_raw();
        DROP_TARGET_PTR.store(raw as usize, Ordering::Relaxed);
        crate::log::line("drop  island takes files itself (IDropTarget)".to_string());
    }
    raw
}

#[implement(IDropTarget)]
struct DropTarget {
    valid: UnsafeCell<bool>,
    effect: UnsafeCell<DROPEFFECT>,
}

impl DropTarget {
    fn new() -> Self {
        Self {
            valid: UnsafeCell::new(false),
            effect: UnsafeCell::new(DROPEFFECT_NONE),
        }
    }

    /// Every path in a CF_HDROP payload, or None when it is not a file drop. The
    /// returned `HDROP` must be released with `DragFinish` once the drop is done.
    unsafe fn paths(data: windows::core::Ref<'_, IDataObject>) -> Option<(Vec<String>, HDROP)> {
        let format = FORMATETC {
            cfFormat: CF_HDROP.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let medium = unsafe { data.as_ref()?.GetData(&format).ok()? };
        let hdrop = HDROP(medium.u.hGlobal.0 as _);
        let count = unsafe { DragQueryFileW(hdrop, 0xFFFF_FFFF, None) };
        let mut paths = Vec::new();
        for index in 0..count {
            let len = unsafe { DragQueryFileW(hdrop, index, None) };
            if len == 0 {
                continue;
            }
            let mut buffer = vec![0u16; len as usize + 1];
            unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) };
            paths.push(String::from_utf16_lossy(&buffer[..len as usize]));
        }
        Some((paths, hdrop))
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        data: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        // Only the explicit upload flow (the plus tab) accepts files; anywhere
        // else the island stays out of the way and the drag passes through.
        let accepting = APP
            .get()
            .and_then(|app| app.try_state::<crate::Shared>())
            .map(|s| s.gate.accept_drops.load(Ordering::Relaxed))
            .unwrap_or(false);
        let valid = accepting && unsafe { DropTarget::paths(data).is_some() };
        let value = if valid { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
        unsafe {
            *self.valid.get() = valid;
            *self.effect.get() = value;
            *effect = value;
        }
        // Only a real file drag turns the island into the drop box, never an
        // accidental grab of text, an icon or anything else the shell won't hand us.
        if valid {
            if let Some(app) = APP.get() {
                let _ = app.emit_to(WINDOW_LABEL, "drag", DragEvent { kind: "enter", paths: Vec::new() });
            }
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe { *effect = *self.effect.get() };
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if unsafe { *self.valid.get() } {
            unsafe { *self.valid.get() = false };
            if let Some(app) = APP.get() {
                let _ = app.emit_to(WINDOW_LABEL, "drag", DragEvent { kind: "leave", paths: Vec::new() });
            }
        }
        Ok(())
    }

    fn Drop(
        &self,
        data: windows::core::Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        _effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let mut paths = Vec::new();
        if let Some((p, hdrop)) = unsafe { DropTarget::paths(data) } {
            paths = p;
            unsafe { DragFinish(hdrop) };
        }
        if let Some(app) = APP.get() {
            let _ = app.emit_to(WINDOW_LABEL, "drag", DragEvent { kind: "drop", paths });
        }
        Ok(())
    }
}

/// The page the island is supposed to be showing, remembered from the first URL that
/// looks like ours. Used to put it back if anything navigates it away.
static HOME_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static PAGE_LOG: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A browser inside the island is a liability: anything that navigates it, a dropped
/// file, a stray link, leaves a page with no way back to the app. If the island is no
/// longer on its own page, send it home.
pub fn keep_on_its_own_page(win: &WebviewWindow) {
    let Ok(url) = win.url() else { return };
    let url = url.to_string();
    match HOME_URL.get() {
        // Compared by origin, not by full URL: the page's own paths must not look like
        // a failed navigation, or the island would be reloaded every half second.
        Some(home) if !url.starts_with(home.as_str()) => {
            crate::log::line(format!("island was on {url} — sending it back home"));
            let _ = win.eval(format!("location.replace({:?})", format!("{home}/index.html")));
        }
        Some(_) => {}
        None => {
            let origin = url
                .split_once("://")
                .and_then(|(_, rest)| rest.split('/').next())
                .unwrap_or("");
            let ours = origin.starts_with("tauri.localhost") || origin.starts_with("localhost");
            if ours {
                let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("http");
                let home = format!("{scheme}://{origin}");
                crate::log::line(format!("island page  {url}"));
                let _ = HOME_URL.set(home);
            } else if PAGE_LOG.fetch_add(1, Ordering::Relaxed) < 8 {
                crate::log::line(format!("island page  {url}  (not an Oczi page)"));
            }
        }
    }
}

/// One stage of a file drag, in the shape the front end's `onDragDrop` expects.
#[derive(Clone, serde::Serialize)]
struct DragEvent {
    #[serde(rename = "type")]
    kind: &'static str,
    paths: Vec<String>,
}

static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();
/// Raw pointer to the leaked `DropTarget`. A COM interface is neither `Send` nor
/// `Sync`, so only the `usize` is stored here and the interface is rebuilt on the
/// (main) thread that actually registers it.
static DROP_TARGET_PTR: AtomicUsize = AtomicUsize::new(0);

/// True while the left mouse button is held, the only signal we get that a
/// drag might be in flight before it reaches the window.
fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

fn monitor_contains(m: &Monitor, x: f64, y: f64) -> bool {
    let p = m.position();
    let s = m.size();
    x >= p.x as f64
        && x < (p.x + s.width as i32) as f64
        && y >= p.y as f64
        && y < (p.y + s.height as i32) as f64
}

/// The display the island lives on: the primary one, or the one under the cursor.
fn target_monitor(app: &AppHandle, pref: &str) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    if pref == "cursor" {
        if let Some((cx, cy)) = cursor_physical() {
            if let Some(m) = monitors.iter().find(|m| monitor_contains(m, cx, cy)) {
                return Some(m.clone());
            }
        }
    }
    app.primary_monitor()
        .ok()
        .flatten()
        .or_else(|| monitors.into_iter().next())
}

pub fn screen_info(app: &AppHandle, pref: &str) -> ScreenInfo {
    match target_monitor(app, pref) {
        Some(m) => {
            let scale = m.scale_factor();
            let p = m.position();
            let s = m.size();
            ScreenInfo {
                x: p.x as f64 / scale,
                y: p.y as f64 / scale,
                width: s.width as f64 / scale,
                height: s.height as f64 / scale,
                scale,
            }
        }
        None => ScreenInfo { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 },
    }
}

/// Places and sizes the window on the chosen display. The size is constant, 
/// see PANEL_W, so this only ever moves it.
pub fn apply_geometry(app: &AppHandle, pref: &str) {
    let Some(win) = window(app) else { return };
    let Some(m) = target_monitor(app, pref) else { return };

    let scale = m.scale_factor();
    let mp = *m.position();
    let ms = *m.size();

    let pw = (PANEL_W * scale).round().max(1.0) as u32;
    let ph = (PANEL_H * scale).round().max(1.0) as u32;
    let x = mp.x + (ms.width as i32 - pw as i32) / 2;
    let y = mp.y;

    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_position(PhysicalPosition::new(x, y));
    // Moving across displays can rescale the window: re-assert the physical size.
    let _ = win.set_size(PhysicalSize::new(pw, ph));
    let _ = win.set_always_on_top(true);
}

fn hwnd_of(win: &WebviewWindow) -> Option<HWND> {
    let raw = win.hwnd().ok()?.0 as isize;
    if raw == 0 {
        return None;
    }
    Some(HWND(raw as *mut _))
}

/// WS_EX_NOACTIVATE keeps clicks from stealing focus; WS_EX_TOOLWINDOW keeps the
/// island out of Alt-Tab.
pub fn make_non_activating(win: &WebviewWindow) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Temporarily allow activation so a text field inside the island can be typed in.
pub fn set_activating(win: &WebviewWindow, activating: bool) {
    let Some(hwnd) = hwnd_of(win) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let want = if activating {
            ex & !(WS_EX_NOACTIVATE.0 as isize)
        } else {
            ex | WS_EX_NOACTIVATE.0 as isize
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, want);
    }
}

/// Position, size and scale of the monitor the island lives on. Any change here
/// means the island has to be placed again.
fn current_screen_key(app: &AppHandle) -> Option<(i32, i32, u32, u32, u64)> {
    let pref = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().screen.clone())
        .unwrap_or_else(|| "primary".into());
    let m = target_monitor(app, &pref)?;
    let p = m.position();
    let size = m.size();
    Some((p.x, p.y, size.width, size.height, m.scale_factor().to_bits()))
}

// Emits `cursor` (window-logical coordinates) and owns the click-through flag.
pub fn spawn_cursor_poll(app: AppHandle, gate: Arc<PollGate>) {
    std::thread::spawn(move || {
        let mut was_down = false;
        // Remembered across ticks so a display change while hidden is noticed the
        // moment the island comes back.
        let mut last_screen: Option<(i32, i32, u32, u32, u64)> = None;
        let mut last = (f64::MIN, f64::MIN);
        let mut ticks: u32 = 0;
        let mut drag_probe: u32 = 0;
        loop {
            let idle = !gate.is_active() || gate.collapsed.load(Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(if idle {
                POLL_HZ_IDLE
            } else {
                POLL_HZ_ACTIVE
            }));
            ticks = ticks.wrapping_add(1);

            let check_every = if idle { 8 } else { 30 };
            if ticks % check_every == 0 {
                if let Some(win) = window(&app) {
                    keep_on_its_own_page(&win);
                }
                let now = current_screen_key(&app);
                if now.is_some() && now != last_screen {
                    let first = last_screen.is_none();
                    last_screen = now;
                    if !first {
                        crate::log::line("display layout changed — repositioning".to_string());
                        let _ = app.emit_to(WINDOW_LABEL, "screen-changed", ());
                    }
                }
            }

            let Some(win) = window(&app) else { continue };
            let Ok(origin) = win.outer_position() else { continue };
            let scale = win.scale_factor().unwrap_or(1.0);
            let Some((cx, cy)) = cursor_physical() else { continue };
            let x = (cx - origin.x as f64) / scale;
            let y = (cy - origin.y as f64) / scale;
            let size = match win.inner_size() {
                Ok(s) => (s.width as f64 / scale, s.height as f64 / scale),
                Err(_) => (PANEL_W, PANEL_H),
            };

            let r = *gate.rect.lock().unwrap();
            let on_island = r.w > 0.0
                && x >= r.x - HIT_MARGIN
                && x <= r.x + r.w + HIT_MARGIN
                && y >= r.y - HIT_MARGIN
                && y <= r.y + r.h + HIT_MARGIN;

            let down = left_button_down();
            if down && !was_down {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || own_file_drops(&handle));
                if !on_island && !idle {
                    let _ = app.emit_to(WINDOW_LABEL, "click-outside", ());
                }
            }
            was_down = down;

            let dragging = down && x >= 0.0 && x <= size.0 && y >= 0.0 && y <= size.1;

            let accept = on_island || dragging;

            // Re-assert the drop target for as long as a button is held, not just on
            // the press: WebView2 recreates its render widget from time to time and it
            // comes back with its own drop target attached.
            if down {
                drag_probe = drag_probe.wrapping_add(1);
                if drag_probe % 4 == 0 {
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || own_file_drops(&handle));
                }
            } else {
                drag_probe = 0;
            }

            // Note this runs before the "did the cursor move" shortcut below: a
            // cursor parked inside the wake band while the island hides under it
            // must still flip the flag, or the hover never lands.
            if gate.ignoring.load(Ordering::Relaxed) == accept {
                gate.ignoring.store(!accept, Ordering::Relaxed);
                let _ = win.set_ignore_cursor_events(!accept);
            }

            if (x - last.0).abs() < 1.0 && (y - last.1).abs() < 1.0 {
                continue;
            }
            last = (x, y);

            let _ = win.emit("cursor", CursorPayload { x, y });
        }
    });
}

// The summon hotkey (Ctrl+Alt+M by default) and the snip hotkey (Ctrl+Alt+Shift+S, fixed).
pub fn spawn_hotkey(app: AppHandle, initial: String) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_S,
    };
    use windows::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_REMOVE, WM_HOTKEY};

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    HOTKEY_TX.get_or_init(|| std::sync::Mutex::new(Some(tx)));

    std::thread::spawn(move || unsafe {
        let snip = RegisterHotKey(
            None,
            HOTKEY_SNIP_ID,
            MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
            VK_S.0 as u32,
        )
        .is_ok();

        let mut current: Option<String> = None;
        let apply = |hotkey: &str, current: &mut Option<String>| {
            if current.as_deref() == Some(hotkey) {
                return;
            }
            if current.is_some() {
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            }
            let registered = parse_hotkey(hotkey)
                .map(|(mods, vk)| RegisterHotKey(None, HOTKEY_ID, mods, vk).is_ok())
                .unwrap_or(false);
            *current = Some(hotkey.to_string());
            crate::log::line(format!(
                "hotkeys — {hotkey} {} · Ctrl+Alt+Shift+S {}",
                if registered { "ready" } else { "taken" },
                if snip { "ready" } else { "taken" },
            ));
        };
        apply(&initial, &mut current);

        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message != WM_HOTKEY {
                    continue;
                }
                match msg.wParam.0 as i32 {
                    HOTKEY_SNIP_ID => {
                        let _ = app.emit_to(WINDOW_LABEL, "hotkey-snip", ());
                    }
                    _ => {
                        let _ = app.emit_to(WINDOW_LABEL, "hotkey", ());
                    }
                }
            }
            match rx.try_recv() {
                Ok(hotkey) => apply(&hotkey, &mut current),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                Err(_) => {}
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    });
}

/// "Ctrl+Alt+M" → the RegisterHotKey modifiers and virtual key. Unknown chords
/// are refused, the settings window only offers the ones listed here, and a
/// hand-edited settings.json with anything else falls back to the default.
pub fn parse_hotkey(hotkey: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, VK_G, VK_M, VK_SPACE,
    };
    let mut mods = MOD_NOREPEAT;
    let mut vk: Option<u32> = None;
    for part in hotkey.split('+') {
        match part.trim() {
            "Ctrl" => mods |= MOD_CONTROL,
            "Alt" => mods |= MOD_ALT,
            "Shift" => mods |= MOD_SHIFT,
            "M" => vk = Some(VK_M.0 as u32),
            "G" => vk = Some(VK_G.0 as u32),
            "Space" => vk = Some(VK_SPACE.0 as u32),
            _ => return None,
        }
    }
    Some((mods, vk?))
}

/// Called from `save_settings` so a changed chord takes effect immediately.
pub fn update_hotkey(hotkey: &str) {
    if let Some(tx) = HOTKEY_TX.get() {
        let tx = tx.lock().unwrap();
        if let Some(tx) = tx.as_ref() {
            let _ = tx.send(hotkey.to_string());
        }
    }
}

static HOTKEY_TX: std::sync::OnceLock<std::sync::Mutex<Option<std::sync::mpsc::Sender<String>>>> =
    std::sync::OnceLock::new();
