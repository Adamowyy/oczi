// Voice dictation, the offline half of it: a recording made in the island comes here as
// 16 kHz mono samples and leaves as text.
//
// Why this module exists at all, and why it looks like this:
//
// * The microphone is captured in the island, not here. A webview reaches the device with
//   `getUserMedia` in about ten lines; doing it natively would mean WASAPI, a resampler and
//   a WAV writer — several hundred lines of audio plumbing for the same samples.
// * The speech engine is whisper.cpp, linked straight into this binary through whisper-rs.
//   No service, no API key, no per-use cost and nothing to keep alive: the model is a file,
//   and the transcription happens on this machine while the app is running. A cloud service
//   was rejected on purpose — asking anyone who installs Oczi to open a second paid account
//   is how a feature stops being used.
// * Only compiled in the `voice` feature: the ordinary installer must not carry a speech
//   engine it never calls, nor the model file.
//
// The model is loaded per recording instead of being held open. Loading costs ~140 ms here
// while a resident context costs several hundred megabytes, and Oczi lives in the corner of
// the screen all day — the memory matters more than the milliseconds.

use std::path::PathBuf;
use std::time::Instant;

use crate::log;

/// The model the voice installer ships. Sizes are the ones this machine measured
/// (Ryzen 9 5900HX, 8 threads, greedy): small f16 needs ~3.2 s for a sentence with
/// automatic language detection and ~1.8 s with the language pinned. `medium` was twice
/// as accurate on names but took 9.5 s, and `large-v3-turbo` took 19-24 s, so neither is
/// usable for dictation on a CPU. Bigger models can still be dropped in by hand: see
/// `model_path`.
pub const MODEL_FILE: &str = "ggml-small.bin";

/// How the recording is handed over: 16 kHz mono, because that is what Whisper was trained
/// on and whisper.cpp resamples nothing.
pub const SAMPLE_RATE: u32 = 16_000;

/// Why a transcription did not happen. The island turns each one into a sentence in the
/// user's language — these strings are codes, not text, so the UI owns the wording and
/// both language tables stay complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// No model file anywhere we look.
    NoModel,
    /// The recording was too short to hold speech, or too long for one pass.
    BadAudio,
    /// whisper.cpp refused to load the model or failed mid-run.
    Engine,
}

impl Fault {
    pub fn code(self) -> &'static str {
        match self {
            Fault::NoModel => "no-model",
            Fault::BadAudio => "bad-audio",
            Fault::Engine => "engine",
        }
    }
}

/// Where the model can be found, in order of who wins:
///
/// 1. `%APPDATA%\Oczi\models\<file>` — a copy the user dropped there. This is how a bigger
///    or a language-tuned model is tried without rebuilding anything.
/// 2. next to the executable — where the voice installer unpacks it.
/// 3. `<exe dir>\models\<file>` — the layout of a development build.
///
/// `OCZI_WHISPER_MODEL` overrides all three, which is what the headless check uses.
pub fn model_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OCZI_WHISPER_MODEL") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let mut candidates = Vec::new();
    candidates.push(crate::settings::config_dir().join("models").join(MODEL_FILE));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(MODEL_FILE));
            candidates.push(dir.join("models").join(MODEL_FILE));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Threads for the encoder. Half the logical cores, capped at eight: past that whisper.cpp
/// stops getting faster and starts fighting the rest of the desktop for the CPU — and this
/// runs on a machine the user is still typing on.
pub fn threads() -> i32 {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    ((logical / 2).max(1) as i32).min(8)
}

/// Language for one run. `auto` is the default and costs a second encoder pass, which is
/// worth it: a wrong pinned language makes Whisper *translate* Polish into English rather
/// than transcribe it, and that looks like a broken microphone rather than a setting.
pub fn language(setting: &str) -> &'static str {
    match setting {
        "pl" => "pl",
        "en" => "en",
        _ => "auto",
    }
}

/// What the engine is told to expect. Whisper uses this as context, not as an instruction:
/// a short line of the words this app is actually asked for biases the decode towards them,
/// which is what keeps a spoken command from coming back as a near-miss. Polish first,
/// because that is what most dictation is, with the English terms that get mixed in.
// i18n-ok: a vocabulary hint for the engine, never shown to a person.
const PROMPT: &str = "Oczi, uruchom przeglądarkę, otwórz przeglądarkę, znajdź w internecie, zapisz w notatkach, przypomnij mi, ustaw przypomnienie, wyślij wiadomość, otwórz plik, otwórz folder, terminal, polecenie, zadanie, spotkanie, jutro, o której, config, plik, a także: open the browser, run the tests, search the web, write it down";

/// How much of the encoder to run. Whisper's encoder always works on a thirty-second window,
/// so a five-second question pays for thirty — this is the one dial that makes a short
/// dictation cost what it is actually worth. A shorter context is roughly proportional in
/// time, and the price is accuracy on the opening words, so the floor stays well above the
/// length of a sentence.
///
/// `OCZI_AUDIO_CTX` overrides it, which is how the two were compared on real recordings.
fn audio_ctx(samples: &[f32]) -> i32 {
    if let Ok(forced) = std::env::var("OCZI_AUDIO_CTX") {
        if let Ok(value) = forced.parse::<i32>() {
            return value.clamp(1, 1500);
        }
    }
    let seconds = samples.len() as f32 / SAMPLE_RATE as f32;
    // 1500 positions is the full window; a comfortable margin over the audio's own length,
    // because everything after the end of the audio is where the tail of a sentence lands.
    let scaled = (seconds / 30.0 * 1500.0 * 1.6).round() as i32;
    scaled.clamp(600, 1500)
}

/// The engine, kept loaded between recordings. Building the context means reading the whole
/// model — 466 MB — and costs about 150 ms, which is a sixth of a short dictation and is spent
/// again on every single one. One context serves every call; whisper.cpp states are cheap and
/// are made per recording below.
static CONTEXT: std::sync::OnceLock<whisper_rs::WhisperContext> = std::sync::OnceLock::new();

// Whether the engine heard words at all.
fn is_non_speech(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    let bracketed = (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'));
    if !bracketed || t.chars().count() > 40 {
        return false;
    }
    let inner = t[1..t.len() - 1].trim();
    let letters: Vec<char> = inner.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty() && letters.iter().all(|c| c.is_uppercase())
}

/// Text for a recording, or the reason there is none.
pub fn transcribe(samples: &[f32], setting_language: &str) -> Result<String, Fault> {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    if samples.len() < (SAMPLE_RATE as usize / 2) || samples.len() > SAMPLE_RATE as usize * 120 {
        // Under half a second there is nothing to hear; over two minutes is not a dictation
        // and would hold the encoder for half a minute.
        return Err(Fault::BadAudio);
    }

    let model = model_path().ok_or(Fault::NoModel)?;
    let started = Instant::now();
    if CONTEXT.get().is_none() {
        let ctx = WhisperContext::new_with_params(&model, WhisperContextParameters::default())
            .map_err(|e| {
                log::line(format!("voice  model failed: {e}"));
                Fault::Engine
            })?;
        // A race would only mean a second context being dropped; the stored one wins.
        let _ = CONTEXT.set(ctx);
    }
    let ctx = CONTEXT.get().ok_or(Fault::Engine)?;
    let load_ms = started.elapsed().as_millis();

    let mut state = ctx.create_state().map_err(|e| {
        log::line(format!("voice  state failed: {e}"));
        Fault::Engine
    })?;

    // Greedy decoding, not the library default of beam 5 with 5 candidates: dictation wants
    // the answer now, and the beam search multiplies decode time for a difference nobody
    // can hear in a sentence they just spoke.
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads());
    params.set_language(Some(language(setting_language)));
    params.set_translate(false);
    // Vocabulary, and a second attempt at raising temperature when a window comes out
    // unconvincing: greedy decoding is fast, and the fallback is what gives a garbled
    // sentence another chance at being the sentence that was said.
    params.set_initial_prompt(PROMPT);
    params.set_temperature_inc(0.2);
    // Shorter recordings run a shorter encoder: see `audio_ctx`.
    params.set_audio_ctx(audio_ctx(samples));
    // The library prints progress and timings to stderr; a windowed app has no console, so
    // anything it wants said goes through our own log.
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);

    let started = Instant::now();
    state.full(params, samples).map_err(|e| {
        log::line(format!("voice  run failed: {e}"));
        Fault::Engine
    })?;
    let run_ms = started.elapsed().as_millis();

    let mut text = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(segment) = state.get_segment(i) {
            if let Ok(part) = segment.to_str_lossy() {
                text.push_str(&part);
            }
        }
    }
    let text = text.trim().to_string();
    // Whisper answers audio it cannot hear words in with a marker rather than with nothing —
    // `[BLANK_AUDIO]`, `[MUSIC]`, `[NOISE]` — and sometimes with an invented sentence. A
    // marker sent as a question opens a chat about nothing, so it becomes an empty
    // transcript: the island folds away quietly, which is what a recording with no words in
    // it deserves.
    if is_non_speech(&text) {
        log::line(format!(
            "voice  model={} audio={:.1}s load={load_ms}ms run={run_ms}ms lang={} text=<none>",
            model
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            samples.len() as f32 / SAMPLE_RATE as f32,
            language(setting_language),
        ));
        return Ok(String::new());
    }
    // The line that makes a bad dictation diagnosable later: which model, how long the audio
    // was, what the two phases cost and what came out.
    log::line(format!(
        "voice  model={} audio={:.1}s load={load_ms}ms run={run_ms}ms lang={} text={}",
        model
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        samples.len() as f32 / SAMPLE_RATE as f32,
        language(setting_language),
        text.chars().take(120).collect::<String>()
    ));
    Ok(text)
}

/// Text for a recording that arrives as raw little-endian floats over IPC, in the language
/// the user pinned — or auto-detected, which costs a second pass through the encoder.
///
/// The island sends the samples this way rather than as JSON: two minutes of audio is
/// eight megabytes of f32, and a JSON array of that is both larger and slower to parse on
/// both sides. Anything that is not a whole number of floats is a bug on the other end.
pub fn transcribe_pcm(bytes: &[u8], setting_language: &str) -> Result<String, Fault> {
    if bytes.len() % 4 != 0 {
        return Err(Fault::BadAudio);
    }
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    transcribe(&samples, setting_language)
}

// 16 kHz mono 16-bit PCM out of a RIFF file, as one channel of floats in [-1, 1).
pub fn wav_16k_mono(bytes: &[u8]) -> Result<Vec<f32>, Fault> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Fault::BadAudio);
    }
    let mut pos = 12usize;
    let (mut seen_fmt, mut samples) = (false, Vec::new());
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size =
            u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]])
                as usize;
        let body = pos + 8;
        match id {
            b"fmt " if body + 16 <= bytes.len() => {
                let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
                let rate = u32::from_le_bytes([
                    bytes[body + 4],
                    bytes[body + 5],
                    bytes[body + 6],
                    bytes[body + 7],
                ]);
                let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
                if channels != 1 || rate != SAMPLE_RATE || bits != 16 {
                    return Err(Fault::BadAudio);
                }
                seen_fmt = true;
            }
            b"data" => {
                let end = (body + size).min(bytes.len());
                samples.extend(
                    bytes[body..end]
                        .chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0),
                );
            }
            _ => {}
        }
        pos = body + size + (size & 1);
    }
    if !seen_fmt {
        return Err(Fault::BadAudio);
    }
    Ok(samples)
}
