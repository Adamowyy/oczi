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
    // Flash only: it answers everything Oczi asks of it, and the slower model is a second
    // thing to choose with no visible gain in the card's few lines.
    models: &["deepseek-flash"],
    default_model: "deepseek-flash",
    reasoning: Reasoning::ThinkingSwitch,
};

/// Tried in this order. The free tier meters each model separately: 3.6-flash gives 20 a day,
/// the Lite model behind it far more. Measured: 0.7 s, 1.2 s, and 3.8-flash needs the `low` retry.
pub const GEMINI: Provider = Provider {
    id: "gemini",
    label: "Gemini",
    endpoint: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
    key_name: "gemini-api-key",
    models: &["gemini-3.5-flash-lite", "gemini-3.1-flash-lite"],
    default_model: "gemini-3.5-flash-lite",
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

/// The next model this call has not tried yet, `None` once the table is exhausted.
pub fn next_untried(provider: &Provider, tried: &[String]) -> Option<&'static str> {
    provider
        .models
        .iter()
        .copied()
        .find(|model| !tried.iter().any(|t| t.eq_ignore_ascii_case(model)))
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
            // Gemini has no "off", only levels, and the card has no button for it: the cheapest
            // level is always what goes out. A level it cannot take is a 400, not a downgrade, so
            // `deepseek::call_once` walks down to `low` when that happens.
            let _ = thinking;
            body["reasoning_effort"] = json!("minimal");
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

/// The size of the daily allowance a 429 says is spent, when it says one is.
///
/// A free tier on Gemini is an allowance per day, and the body spells it out — "Quota
/// exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests,
/// limit: 20" — while a per-minute limit carries `_per_minute` in the same place and clears
/// itself by the time the user has typed the next question. Only the daily one is worth
/// telling the user about: waiting two seconds for it is time nobody gets back, and the
/// app exists to be glanced at. `None` means "not a spent allowance", so the caller keeps
/// its normal retry.
pub fn daily_quota_spent(text: &str) -> Option<u32> {
    let lower = text.to_ascii_lowercase();
    if !lower.contains("quota") || lower.contains("per_minute") {
        return None;
    }
    let at = lower.find("limit:")? + "limit:".len();
    let digits: String = lower[at..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(char::is_ascii_digit)
        .collect();
    Some(digits.parse().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_spent_daily_allowance_is_read_with_its_size() {
        // The body Gemini actually sends for a free-tier key that has used its day up.
        let text = r#"[{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details. * Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 20, model: gemini-3.6-flash, description: Generate content requests.","status":"RESOURCE_EXHAUSTED"}}]"#;
        assert_eq!(daily_quota_spent(text), Some(20));
    }

    #[test]
    fn a_per_minute_limit_is_not_a_spent_allowance() {
        let text = r#"{"error":{"message":"Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests_per_minute, limit: 10}}"#;
        assert_eq!(daily_quota_spent(text), None);
    }

    #[test]
    fn failures_that_are_not_quota_errors_are_left_to_the_retry() {
        assert_eq!(daily_quota_spent(r#"{"error":{"message":"Insufficient Balance"}}"#), None);
        assert_eq!(daily_quota_spent("upstream connect error"), None);
    }

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
        assert!(!model_ok("deepseek", GEMINI.models[0]));
        assert!(model_ok("gemini", GEMINI.models[0]));
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
        let none: Vec<String> = Vec::new();
        // Read from the table, so a model renamed in it does not leave a stale name in here.
        let (first, second) = (GEMINI.models[0], GEMINI.models[1]);
        // A turn starts on the default, then the walk moves through the rest and stops.
        assert_eq!(next_untried(&GEMINI, &none), Some(first));
        let one = vec![first.to_string()];
        assert_eq!(next_untried(&GEMINI, &one), Some(second));
        // Nothing left to try is a `None`, not a wrap back to a name that just refused: on a free
        // tier that wrap is what turns "its day is spent" into three more attempts and no answer.
        let all: Vec<String> = GEMINI.models.iter().map(|model| model.to_string()).collect();
        assert_eq!(next_untried(&GEMINI, &all), None);
        // A name the provider does not serve is not a model this call has used up.
        let nonsense = vec!["nonsense".to_string()];
        assert_eq!(next_untried(&GEMINI, &nonsense), Some(first));
        // DeepSeek has one model, so its walk ends at once.
        assert_eq!(next_untried(&DEEPSEEK, &["deepseek-flash".to_string()]), None);
        assert_eq!(DEEPSEEK.models, &["deepseek-flash"]);
    }

    #[test]
    fn the_gemini_table_is_a_budget_not_a_preference_list() {
        // The free tier meters each model separately: 500 requests a day on a Lite model against
        // 20 on the rest, while a turn costs two to four requests. So the table carries Lite models
        // only — a "better" model first in it silently brings the 20-a-day ceiling back.
        assert!(GEMINI.models.len() >= 2);
        for model in GEMINI.models {
            assert!(model.ends_with("-lite"), "{model} is not a Lite model");
        }
        assert_eq!(GEMINI.default_model, GEMINI.models[0]);
    }


    #[test]
    fn reasoning_is_translated_per_provider() {
        let mut gemini = json!({});
        apply_reasoning(&mut gemini, &GEMINI, false);
        assert_eq!(gemini["reasoning_effort"], json!("minimal"));
        assert!(gemini.get("thinking").is_none());
        // The thinking switch is DeepSeek's alone: Gemini takes the cheapest level either way.
        let mut asked = json!({});
        apply_reasoning(&mut asked, &GEMINI, true);
        assert_eq!(asked["reasoning_effort"], json!("minimal"));

        let mut deepseek = json!({});
        apply_reasoning(&mut deepseek, &DEEPSEEK, true);
        assert_eq!(deepseek["thinking"]["type"], json!("enabled"));
        assert!(deepseek.get("reasoning_effort").is_none());
    }
}
