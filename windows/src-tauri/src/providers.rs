// The chat provider table. Oczi answers through DeepSeek (paid per token) or Google's
// Gemini API (a real free tier on an OpenAI-compatible endpoint). One table, read by the
// client, the settings validator and the settings window's dropdown, so the three can
// never drift apart.
//
// The model allowlist is per provider on purpose: a single list for the whole app
// rewrites the *other* provider's model to this one's default on the next load, and the
// switch then looks broken with nothing in the UI to explain it.

use serde_json::{json, Value};

/// How a provider turns reasoning down.
#[derive(Clone, Copy)]
pub enum Reasoning {
    /// DeepSeek takes an explicit object, and `disabled` is a real mode.
    ThinkingSwitch,
    /// Gemini 3.x cannot stop thinking at all — only pick a level — so the cheapest
    /// level it accepts stands in for "off". `minimal` is rejected outright by some of
    /// its models, which is why the floor is `low` and not `minimal`.
    EffortLevel,
}

pub struct Provider {
    pub id: &'static str,
    /// Brand name, identical in every language, so it is not an i18n key.
    pub label: &'static str,
    pub endpoint: &'static str,
    /// Entry name in the Windows Credential Manager.
    pub key_name: &'static str,
    pub models: &'static [&'static str],
    /// The first entry of `models`; what a fresh install and a provider switch use.
    pub default_model: &'static str,
    pub reasoning: Reasoning,
}

/// Model IDs from api-docs.deepseek.com; the retired `deepseek-chat` /
/// `deepseek-reasoner` aliases now error, and thinking is a per-request switch.
pub const DEEPSEEK: Provider = Provider {
    id: "deepseek",
    label: "DeepSeek",
    endpoint: "https://api.deepseek.com/chat/completions",
    key_name: "deepseek-api-key",
    models: &["deepseek-flash", "deepseek-v4-pro"],
    default_model: "deepseek-flash",
    reasoning: Reasoning::ThinkingSwitch,
};

/// ID from ai.google.dev/gemini-api/docs/models (read 2026-10-22). One model on purpose: it is
/// the only one that answered the app's real body every time it was tried — 1.2–2.0 s, while
/// 3.1-flash-lite and 3.8-flash returned 503 at capacity and 3.5-flash-lite needed 8–22 s, long
/// enough in front of the notch to read as a hung app. That means no fallback when this one is
/// at capacity or its daily quota runs out: the chat then retries three times and says so.
/// Adding a second name back is a one-line change in this list. The 2.5 family is restricted to
/// projects that already used it — never start on it.
pub const GEMINI: Provider = Provider {
    id: "gemini",
    label: "Gemini",
    endpoint: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
    key_name: "gemini-api-key",
    models: &["gemini-3.6-flash"],
    default_model: "gemini-3.6-flash",
    reasoning: Reasoning::EffortLevel,
};

pub const PROVIDERS: [&Provider; 2] = [&DEEPSEEK, &GEMINI];

/// The provider a fresh install talks to. DeepSeek stays the default until Gemini has
/// been exercised with a real key: it is the one a user falls back on.
pub const DEFAULT_PROVIDER: &str = DEEPSEEK.id;

/// The provider with this id, or the default one — an unknown id in settings.json is not
/// an error worth showing anyone.
pub fn get(id: &str) -> &'static Provider {
    PROVIDERS
        .iter()
        .find(|provider| provider.id == id)
        .copied()
        .unwrap_or(&DEEPSEEK)
}

/// Whether this model belongs to this provider. A stored model from the other provider
/// is replaced on load rather than 400-ing on every chat.
pub fn model_ok(id: &str, model: &str) -> bool {
    get(id).models.contains(&model)
}

/// The next model in this provider's list, wrapping around. A model answering 503 is out of
/// capacity for everyone on the free tier, and a 429 is its own per-model quota — in both cases
/// the next name in the table is the only other thing the app may send.
pub fn next_model(provider: &Provider, current: &str) -> &'static str {
    let models = provider.models;
    match models.iter().position(|model| *model == current) {
        Some(at) => models[(at + 1) % models.len()],
        None => provider.default_model,
    }
}

/// Adds the provider's own reasoning field to a request body. Both are sent explicitly:
/// left out, Gemini picks the model's default level (slow enough to be felt on a glance
/// from the notch) and DeepSeek's flash model thinks by default.
pub fn apply_reasoning(body: &mut Value, provider: &Provider, thinking: bool) {
    match provider.reasoning {
        Reasoning::ThinkingSwitch => {
            body["thinking"] = json!({ "type": if thinking { "enabled" } else { "disabled" } });
        }
        Reasoning::EffortLevel => {
            // Gemini 3.x has no "off": the cheapest level stands in for it. `minimal` is the
            // value some Lite models refuse outright — a 400, not a downgrade — so
            // `deepseek::call_once` walks down to `low` when that happens.
            body["reasoning_effort"] = json!(if thinking { "high" } else { "minimal" });
        }
    }
}

/// The provider's own sentence out of a failed response.
///
/// Some vendors wrap the error object in a one-element array — Gemini does — and a reader
/// written for the plain OpenAI shape degrades silently on that: a bad key then shows two
/// hundred characters of raw JSON instead of the sentence that explains it. Object first,
/// then the first array element, and `None` when neither shape carries a message.
pub fn error_detail(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let error = match value.get("error") {
        Some(_) => Some(&value),
        None => value.as_array()?.first(),
    }?;
    error
        .get("error")?
        .get("message")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_errors_are_read_out_of_the_array_shape() {
        let text = r#"[{"error":{"code":400,"message":"Please pass a valid API key","status":"INVALID_ARGUMENT"}}]"#;
        assert_eq!(
            error_detail(text).as_deref(),
            Some("Please pass a valid API key")
        );
    }

    #[test]
    fn openai_shaped_errors_still_work() {
        let text = r#"{"error":{"message":"Insufficient Balance","type":"authentication_error"}}"#;
        assert_eq!(error_detail(text).as_deref(), Some("Insufficient Balance"));
    }

    #[test]
    fn text_that_is_not_json_has_no_detail() {
        assert_eq!(error_detail("<html>502</html>"), None);
    }

    #[test]
    fn a_model_belongs_to_one_provider_only() {
        assert!(model_ok("deepseek", "deepseek-flash"));
        assert!(!model_ok("deepseek", "gemini-3.6-flash"));
        assert!(model_ok("gemini", "gemini-3.6-flash"));
        assert!(!model_ok("gemini", "deepseek-flash"));
    }

    #[test]
    fn every_default_model_is_in_its_own_allowlist() {
        for provider in PROVIDERS {
            assert_eq!(provider.models.first().copied(), Some(provider.default_model));
            assert!(model_ok(provider.id, provider.default_model));
        }
    }

    #[test]
    fn the_next_model_walks_the_table_and_wraps() {
        // One model per provider is a valid table: the walk then stays on it.
        assert_eq!(next_model(&GEMINI, "gemini-3.6-flash"), "gemini-3.6-flash");
        assert_eq!(next_model(&DEEPSEEK, "deepseek-flash"), "deepseek-v4-pro");
        assert_eq!(next_model(&DEEPSEEK, "deepseek-v4-pro"), "deepseek-flash");
        // An unknown name is not a panic: the default is a model the provider serves.
        assert_eq!(next_model(&GEMINI, "nonsense"), GEMINI.default_model);
    }

    #[test]
    fn reasoning_is_translated_per_provider() {
        let mut gemini = json!({});
        apply_reasoning(&mut gemini, &GEMINI, false);
        assert_eq!(gemini["reasoning_effort"], json!("minimal"));
        assert!(gemini.get("thinking").is_none());

        let mut deepseek = json!({});
        apply_reasoning(&mut deepseek, &DEEPSEEK, true);
        assert_eq!(deepseek["thinking"]["type"], json!("enabled"));
        assert!(deepseek.get("reasoning_effort").is_none());
    }
}
