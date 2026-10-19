// Game prices: a title in, and the numbers from gg.deals come out — the lowest price now (retail
// and keyshops separately), the historical low, and the page that lists the shops.
//
// Two public services, no scraping anywhere:
//
// * Steam's own store search resolves the title to an App ID and carries the artwork. It needs no
//   key and answers JSON (`store.steampowered.com/api/storesearch`).
// * gg.deals' Prices API does the prices (`api.gg.deals/v1/prices/by-steam-app-id`), and it needs a
//   key the user generates in their own account — so this module reads one from the Credential
//   Manager, the same way the other keyed integrations do, and says so plainly when it is missing.
//
// The API reports prices in the currency the key was issued for and ignores a currency parameter
// (four spellings tried, 2026-10-19), so the currency code is shown next to every number instead of
// being converted: an invented exchange rate would be a worse lie than a PLN number on a USD key.

use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use crate::island::WINDOW_LABEL;
use crate::log;

const STEAM_SEARCH: &str = "https://store.steampowered.com/api/storesearch/";
const GG_PRICES: &str = "https://api.gg.deals/v1/prices/by-steam-app-id/";
const GG_KEY: &str = "ggdeals-api-key";
const GG_REGION: &str = "ggdeals-region";
/// The regions the API accepts. A region is what decides the currency: without one the answer is
/// always in USD, which is why a Polish account saw dollars until `region=pl` was sent.
const REGIONS: &[&str] = &[
    "au", "be", "br", "ca", "ch", "de", "dk", "es", "eu", "fi", "fr", "gb", "ie", "it", "nl", "no",
    "pl", "se", "us",
];
/// The artwork's home. A `header.jpg` is 460×215 and exists for every store app.
const STEAM_IMAGE: &str = "https://cdn.cloudflare.steamstatic.com/steam/apps";

/// One game's prices, in the shape the card reads.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub title: String,
    /// The Steam App ID the price was looked up by — worth showing when a search was ambiguous.
    pub app_id: u64,
    pub image: Option<String>,
    pub currency: String,
    /// The lowest price right now, whichever of the two sources is cheaper.
    pub best: Option<String>,
    /// Which of them that was: `retail` or `keyshops`.
    pub best_from: Option<String>,
    pub retail: Option<String>,
    pub keyshops: Option<String>,
    pub historical_retail: Option<String>,
    pub historical_keyshops: Option<String>,
    /// The gg.deals page for this game, where the shops themselves are listed. The API returns no
    /// shop address, so this is the closest thing to \"buy it here\" it can offer.
    pub url: Option<String>,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(crate::web::USER_AGENT)
        .build()
        .map_err(|e| format!("HTTP: {e}"))
}

/// Which region's prices to ask for. The user's own choice wins, then the interface language (a
/// Polish interface is a Polish user, and Polish prices), and the USA otherwise — the API's default
/// is USD for everyone, which is exactly the answer a European user did not ask for.
fn region() -> String {
    if let Some(stored) = crate::secrets::get(GG_REGION) {
        let stored = stored.trim().to_lowercase();
        if REGIONS.contains(&stored.as_str()) {
            return stored;
        }
    }
    if crate::settings::load().language == "pl" {
        "pl".to_string()
    } else {
        "us".to_string()
    }
}

/// Percent-encoding for a query string, no dependency: everything but the unreserved characters.
fn urlencode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else if c == ' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The first store app a title search returns. Steam's order is relevance, so the first one is the
/// game the user meant far more often than not — and the card names what it found, so a wrong guess
/// is visible instead of silent.
fn steam_hit(json: &Value) -> Option<(u64, String)> {
    let items = json.get("items")?.as_array()?;
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("app") {
            continue;
        }
        let id = item.get("id").and_then(Value::as_u64)?;
        let name = item.get("name").and_then(Value::as_str)?.to_string();
        if name.is_empty() {
            continue;
        }
        return Some((id, name));
    }
    None
}

/// The price block out of the API's answer, keyed by the App ID that was asked for.
fn gg_prices(json: &Value, app_id: u64) -> Option<Offer> {
    let entry = json.get("data")?.get(app_id.to_string())?.clone();
    if entry.is_null() {
        return None;
    }
    let prices = entry.get("prices")?;
    let text = |key: &str| {
        prices
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.is_empty() && s != "0.00")
    };
    let retail = text("currentRetail");
    let keyshops = text("currentKeyshops");
    let best = match (&retail, &keyshops) {
        (Some(r), Some(k)) => {
            let cheaper = match (r.parse::<f64>(), k.parse::<f64>()) {
                (Ok(a), Ok(b)) if b < a => Some((k.clone(), "keyshops")),
                (Ok(_), Ok(_)) => Some((r.clone(), "retail")),
                _ => Some((r.clone(), "retail")),
            };
            cheaper
        }
        (Some(r), None) => Some((r.clone(), "retail")),
        (None, Some(k)) => Some((k.clone(), "keyshops")),
        (None, None) => None,
    };
    Some(Offer {
        title: entry
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        app_id,
        image: Some(format!("{STEAM_IMAGE}/{app_id}/header.jpg")),
        currency: prices
            .get("currency")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        best: best.as_ref().map(|(value, _)| value.clone()),
        best_from: best.map(|(_, from)| from.to_string()),
        retail,
        keyshops,
        historical_retail: text("historicalRetail"),
        historical_keyshops: text("historicalKeyshops"),
        url: entry.get("url").and_then(Value::as_str).map(str::to_string),
    })
}

/// One game's prices by the title the user typed.
pub async fn lookup(title: &str) -> Result<Offer, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("Pusta nazwa gry — podaj tytuł.".to_string());
    }
    let Some(key) = crate::secrets::get(GG_KEY) else {
        return Err(
            "Brak klucza gg.deals. Dodaj go w Ustawieniach → Integracje (Ceny): klucz generujesz na gg.deals w swoim koncie, w sekcji API."
                .to_string(),
        );
    };
    let http = client()?;

    let search_url = format!("{STEAM_SEARCH}?term={}&l=polish&cc=PL", urlencode(title));
    let search = http
        .get(&search_url)
        .send()
        .await
        .map_err(|e| format!("Steam: {e}"))?;
    if !search.status().is_success() {
        return Err(format!("Steam szukanie → HTTP {}", search.status()));
    }
    let search_json: Value = search.json().await.map_err(|e| format!("Steam: {e}"))?;
    let (app_id, name) = steam_hit(&search_json)
        .ok_or_else(|| format!("Steam nie zna gry „{title}”."))?;
    log::line(format!("prices search: {title} → {app_id} ({name})"));

    let price_url = format!(
        "{GG_PRICES}?ids={app_id}&region={}&key={}",
        urlencode(&region()),
        urlencode(&key)
    );
    let prices = http
        .get(&price_url)
        .send()
        .await
        .map_err(|e| format!("gg.deals: {e}"))?;
    let code = prices.status();
    if !code.is_success() {
        return Err(match code.as_u16() {
            401 | 403 => "Klucz gg.deals nie został przyjęty — sprawdź go w Ustawieniach.".to_string(),
            429 => "gg.deals: zbyt wiele zapytań, spróbuj za chwilę.".to_string(),
            _ => format!("gg.deals → HTTP {code}"),
        });
    }
    let json: Value = prices.json().await.map_err(|e| format!("gg.deals: {e}"))?;
    let mut offer = gg_prices(&json, app_id)
        .ok_or_else(|| format!("gg.deals nie ma cen dla „{name}”."))?;
    // The store's own name is the one the user typed against; the API's title can differ in case.
    if offer.title.is_empty() {
        offer.title = name;
    }
    offer.app_id = app_id;
    log::line(format!(
        "prices: {} {} {} (retail {:?}, keyshops {:?})",
        offer.title, offer.best.clone().unwrap_or_default(), offer.currency, offer.retail, offer.keyshops
    ));
    Ok(offer)
}

/// Hands the card to the island.
pub fn show(offer: &Offer) {
    let Some(app) = crate::app() else {
        log::line("prices card dropped: no app handle yet");
        return;
    };
    if let Err(e) = app.emit_to(WINDOW_LABEL, "prices", offer) {
        log::line(format!("prices card emit failed: {e}"));
    }
}

/// The tool result the model answers from, in Polish like every other tool result.
pub fn describe(offer: &Offer) -> String {
    let money = |value: &Option<String>| {
        value
            .as_ref()
            .map(|v| format!("{v} {}", offer.currency))
            .unwrap_or_else(|| "brak".to_string())
    };
    let mut out = format!("Ceny: {}", offer.title);
    match (&offer.best, &offer.best_from) {
        (Some(best), Some(from)) => out.push_str(&format!(
            "\nNajniższa teraz: {best} {} ({})",
            offer.currency,
            if from == "keyshops" { "keyshop" } else { "sklep" }
        )),
        _ => out.push_str("\nNajniższa teraz: brak ofert"),
    }
    out.push_str(&format!("\nSklepy: {}", money(&offer.retail)));
    out.push_str(&format!("\nKeyshopy: {}", money(&offer.keyshops)));
    out.push_str(&format!("\nHistoryczne minimum (sklepy): {}", money(&offer.historical_retail)));
    out.push_str(&format!(
        "\nHistoryczne minimum (keyshopy): {}",
        money(&offer.historical_keyshops)
    ));
    if let Some(url) = &offer.url {
        out.push_str(&format!("\nStrona ze sklepami: {url}"));
    }
    out.push_str(&format!("\nSteam App ID: {}", offer.app_id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real answer for Arma 3, cut to the fields this module reads.
    const ANSWER: &str = r#"{"success":true,"data":{"107410":{"title":"Arma 3","url":"https://gg.deals/game/arma-3/","prices":{"currentRetail":"10.10","currentKeyshops":"10.45","historicalRetail":"2.99","historicalKeyshops":"3.75","currency":"USD"}}}}"#;

    #[test]
    fn reads_the_prices_and_picks_the_cheaper_of_the_two() {
        let json: Value = serde_json::from_str(ANSWER).unwrap();
        let offer = gg_prices(&json, 107410).expect("offer");
        assert_eq!(offer.title, "Arma 3");
        assert_eq!(offer.currency, "USD");
        assert_eq!(offer.best.as_deref(), Some("10.10"));
        assert_eq!(offer.best_from.as_deref(), Some("retail"));
        assert_eq!(offer.historical_retail.as_deref(), Some("2.99"));
        assert_eq!(offer.url.as_deref(), Some("https://gg.deals/game/arma-3/"));
        // The artwork is Steam's own header for that app.
        assert_eq!(
            offer.image.as_deref(),
            Some("https://cdn.cloudflare.steamstatic.com/steam/apps/107410/header.jpg")
        );
    }

    #[test]
    fn a_keyshop_only_game_still_has_a_lowest_price_and_a_missing_one_says_so() {
        let json: Value = serde_json::from_str(
            r#"{"success":true,"data":{"7":{"title":"X","prices":{"currentRetail":null,"currentKeyshops":"24.30","historicalRetail":null,"historicalKeyshops":null,"currency":"PLN"}}}}"#,
        )
        .unwrap();
        let offer = gg_prices(&json, 7).expect("offer");
        assert_eq!(offer.best.as_deref(), Some("24.30"));
        assert_eq!(offer.best_from.as_deref(), Some("keyshops"));
        assert_eq!(offer.currency, "PLN");
        // …and a game the API does not know is not an offer at all.
        let none: Value =
            serde_json::from_str(r#"{"success":true,"data":{"1":null}}"#).unwrap();
        assert!(gg_prices(&none, 1).is_none());
    }

    #[test]
    fn steam_search_takes_the_first_app_and_skips_the_other_kinds() {
        let json: Value = serde_json::from_str(
            r#"{"total":2,"items":[{"type":"sub","name":"Pack","id":9},{"type":"app","name":"Arma 3","id":107410}]}"#,
        )
        .unwrap();
        assert_eq!(steam_hit(&json), Some((107410, "Arma 3".to_string())));
        assert_eq!(steam_hit(&serde_json::json!({"total":0,"items":[]})), None);
    }

    #[test]
    fn a_title_survives_the_query_string() {
        assert_eq!(urlencode("arma 3"), "arma+3");
        assert_eq!(urlencode("Wiedźmin 3: Dzikie Polowanie"), "Wied%C5%BAmin+3%3A+Dzikie+Polowanie");
    }

    /// A live run against both services with whatever key this machine has stored. Ignored by
    /// default: it needs the network and someone's personal key.
    /// Run it by hand: `cargo test --release --lib prices::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_lookup_uses_the_stored_key() {
        let offer = tauri::async_runtime::block_on(lookup("Arma 3")).expect("an offer");
        println!("{offer:#?}");
        assert_eq!(offer.app_id, 107410);
        assert!(offer.best.is_some(), "no price on the card");
    }

    #[test]
    fn the_tool_result_names_money_and_its_currency() {
        let json: Value = serde_json::from_str(ANSWER).unwrap();
        let offer = gg_prices(&json, 107410).unwrap();
        let text = describe(&offer);
        for needle in ["Arma 3", "10.10 USD", "2.99 USD", "gg.deals/game/arma-3"] {
            assert!(text.contains(needle), "{needle} missing from {text}");
        }
    }
}
