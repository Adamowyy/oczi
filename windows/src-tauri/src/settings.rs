// Preferences, stored as plain JSON in %APPDATA%\Oczi\settings.json.
// No secret ever lands here, API keys live in the Windows Credential Manager.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    /// Seconds the small bar waits before it hides, once the cursor has left it.
    #[serde(default = "default_notch_hide")]
    pub notch_hide_interval: f64,
    /// Where the island sits along the top of its screen: 0 = flush left, 0.5 the
    /// middle, 1 = flush right of the working area. Dragged into place and kept.
    #[serde(default = "default_anchor")]
    pub island_anchor: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    /// DeepSeek model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// Thinking mode. Off answers in about a second; on makes the model reason
    /// first and bill the trace as output tokens.
    #[serde(default)]
    pub thinking: bool,
    /// The chord that summons the island. Only the combinations the settings
    /// window offers are valid; anything else is replaced on load.
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// Whether the chat may search the live web. On by default: the alternative
    /// is a model answering from a training cut-off that has already passed.
    #[serde(default = "default_true")]
    pub web_search: bool,
    /// Which search backend the web tools use, see `crate::web::PROVIDERS`.
    /// The keyless one needs no setup, so it is the default.
    #[serde(default = "default_search_provider")]
    pub search_provider: String,
    /// UI language. English unless the user picked Polish in the settings.
    #[serde(default = "default_language")]
    pub language: String,
    /// Let the chat run commands on this machine. Off unless the user turned it
    /// on and accepted the warning.
    #[serde(default)]
    pub terminal_enabled: bool,
}

fn default_anchor() -> f64 {
    0.5
}

fn default_notch_hide() -> f64 {
    60.0
}

fn default_true() -> bool {
    true
}

fn default_search_provider() -> String {
    crate::web::PROVIDERS[0].to_string()
}

fn default_model() -> String {
    crate::deepseek::DEFAULT_MODEL.to_string()
}

fn default_hotkey() -> String {
    "Ctrl+Alt+M".to_string()
}

/// Languages the UI ships with. Anything else in settings.json becomes English.
pub const LANGUAGES: [&str; 2] = ["en", "pl"];

fn default_language() -> String {
    "en".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            notch_hide_interval: default_notch_hide(),
            island_anchor: default_anchor(),
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            model: default_model(),
            thinking: false,
            hotkey: default_hotkey(),
            web_search: true,
            search_provider: default_search_provider(),
            language: default_language(),
            terminal_enabled: false,
        }
    }
}

/// %APPDATA%\Oczi
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Oczi")
}

/// %LOCALAPPDATA%\Oczi, where the log and the ingested file inbox live.
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Oczi")
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// Model IDs the API currently serves. Anything else in a settings.json, a
/// retired alias like `deepseek-chat`, or the Claude model an older build wrote
///, would 400 on every chat, so it is replaced with the default on load.
pub const KNOWN_MODELS: [&str; 2] = ["deepseek-flash", "deepseek-v4-pro"];

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => {
            let mut settings: Settings = serde_json::from_slice(&bytes).unwrap_or_default();
            if !KNOWN_MODELS.contains(&settings.model.as_str()) {
                settings.model = default_model();
            }
            if crate::island::parse_hotkey(&settings.hotkey).is_none() {
                settings.hotkey = default_hotkey();
            }
            if !crate::web::PROVIDERS.contains(&settings.search_provider.as_str()) {
                settings.search_provider = default_search_provider();
            }
            if !LANGUAGES.contains(&settings.language.as_str()) {
                settings.language = default_language();
            }
            settings
        }
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}
