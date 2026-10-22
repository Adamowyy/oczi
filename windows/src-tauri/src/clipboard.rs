// What a pin hands over: its pixels, or text read out of them.

use std::path::Path;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

const CF_DIB: u32 = 8;
const CF_UNICODETEXT: u32 = 13;

/// Puts text on the clipboard. The block is the clipboard's once handed over, so it is never freed
/// here; closing the clipboard has to happen on every path.
pub fn set_text(text: &str) -> Result<(), String> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    let size = utf16.len() * 2;

    unsafe {
        OpenClipboard(None).map_err(|e| format!("could not open the clipboard: {e}"))?;
        let outcome = (|| {
            EmptyClipboard().map_err(|e| format!("could not clear the clipboard: {e}"))?;
            let block =
                GlobalAlloc(GMEM_MOVEABLE, size).map_err(|e| format!("could not allocate: {e}"))?;
            let bytes = std::slice::from_raw_parts(utf16.as_ptr() as *const u8, size);
            copy_into(block, bytes)?;
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(block.0)))
                .map_err(|e| format!("could not set the text: {e}"))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        outcome
    }
}

/// Puts the pin's picture on the clipboard as a DIB, the format a paste target expects.
pub fn set_image(path: &Path) -> Result<(), String> {
    let dib = dib_bits(path)?;
    unsafe {
        OpenClipboard(None).map_err(|e| format!("could not open the clipboard: {e}"))?;
        let outcome = (|| {
            EmptyClipboard().map_err(|e| format!("could not clear the clipboard: {e}"))?;
            let block = GlobalAlloc(GMEM_MOVEABLE, dib.len())
                .map_err(|e| format!("could not allocate: {e}"))?;
            copy_into(block, &dib)?;
            SetClipboardData(CF_DIB, Some(HANDLE(block.0)))
                .map_err(|e| format!("could not set the picture: {e}"))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        outcome
    }
}

unsafe fn copy_into(block: HGLOBAL, bytes: &[u8]) -> Result<(), String> {
    let ptr = GlobalLock(block);
    if ptr.is_null() {
        return Err("could not lock the block".into());
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len());
    let _ = GlobalUnlock(block);
    Ok(())
}

/// A `BITMAPINFOHEADER` and 32-bit BGRA rows, bottom-up. `snip.rs` writes RGBA, so the channels
/// are swapped and the rows walked backwards.
fn dib_bits(path: &Path) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("could not read the pin: {e}"))?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("could not read the picture: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("could not decode the picture: {e}"))?;
    if info.color_type != png::ColorType::Rgba {
        return Err("the pin is not a colour picture".into());
    }
    let (w, h) = (info.width as i32, info.height as i32);
    let stride = (w * 4) as usize;

    let mut out = Vec::with_capacity(40 + stride * h as usize);
    out.extend_from_slice(&40u32.to_le_bytes()); // biSize
    out.extend_from_slice(&w.to_le_bytes()); // biWidth
    out.extend_from_slice(&h.to_le_bytes()); // biHeight, positive = bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    out.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(stride * h as usize).to_le_bytes() as &[u8]); // biSizeImage
    out.extend_from_slice(&2835i32.to_le_bytes()); // biXPelsPerMeter
    out.extend_from_slice(&2835i32.to_le_bytes()); // biYPelsPerMeter
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    for row in (0..h).rev() {
        let start = (row as usize) * stride;
        for pixel in buf[start..start + stride].chunks_exact(4) {
            out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    Ok(out)
}
