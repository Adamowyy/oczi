// Reading text out of a pinned picture with the OCR engine Windows ships. No model to download.

use std::path::Path;

use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

/// The text a picture holds, newlines kept as the engine laid them out. Empty means no words.
pub fn text_from_png(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("could not read the pin: {e}"))?;
    let engine = engine()?;

    unsafe {
        // A worker thread has no apartment; MTA because nothing here pumps messages.
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
        if let Err(err) = RoInitialize(RO_INIT_MULTITHREADED) {
            // A thread already in another apartment is fine to carry on from.
            if err.code() != windows::Win32::Foundation::RPC_E_CHANGED_MODE {
                return Err(format!("could not start WinRT: {err}"));
            }
        }

        // The encoded bytes go in as they are: the system decoder reads PNG itself.
        let stream = InMemoryRandomAccessStream::new()
            .map_err(|e| format!("could not open a memory stream: {e}"))?;
        let writer = DataWriter::CreateDataWriter(&stream)
            .map_err(|e| format!("could not write the picture: {e}"))?;
        writer
            .WriteBytes(&bytes)
            .map_err(|e| format!("could not write the picture: {e}"))?;
        writer
            .StoreAsync()
            .and_then(|op| op.get())
            .map_err(|e| format!("could not write the picture: {e}"))?;
        writer
            .FlushAsync()
            .and_then(|op| op.get())
            .map_err(|e| format!("could not write the picture: {e}"))?;
        stream
            .Seek(0)
            .map_err(|e| format!("could not rewind the picture: {e}"))?;

        let decoder = BitmapDecoder::CreateAsync(&stream)
            .and_then(|op| op.get())
            .map_err(|e| format!("could not decode the picture: {e}"))?;
        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .and_then(|op| op.get())
            .map_err(|e| format!("could not decode the picture: {e}"))?;

        let result = engine
            .RecognizeAsync(&bitmap)
            .and_then(|op| op.get())
            .map_err(|e| format!("could not read the picture: {e}"))?;
        let text = result
            .Text()
            .map_err(|e| format!("could not read the picture: {e}"))?;
        Ok(text.to_string_lossy().trim().to_string())
    }
}

/// Polish first, then the profile's language, then English.
fn engine() -> Result<OcrEngine, String> {
    if let Ok(language) = Language::CreateLanguage(&HSTRING::from("pl")) {
        if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
            return Ok(engine);
        }
    }
    if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
        return Ok(engine);
    }
    if let Ok(language) = Language::CreateLanguage(&HSTRING::from("en-US")) {
        if let Ok(engine) = OcrEngine::TryCreateFromLanguage(&language) {
            return Ok(engine);
        }
    }
    Err("this machine has no OCR language installed".into())
}
