// Live web access for the chat: the app performs the lookups and hands the model
// text back to answer from. Keyless by default, and only on when the user asks.
//
// The keyless path is not one engine: any single free engine starts refusing a
// machine after a couple of lookups, and the chat then answers that it cannot
// reach the web. `free_search` walks three of them and skips whichever one has
// just refused us.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::secrets;

const TIMEOUT: Duration = Duration::from_secs(15);
/// Results handed to the model per search. Enough to cross-check, few enough
/// that the tool result stays a paragraph rather than a page.
const SEARCH_RESULTS: usize = 6;
/// A page is read into at most this much text; the model does not need the
/// footer, and the body has to stay small enough to send back.
const PAGE_CHARS: usize = 12_000;
/// Hard ceiling on the bytes pulled off the wire for one page.
const PAGE_BYTES: usize = 512 * 1024;
/// A page larger than this is refused before it is downloaded.
const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Backends the settings window offers, in display order.
pub const PROVIDERS: [&str; 3] = ["duckduckgo", "brave", "tavily"];

pub fn provider_key(provider: &str) -> Option<&'static str> {
    match provider {
        "brave" => Some("brave-api-key"),
        "tavily" => Some("tavily-api-key"),
        _ => None,
    }
}

pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// One search. `provider` names a backend the settings window can select; an
/// unknown name or a missing key silently falls back to the keyless backend
/// rather than failing the turn.
pub async fn search(provider: &str, query: &str) -> Result<Vec<Hit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Puste zapytanie.".to_string());
    }
    let client = client()?;

    let keyed = match provider_key(provider).and_then(secrets::get) {
        Some(key) => match provider {
            "brave" => Some(brave(&client, &key, query).await),
            "tavily" => Some(tavily(&client, &key, query).await),
            _ => None,
        },
        None => None,
    };

    let hits = match keyed {
        Some(Ok(hits)) if !hits.is_empty() => hits,
        Some(Ok(_)) => free_search(&client, query).await?,
        Some(Err(err)) => {
            // A stale key should not cost the user their answer: log it and use
            // the free engines instead.
            crate::log::line(format!("web  {provider} failed ({err}) — falling back to the free engines"));
            free_search(&client, query).await?
        }
        None => free_search(&client, query).await?,
    };

    if hits.is_empty() {
        return Err(format!("No results for “{query}”."));
    }
    crate::log::line(format!("web  search {provider} „{query}” → {} hits", hits.len()));
    Ok(hits)
}

/// One page, as plain text. Only http(s) is accepted, and only on the public
/// internet, a fetched page is untrusted input, and it must not be able to
/// point the app back at the machine it runs on.
pub async fn fetch(url: &str) -> Result<String, String> {
    let body = fetch_html(url, PAGE_BYTES).await?;
    let text = page_text(&body);
    if text.trim().is_empty() {
        return Err("The page has no readable text (it may need JavaScript).".to_string());
    }
    crate::log::line(format!("web  fetch {url} → {} chars", text.len()));
    Ok(text)
}

/// A page's own markup, byte for byte, for the callers that have to read the page's own JSON
/// rather than its text — YouTube's results carry the video ids there and nowhere else, and
/// they sit far enough into a 1.7 MB page that the text tools' own cap would miss them, so
/// the cap is the caller's. The guards are the same: http(s) only, public hosts only, and a
/// page that announces itself as larger than `MAX_PAGE_BYTES` is refused before download.
pub(crate) async fn fetch_html(url: &str, cap: usize) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("The address must start with http:// or https://".to_string());
    }
    let host = host_of(url).ok_or_else(|| "Cannot read the host from that address.".to_string())?;
    if is_private_host(&host) {
        return Err(format!("{host} to adres lokalny — nie czytam go."));
    }

    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Could not open the page: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("The page answered {status}."));
    }
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !kind.is_empty() && !kind.contains("text/html") && !kind.contains("text/plain") {
        return Err(format!("That is not a text page ({kind})."));
    }
    // A page that announces itself as huge is refused before it is downloaded:
    // the cap below would otherwise only trim what was already in memory.
    if let Some(size) = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        if size > MAX_PAGE_BYTES {
            return Err(format!("The page is too large ({size} B)."));
        }
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Could not read the page: {e}"))?;
    Ok(String::from_utf8_lossy(&bytes[..bytes.len().min(cap)]).into_owned())
}

/// The tool result the model reads: numbered, plain, with the URL spelled out
/// so it can cite it without inventing one.
pub fn format_hits(hits: &[Hit]) -> String {
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, hit.title, hit.url));
        if !hit.snippet.is_empty() {
            out.push_str(&format!("   {}\n", hit.snippet));
        }
    }
    // `String::truncate` panics off a char boundary, and these snippets are
    // frequently Polish.
    truncate_chars(&mut out, 6_000);
    out
}

fn truncate_chars(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

// ── Backends ──────────────────────────────────────────────────────────────────

/// What one keyless engine had to say. The distinction is what the chain acts
/// on: a block lasts minutes and the next engine should be tried at once, while
/// an ordinary failure is worth one more attempt later.
enum Attempt {
    Hits(Vec<Hit>),
    /// Answered and parsed, and there was nothing — not worth a cooldown.
    Empty,
    /// The engine refused this address: rate limit, captcha, an empty 202.
    Blocked(String),
    /// Anything else: a timeout, a 5xx, a body that would not parse.
    Failed(String),
}

/// The keyless engines, in the order they are tried. No single one survives an
/// ordinary session — DuckDuckGo starts answering 202 with an empty body after a
/// couple of lookups from one address, which is how a chat comes to say it can
/// no longer reach the web. Two more agreeing on the same query is enough for a
/// lookup to come back.
const FREE_BACKENDS: [&str; 3] = ["duckduckgo", "bing", "brave"];

/// How long an engine that refused this address is skipped. Long enough that
/// asking again does not deepen the block, short enough that one block does not
/// cost the session.
const BLOCK_COOLDOWN: Duration = Duration::from_secs(300);
/// One failed request is not a verdict about the engine.
const FAIL_COOLDOWN: Duration = Duration::from_secs(60);

/// `engine → the moment it may be used again`. A block belongs to the machine
/// and the engine together, so one cell per process is the right scope: the
/// second search in the same minute is exactly the one that used to fail.
fn cool_downs() -> &'static Mutex<HashMap<&'static str, Instant>> {
    static CELL: OnceLock<Mutex<HashMap<&'static str, Instant>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cooling(engine: &str) -> bool {
    let mut cell = cool_downs().lock().unwrap_or_else(|e| e.into_inner());
    match cell.get(engine).copied() {
        Some(until) if until > Instant::now() => true,
        Some(_) => {
            // Expired: forget it, so the map does not grow across a long run.
            cell.remove(engine);
            false
        }
        None => false,
    }
}

fn cool_down(engine: &'static str, for_: Duration) {
    cool_downs()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(engine, Instant::now() + for_);
}

/// The free path: every keyless engine in turn, skipping whichever ones refused
/// us a moment ago. Returns the first non-empty set of hits.
async fn free_search(client: &reqwest::Client, query: &str) -> Result<Vec<Hit>, String> {
    let mut notes: Vec<String> = Vec::new();
    for engine in FREE_BACKENDS {
        if cooling(engine) {
            notes.push(format!("{engine}: cooldown"));
            continue;
        }
        let attempt = match engine {
            "bing" => bing(client, query).await,
            "brave" => brave_page(client, query).await,
            _ => duckduckgo(client, query).await,
        };
        match attempt {
            Attempt::Hits(hits) if !hits.is_empty() => {
                crate::log::line(format!("web  {engine} „{query}” → {} hits", hits.len()));
                return Ok(hits);
            }
            Attempt::Hits(_) | Attempt::Empty => notes.push(format!("{engine}: no results")),
            Attempt::Blocked(why) => {
                crate::log::line(format!(
                    "web  {engine} blocked ({why}) — skipped for {} s",
                    BLOCK_COOLDOWN.as_secs()
                ));
                cool_down(engine, BLOCK_COOLDOWN);
                notes.push(format!("{engine}: blocked"));
            }
            Attempt::Failed(why) => {
                crate::log::line(format!("web  {engine} failed ({why})"));
                cool_down(engine, FAIL_COOLDOWN);
                notes.push(format!("{engine}: failed"));
            }
        }
    }
    crate::log::line(format!("web  no engine answered „{query}”: {}", notes.join("; ")));
    // i18n-ok: model-facing tool error; the model says it in the user's own language.
    Err("Żadna wyszukiwarka teraz nie odpowiada — powiedz, że spróbujesz za chwilę, albo przeczytaj konkretną stronę przez fetch_url.".to_string())
}

/// DuckDuckGo's no-JavaScript endpoint. No key, no account, the reason web
/// access needs no setup at all — and the one that throttles soonest: after a
/// couple of lookups the same address gets 202 with an empty body.
async fn duckduckgo(client: &reqwest::Client, query: &str) -> Attempt {
    let response = match client
        .post("https://html.duckduckgo.com/html/")
        .header("content-type", "application/x-www-form-urlencoded")
        .form(&[("q", query), ("kl", "wt-wt")])
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return Attempt::Failed(format!("did not answer: {e}")),
    };
    let status = response.status();
    if status == reqwest::StatusCode::ACCEPTED {
        return Attempt::Blocked("rate limit".to_string());
    }
    let html = match response.text().await {
        Ok(html) => html,
        Err(e) => return Attempt::Failed(format!("read error: {e}")),
    };
    if !status.is_success() {
        return Attempt::Failed(format!("answered {status}"));
    }
    settled(parse_ddg(&html), &html)
}

/// Bing's result page. The markup has been the same for a decade and an app
/// reading it is not what it fights, which makes it the dependable second
/// engine.
async fn bing(client: &reqwest::Client, query: &str) -> Attempt {
    match search_page(client, "https://www.bing.com/search", query).await {
        Ok(html) => settled(parse_bing(&html), &html),
        Err(attempt) => attempt,
    }
}

/// Brave's own result page, read for the markup it renders on the server. The
/// API wants a key; this page does not.
async fn brave_page(client: &reqwest::Client, query: &str) -> Attempt {
    match search_page(client, "https://search.brave.com/search", query).await {
        Ok(html) => settled(parse_brave(&html), &html),
        Err(attempt) => attempt,
    }
}

/// One GET against a result page, reduced to what every engine here shares: the
/// status tells a block from an outage, and the body goes to the parser that
/// knows that engine's markup.
async fn search_page(client: &reqwest::Client, url: &str, query: &str) -> Result<String, Attempt> {
    let response = match client
        .get(url)
        .query(&[("q", query)])
        .send()
        .await
    {
        Ok(response) => response,
        Err(e) => return Err(Attempt::Failed(format!("did not answer: {e}"))),
    };
    let status = response.status();
    // 403 and 429 are the engines' way of saying "this address, later"; 202 with
    // an empty body is DuckDuckGo's version of the same thing.
    if status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status == reqwest::StatusCode::ACCEPTED
    {
        return Err(Attempt::Blocked(format!("answered {status}")));
    }
    let html = match response.text().await {
        Ok(html) => html,
        Err(e) => return Err(Attempt::Failed(format!("read error: {e}"))),
    };
    if !status.is_success() {
        return Err(Attempt::Failed(format!("answered {status}")));
    }
    Ok(html)
}

/// A parse that found nothing means either "no such page on the web" or "this is
/// the engine's block page". Only the second is worth a cooldown, so the body is
/// read for the words those pages carry.
fn settled(hits: Vec<Hit>, html: &str) -> Attempt {
    if !hits.is_empty() {
        return Attempt::Hits(hits);
    }
    match blocked_marker(html) {
        Some(why) => Attempt::Blocked(why.to_string()),
        None => Attempt::Empty,
    }
}

/// Text that only a search engine's own "you are a robot" page carries.
const BLOCK_MARKERS: [&str; 7] = [
    "captcha",
    "unusual traffic",
    "anomaly",
    "challenge-platform",
    "verify you are human",
    "are you a robot",
    "enable javascript and reload",
];

fn blocked_marker(html: &str) -> Option<&'static str> {
    BLOCK_MARKERS
        .iter()
        .copied()
        .find(|needle| find_ci(html, needle).is_some())
}

async fn brave(client: &reqwest::Client, key: &str, query: &str) -> Result<Vec<Hit>, String> {
    let count = SEARCH_RESULTS.to_string();
    let response = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .query(&[("q", query), ("count", count.as_str())])
        .header("accept", "application/json")
        .header("x-subscription-token", key)
        .send()
        .await
        .map_err(|e| format!("Brave did not answer: {e}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Brave: bad response: {e}"))?;
    if !status.is_success() {
        return Err(format!("Brave API {status}: {}", api_error(&body)));
    }
    let hits = body
        .pointer("/web/results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Hit {
                        title: item.get("title")?.as_str()?.to_string(),
                        url: item.get("url")?.as_str()?.to_string(),
                        snippet: item
                            .get("description")
                            .and_then(Value::as_str)
                            .map(strip_tags)
                            .unwrap_or_default(),
                    })
                })
                .take(SEARCH_RESULTS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(hits)
}

async fn tavily(client: &reqwest::Client, key: &str, query: &str) -> Result<Vec<Hit>, String> {
    let response = client
        .post("https://api.tavily.com/search")
        .json(&serde_json::json!({
            "api_key": key,
            "query": query,
            "search_depth": "basic",
            "max_results": SEARCH_RESULTS,
        }))
        .send()
        .await
        .map_err(|e| format!("Tavily did not answer: {e}"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Tavily: bad response: {e}"))?;
    if !status.is_success() {
        return Err(format!("Tavily API {status}: {}", api_error(&body)));
    }
    let hits = body
        .get("results")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(Hit {
                        title: item.get("title")?.as_str()?.to_string(),
                        url: item.get("url")?.as_str()?.to_string(),
                        snippet: item
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .chars()
                            .take(400)
                            .collect(),
                    })
                })
                .take(SEARCH_RESULTS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(hits)
}

fn api_error(body: &Value) -> String {
    body.get("error")
        .and_then(|e| e.get("message").or(Some(e)))
        .and_then(Value::as_str)
        .unwrap_or("unknown error")
        .chars()
        .take(200)
        .collect()
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| format!("Could not build the HTTP client: {e}"))
}

// ── Parsing ───────────────────────────────────────────────────────────────────

/// Pulls the numbered results out of DuckDuckGo's HTML. Written by hand rather
/// than with a parser crate: the markup is a flat list of three anchors, and a
/// regex engine is not worth a dependency here.
fn parse_ddg(html: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    let mut rest = html;
    // Walk the anchors and keep the ones DuckDuckGo marks as results. Slicing
    // to the `class=` attribute instead would cut off the opening `<a`, and the
    // attribute's position is not the tag's.
    while let Some(pos) = rest.find("<a") {
        let anchor = &rest[pos..];
        let Some(tag_end) = anchor.find('>') else { break };
        let tag = &anchor[..tag_end];
        if !tag.contains("class=\"result__a\"") {
            rest = &rest[pos + 2..];
            continue;
        }
        let url = attr(tag, "href").map(|href| clean_ddg_url(&href)).unwrap_or_default();
        let after = &anchor[tag_end + 1..];
        let Some(title_end) = after.find("</a>") else { break };
        let title = one_line(&html_unescape(&strip_tags(&after[..title_end])));

        // The snippet follows the title in the same block; when a result has
        // none, the next result's title ends the search for it.
        let tail = &after[title_end..];
        let stop = tail.find("class=\"result__a\"").unwrap_or(tail.len());
        let snippet = tail[..stop]
            .find("class=\"result__snippet\"")
            .and_then(|s| {
                let block = &tail[s..];
                let open = block.find('>')? + 1;
                let close = block[open..].find("</a>")? + open;
                Some(one_line(&html_unescape(&strip_tags(&block[open..close]))))
            })
            .unwrap_or_default();

        if !url.is_empty() && !title.is_empty() {
            hits.push(Hit { title, url, snippet });
        }
        if hits.len() >= SEARCH_RESULTS {
            break;
        }
        rest = &after[title_end..];
    }
    hits
}

/// `//duckduckgo.com/l/?uddg=https%3A%2F%2F…` is what DDG hands back when it
/// wraps a link; the real target is the `uddg` parameter.
fn clean_ddg_url(href: &str) -> String {
    let href = html_unescape(href);
    let encoded = href
        .split_once("uddg=")
        .map(|(_, rest)| rest.split('&').next().unwrap_or(rest));
    match encoded {
        Some(value) => percent_decode(value),
        None => href,
    }
}

/// Pulls the results out of Bing's page: one `<li class="b_algo">` per result,
/// the title in the `<h2>` anchor and the snippet in the first paragraph under
/// it. The address arrives wrapped in Bing's own redirector.
fn parse_bing(html: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for block in blocks(html, "class=\"b_algo") {
        let Some(heading) = block.find("<h2") else { continue };
        let rest = &block[heading..];
        let Some(anchor_at) = rest.find("<a") else { continue };
        let anchor = &rest[anchor_at..];
        let Some(tag_end) = anchor.find('>') else { continue };
        let Some(url) = attr(&anchor[..tag_end], "href").map(|href| clean_bing_url(&href)) else {
            continue;
        };
        let Some(title_end) = anchor.find("</a>") else { continue };
        let title = one_line(&html_unescape(&strip_tags(&anchor[tag_end + 1..title_end])));
        // Bing renders the snippet as the first paragraph after the heading; a
        // result with none ends before the next one starts, which `blocks` cut.
        let tail = &anchor[title_end..];
        let snippet = tail
            .find("<p")
            .and_then(|p| {
                let para = &tail[p..];
                let open = para.find('>')? + 1;
                let close = para.find("</p>")?;
                (close > open).then(|| one_line(&html_unescape(&strip_tags(&para[open..close]))))
            })
            .unwrap_or_default();
        if !url.is_empty() && !title.is_empty() {
            hits.push(Hit { title, url, snippet });
        }
        if hits.len() >= SEARCH_RESULTS {
            break;
        }
    }
    hits
}

/// Bing wraps a result in `https://www.bing.com/ck/a?…&u=a1<base64url>`; the real
/// address is that parameter, base64url-encoded behind a two-character marker.
fn clean_bing_url(href: &str) -> String {
    let href = html_unescape(href);
    let Some((_, query)) = href.split_once("bing.com/ck/a?") else { return href };
    let Some(value) = query.split('&').find_map(|pair| pair.strip_prefix("u=")) else { return href };
    let encoded = value.strip_prefix("a1").unwrap_or(value);
    base64url(encoded).unwrap_or(href)
}

/// Base64url without padding, decoded by hand: the only thing this module needs
/// it for is Bing's redirector, and it is not worth a dependency.
fn base64url(text: &str) -> Option<String> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Brave renders its results on the server, with the address on the anchor, the
/// title in the div beside it and the snippet in a `generic-snippet` block.
fn parse_brave(html: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for block in blocks(html, "data-type=\"web\"") {
        let Some(anchor_at) = block.find("<a") else { continue };
        let anchor = &block[anchor_at..];
        let Some(tag_end) = anchor.find('>') else { continue };
        let Some(url) = attr(&anchor[..tag_end], "href") else { continue };
        let Some(title_at) = anchor.find("class=\"title ") else { continue };
        let title_block = &anchor[title_at..];
        let Some(open) = title_block.find('>') else { continue };
        let Some(close) = title_block.find("</div>") else { continue };
        if close <= open {
            continue;
        }
        let title = one_line(&html_unescape(&strip_tags(&title_block[open + 1..close])));
        let snippet = anchor
            .find("class=\"generic-snippet")
            .and_then(|s| {
                let tail = &anchor[s..];
                let open = tail.find('>')? + 1;
                let close = tail.find("</div>")?;
                (close > open).then(|| one_line(&html_unescape(&strip_tags(&tail[open..close]))))
            })
            .unwrap_or_default();
        if !url.is_empty() && !title.is_empty() {
            hits.push(Hit { title, url, snippet });
        }
        if hits.len() >= SEARCH_RESULTS {
            break;
        }
    }
    hits
}

/// The chunks of `html` that start at `marker`, each running to the next marker.
/// A result list is a flat run of identical blocks, so this is all the structure
/// a parser needs — and it never slices inside a character.
fn blocks<'a>(html: &'a str, marker: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find(marker) {
        let chunk = &rest[pos..];
        let end = chunk[marker.len()..]
            .find(marker)
            .map(|next| next + marker.len())
            .unwrap_or(chunk.len());
        out.push(&chunk[..end]);
        rest = &chunk[end..];
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

/// Everything outside `<script>`, `<style>`, `<noscript>` and comments, the
/// parts of a page that are never prose.
fn page_text(html: &str) -> String {
    let mut html = html.to_string();
    for (open, close) in [
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
        ("<!--", "-->"),
    ] {
        html = drop_between(&html, open, close);
    }
    // Block boundaries become line breaks, so the text keeps a shape instead of
    // collapsing into one wall of words.
    let mut spaced = html;
    for tag in [
        "</p>", "</div>", "</li>", "</tr>", "</h1>", "</h2>", "</h3>", "</h4>", "</h5>", "</h6>",
        "</section>", "</article>", "<br", "<br/>", "<br />",
    ] {
        spaced = spaced.replace_case_insensitive(tag, "\n");
    }
    let lines = html_unescape(&strip_tags(&spaced))
        .lines()
        .map(|line| one_line(line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    lines.chars().take(PAGE_CHARS).collect()
}

fn drop_between(html: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = find_ci(rest, open) {
        out.push_str(&rest[..start]);
        match find_ci(&rest[start..], close) {
            Some(end) => rest = &rest[start + end + close.len()..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Whitespace runs, newlines included, become single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Byte search that ignores ASCII case. Deliberately not `to_lowercase().find()`:
/// lowercasing can change a string's byte length, which would hand back an index
/// that no longer lines up with the original text.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

trait ReplaceCaseInsensitive {
    fn replace_case_insensitive(&self, needle: &str, with: &str) -> String;
}

impl ReplaceCaseInsensitive for String {
    fn replace_case_insensitive(&self, needle: &str, with: &str) -> String {
        let mut out = String::with_capacity(self.len());
        let mut rest = self.as_str();
        while let Some(pos) = find_ci(rest, needle) {
            out.push_str(&rest[..pos]);
            out.push_str(with);
            rest = &rest[pos + needle.len()..];
        }
        out.push_str(rest);
        out
    }
}

/// The handful of entities a search result or a page body actually contains.
fn html_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        // The window is counted in bytes, so it has to be walked back to a character
        // boundary: what follows a `&` is not always ASCII, and a slice that splits a
        // character panics, which in a release build takes the whole app with it.
        let mut window = tail.len().min(12);
        while window > 0 && !tail.is_char_boundary(window) {
            window -= 1;
        }
        let end = tail[..window].find(';');
        let entity = end.map(|e| &tail[1..e]);
        let decoded = entity.and_then(|name| match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            "hellip" => Some('…'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "oacute" => Some('ó'),
            "eacute" => Some('é'),
            "aacute" => Some('á'),
            "rsquo" => Some('’'),
            "lsquo" => Some('‘'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ => name.strip_prefix('#').and_then(|digits| {
                match digits.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => digits.parse::<u32>().ok(),
                }
                .and_then(char::from_u32)
            }),
        });
        match (decoded, end) {
            (Some(ch), Some(e)) => {
                out.push(ch);
                rest = &tail[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One hex digit, straight off a byte, no string slicing involved.
fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                // Read the two digits as bytes. Slicing the string here panicked the
                // moment an escape sat next to a multi-byte character.
                match (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi * 16 + lo);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── URL safety ────────────────────────────────────────────────────────────────

fn host_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?; // strip any userinfo
    let host = match authority.strip_prefix('[') {
        // An IPv6 literal keeps its colons, so the port split does not apply.
        Some(bracketed) => bracketed.split(']').next()?.to_string(),
        None => authority.split(':').next()?.to_string(),
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// A page the model chose to read must not be able to reach the machine it
/// runs on (or the LAN it sits in).
fn is_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    if host.contains(':') {
        // Literal IPv6: loopback and the unique-local / link-local ranges.
        return host == "::1"
            || host.starts_with("fe80:")
            || host.starts_with("fc")
            || host.starts_with("fd");
    }
    let octets: Vec<u8> = host.split('.').filter_map(|part| part.parse().ok()).collect();
    match octets.as_slice() {
        [127, ..] | [10, ..] | [0, ..] | [169, 254, ..] | [192, 168, ..] => true,
        [172, second, ..] => (16..=31).contains(second),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG: &str = r#"
    <div class="result">
      <a rel="nofollow" class="result__a" href="https://api-docs.deepseek.com/api/list-models/">Lists &amp; Models | <b>DeepSeek</b></a>
      <a class="result__url" href="https://api-docs.deepseek.com/api/list-models/">api-docs.deepseek.com</a>
      <a class="result__snippet" href="https://api-docs.deepseek.com/">Lists the currently available <b>models</b>.</a>
    </div>
    <div class="result">
      <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1&amp;rut=xyz">Example</a>
    </div>
    "#;

    #[test]
    fn ddg_results_and_wrapped_urls() {
        let hits = parse_ddg(DDG);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Lists & Models | DeepSeek");
        assert_eq!(hits[0].url, "https://api-docs.deepseek.com/api/list-models/");
        assert_eq!(hits[0].snippet, "Lists the currently available models.");
        assert_eq!(hits[1].url, "https://example.com/a?b=1");
        assert!(hits[1].snippet.is_empty());
    }

    /// Trimmed from a real Bing answer: the address is the redirector, the title
    /// sits in the `<h2>` and the snippet in the paragraph under it.
    const BING: &str = r#"
    <ol id="b_results">
      <li class="b_algo" data-id iid=SERP.1><h2><a href="https://www.bing.com/ck/a?!&amp;&amp;p=52a1&amp;u=a1aHR0cHM6Ly9leGFtcGxlLmNvbS9hP2I9MQ&amp;ntb=1">Kursy &amp; waluty</a></h2><div class="b_caption"><p class="b_lineclamp2">Aktualne kursy&nbsp;walut &#0183; NBP</p></div></li>
      <li class="b_algo b_algoBorder"><h2><a href="https://plain.example.org/x">Plain</a></h2></li>
    </ol>"#;

    #[test]
    fn bing_results_and_redirector() {
        let hits = parse_bing(BING);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Kursy & waluty");
        assert_eq!(hits[0].url, "https://example.com/a?b=1");
        assert_eq!(hits[0].snippet, "Aktualne kursy walut · NBP");
        assert_eq!(hits[1].url, "https://plain.example.org/x");
        assert!(hits[1].snippet.is_empty());
    }

    #[test]
    fn a_bing_redirector_that_will_not_decode_keeps_its_address() {
        // Better a link that still opens than a result with no address at all.
        let href = "https://www.bing.com/ck/a?!&&p=1&u=a1!!!!&ntb=1";
        assert_eq!(clean_bing_url(href), href);
        assert_eq!(clean_bing_url("https://example.com/plain"), "https://example.com/plain");
    }

    /// Trimmed from Brave's server-rendered page: the anchor carries the address,
    /// the div beside it the title, the `generic-snippet` the text.
    const BRAVE: &str = r#"
    <div class="snippet svelte-jmfu5f" data-pos="0" data-type="web"><div class="result-body"><a href="https://apnews.com/hub/poland" target="_self" class="svelte-14r20fy l1"><div class="site-name-wrapper"></div> <div class="title search-snippet-title line-clamp-1" title="Poland | AP News">Poland | AP News</div></a><div class="generic-snippet svelte-1cwdgg3"><div class="content">2 days ago - Six people <strong>died</strong>.</div></div></div></div>
    <div class="snippet svelte-jmfu5f" data-pos="1" data-type="web"><div class="result-body"><a href="https://example.com/b" class="l1"><div class="title search-snippet-title">Second &amp; last</div></a></div></div>"#;

    #[test]
    fn brave_results_parse() {
        let hits = parse_brave(BRAVE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Poland | AP News");
        assert_eq!(hits[0].url, "https://apnews.com/hub/poland");
        assert_eq!(hits[0].snippet, "2 days ago - Six people died.");
        assert_eq!(hits[1].title, "Second & last");
        assert!(hits[1].snippet.is_empty());
    }

    /// The page DuckDuckGo serves once it has had enough: no results, and the
    /// word that says so. This is the case that has to cool the engine down — an
    /// empty page without it is a query that simply found nothing.
    #[test]
    fn a_block_page_cools_the_engine_down_and_an_empty_one_does_not() {
        match settled(Vec::new(), "<html><body>No results found for that query.</body></html>") {
            Attempt::Empty => {}
            _ => panic!("an empty answer with no block word must not cool anything down"),
        }
        match settled(Vec::new(), "<html><h1>Please solve this CAPTCHA</h1></html>") {
            Attempt::Blocked(_) => {}
            _ => panic!("a captcha page must be reported as a block"),
        }
    }

    #[test]
    fn a_blocked_engine_is_skipped_until_its_cooldown_runs_out() {
        cool_down("test-only", Duration::from_secs(60));
        assert!(cooling("test-only"));
        cool_down("test-only", Duration::ZERO);
        assert!(!cooling("test-only"));
    }

    #[test]
    fn page_text_drops_scripts_and_keeps_shape() {
        let text = page_text("<html><head><style>a{}</style><script>var x=1</script></head><body><h1>Title</h1><p>One &amp; two</p><div>Three</div></body></html>");
        assert_eq!(text, "Title\nOne & two\nThree");
    }

    #[test]
    fn numeric_entities_decode() {
        assert_eq!(html_unescape("caf&#233; &#x27;x&#x27;"), "café 'x'");
    }

    /// Both of these panicked before: a byte-counted slice landing inside a
    /// multi-byte character. In release that is `panic = abort`, so a search result
    /// with the wrong accent in the wrong place closed the app.
    #[test]
    fn entities_and_escapes_survive_multibyte_neighbours() {
        let text = format!("&{}é", "a".repeat(10)); // byte 12 is inside the é
        assert_eq!(html_unescape(&text), text);
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("caf%C3%A9%21"), "café!");
    }

    #[test]
    fn private_hosts_are_refused() {
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("192.168.0.10"));
        assert!(is_private_host("172.20.1.4"));
        assert!(is_private_host("localhost"));
        assert!(!is_private_host("example.com"));
        assert!(!is_private_host("172.32.0.1"));
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://user:pw@Example.COM:8443/x?y").as_deref(), Some("example.com"));
        assert_eq!(host_of("ftp://example.com"), None);
    }

    /// Opt-in check against the real endpoints, since a fixture cannot catch a
    /// DDG markup change: `OCZI_LIVE_WEB=1 cargo test --lib live_web -- --nocapture`
    #[test]
    fn live_web() {
        if std::env::var("OCZI_LIVE_WEB").is_err() {
            return;
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let hits = search("duckduckgo", "euro exchange rate today")
                .await
                .expect("duckduckgo search");
            println!("--- search ---\n{}", format_hits(&hits));
            assert!(!hits.is_empty());
            assert!(hits.iter().any(|hit| !hit.snippet.is_empty()), "no snippets parsed");

            let page = fetch(&hits[0].url).await.expect("fetch first hit");
            println!("--- page: {} ---\n{}", hits[0].url, &page[..page.len().min(600)]);
            assert!(!page.trim().is_empty());
        });
    }
}
