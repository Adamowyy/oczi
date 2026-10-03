// Region screenshots taken by the app itself. The Windows snipping tool never puts
// its snip on the clipboard here, so we freeze the desktop with GDI and crop ourselves.

use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

use crate::{settings, util};

/// A frozen desktop: BGRA, top-down, `w * h` pixels.
pub struct Frame {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

/// What the island is told about a capture. Dimensions only, the pixels never cross
/// IPC: they go to disk and the chat reads them from there.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnipInfo {
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

impl SnipInfo {
    /// Nothing captured (the user pressed Esc). The front end reads `width == 0`.
    pub const CANCELLED: SnipInfo = SnipInfo {
        width: 0,
        height: 0,
        bytes: 0,
    };
}

/// The frozen desktop (while the overlay is up) and the last saved PNG (until the
/// next question picks it up).
#[derive(Default)]
pub struct Snip {
    frame: Mutex<Option<Frame>>,
    pending: Mutex<Option<PathBuf>>,
}

impl Snip {
    /// Hands the saved screenshot to the chat, once.
    pub fn take(&self) -> Option<PathBuf> {
        self.pending.lock().ok().and_then(|mut p| p.take())
    }

    /// Drops a frozen frame that will never be used, so a cancelled capture does not
    /// keep a full-screen bitmap in memory.
    pub fn discard_frame(&self) {
        if let Ok(mut slot) = self.frame.lock() {
            *slot = None;
        }
    }
}

/// Physical bounds of the whole virtual desktop, in the coordinates BitBlt wants.
pub fn desktop_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Copies the desktop into memory, ready to be cropped.
pub fn grab(owner: &Snip) -> Result<(), String> {
    let (vx, vy, w, h) = desktop_bounds();
    if w <= 0 || h <= 0 {
        return Err("Windows reported an empty desktop".into());
    }
    let mut px = vec![0u8; (w as usize) * (h as usize) * 4];

    unsafe {
        let screen = GetDC(Some(HWND::default()));
        if screen.is_invalid() {
            return Err("could not read the screen".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        // The whole virtual desktop, not just the primary monitor.
        let copied = BitBlt(mem, 0, 0, w, h, Some(screen), vx, vy, SRCCOPY);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // Negative height asks for top-down rows, which is the order the crop
                // walks them in.
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let rows = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(px.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        );

        let _ = SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        let _ = ReleaseDC(Some(HWND::default()), screen);

        copied.map_err(|e| format!("could not copy the screen: {e}"))?;
        if rows != h {
            return Err(format!("the screen came back in {rows} rows, expected {h}"));
        }
    }

    if let Ok(mut slot) = owner.frame.lock() {
        *slot = Some(Frame { w, h, px });
    }
    Ok(())
}

/// Crops the frozen frame and writes it out as a PNG. The rectangle is in physical
/// pixels relative to the desktop origin, which is the space the overlay reports in.
pub fn finish(owner: &Snip, x: i32, y: i32, w: i32, h: i32) -> Result<SnipInfo, String> {
    let frame = owner
        .frame
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .ok_or_else(|| "there is no screenshot to crop".to_string())?;

    if w < 4 || h < 4 {
        return Err("that selection was too small".into());
    }
    // Clamp: a drag can end outside the frame.
    let x = x.clamp(0, frame.w - 1);
    let y = y.clamp(0, frame.h - 1);
    let w = w.min(frame.w - x);
    let h = h.min(frame.h - y);

    let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for row in 0..h {
        let start = (((y + row) as usize) * (frame.w as usize) + x as usize) * 4;
        let src = &frame.px[start..start + (w as usize) * 4];
        for pixel in src.chunks_exact(4) {
            // GDI hands back BGRA; PNG wants RGBA.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }

    let bytes = encode_png(w as u32, h as u32, &rgba)?;
    let dir = settings::local_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let path = dir.join("snip.png");
    std::fs::write(&path, &bytes).map_err(|e| format!("could not save the screenshot: {e}"))?;

    if let Ok(mut slot) = owner.pending.lock() {
        *slot = Some(path);
    }
    Ok(SnipInfo {
        width: w as u32,
        height: h as u32,
        bytes: bytes.len() as u64,
    })
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("could not encode the screenshot: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("could not encode the screenshot: {e}"))?;
    }
    Ok(out)
}

/// The image as a data URL, for the chat request body.
pub fn data_url(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read the screenshot: {e}"))?;
    Ok(format!("data:image/png;base64,{}", util::base64_for(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::{encode_png, SnipInfo};

    #[test]
    fn the_png_encoder_writes_a_png_signature() {
        let rgba = vec![255u8; 2 * 2 * 4];
        let png = encode_png(2, 2, &rgba).expect("encoding a 2x2 image");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn a_cancelled_snip_is_recognisable() {
        assert_eq!(SnipInfo::CANCELLED.width, 0);
    }
}
