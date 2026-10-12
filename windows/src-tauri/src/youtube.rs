// Playing something on YouTube. A search page is not playback: "put on some music" has to
// put music on, not a list of links to choose from — so the app finds the video itself and
// opens it with YouTube's own radio list, and one song does not end in silence.
//
// No API key and no account: the results page carries its own JSON (`ytInitialData`), and
// the video ids are read straight out of it. That is the same keyless fetch the web tools
// already use, so this costs nothing and needs nothing set up.

use std::sync::Mutex;
use std::time::{Duration, Instant};

const RESULTS: &str = "https://www.youtube.com/results?search_query=";

/// How much of the results page to read. The ids sit in the page's own JSON, more than 800 kB
/// into a real search — measured, not guessed — so a window smaller than the page finds none
/// of them and the tool would always answer "nothing found". Bounded by `web::MAX_PAGE_BYTES`,
/// which is what refuses a page that is too large before it is downloaded.
const RESULTS_BYTES: usize = 3 * 1024 * 1024;

/// How far into a renderer's own JSON a title may sit. Below this the title belongs to this
/// result; past it the page has moved on to another shelf, which is not this video's.
const TITLE_WINDOW: usize = 4_000;

/// A video: what to open, and what to call it in the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub title: String,
}

/// What to play, from whatever the user said: a link, a bare video id, or something to
/// search for. The first result of a search is what plays — the model tells the user which
/// one it turned out to be, so a wrong guess is visible instead of silent.
pub async fn resolve(query: &str) -> Result<Hit, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Nie wiem, co puścić — podaj tytuł, wykonawcę albo link.".to_string());
    }
    // A link or an id is not something to search for: it *is* the video.
    if let Some(id) = id_in(query) {
        return Ok(Hit {
            id,
            title: query.to_string(),
        });
    }
    // A link that is not a video — a playlist, a search page, a channel — is refused rather
    // than handled: searching for the URL itself finds nothing, and *opening* it is what this
    // tool exists to stop. An invented playlist id opens an empty page, which is precisely the
    // complaint that produced this tool.
    if looks_like_url(query) {
        return Err(
            "To link, nie tytuł — nie odczytam z niego filmu. Podaj tytuł albo wykonawcę, a poszukam i puszczę; link zadziała tylko wtedy, gdy prowadzi do konkretnego filmu."
                .to_string(),
        );
    }
    // `hl=en` keeps the page in one language, whatever the machine's is: the ids are the
    // same either way, and a stable page is what the parser is tested against.
    let url = format!("{RESULTS}{}&hl=en", urlencode(query));
    crate::log::line(format!("youtube search: {query}"));
    let html = crate::web::fetch_html(&url, RESULTS_BYTES).await?;
    let hits = hits_in(&html);
    match hits.into_iter().next() {
        Some(hit) => {
            crate::log::line(format!("youtube play: {} ({})", hit.title, hit.id));
            Ok(hit)
        }
        None => Err(format!(
            "Nie znalazłem żadnego filmu na „{query}” — może inaczej to nazwać?"
        )),
    }
}

/// Resolves and plays: one call, because the two halves are never useful apart.
pub async fn play(query: &str) -> Result<Started, String> {
    let hit = resolve(query).await?;
    let now = Instant::now();
    let reopen = {
        let last = LAST.lock().unwrap();
        should_open(last.as_ref(), &hit.id, now)
    };
    if !reopen {
        crate::log::line(format!("youtube already playing: {} ({})", hit.title, hit.id));
        return Ok(Started { hit, opened: false });
    }
    *LAST.lock().unwrap() = Some((hit.id.clone(), now));
    crate::shell::open_url(&watch_url(&hit.id))?;
    Ok(Started { hit, opened: true })
}

/// What `play` did: the video, and whether a tab was actually opened for it.
pub struct Started {
    pub hit: Hit,
    /// False when the same video was started a moment ago — the second call is answered
    /// instead of opening a second identical tab.
    pub opened: bool,
}

/// How long the same video counts as already playing. Long enough for a model that calls twice
/// on one request, and for a routine that fires while the chat asks for the same thing, short
/// enough that "play that again" half a minute later opens it again.
const REPEAT_WINDOW: Duration = Duration::from_secs(20);

/// The last video this run started, with when.
static LAST: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Whether this video should be started again. Pure, so the rule is testable: the same id
/// inside the window is not reopened.
fn should_open(previous: Option<&(String, Instant)>, id: &str, now: Instant) -> bool {
    match previous {
        Some((prev, at)) => prev != id || now.duration_since(*at) >= REPEAT_WINDOW,
        None => true,
    }
}

/// A link that is not a video: a playlist, a search page, a channel, or a bare host.
fn looks_like_url(text: &str) -> bool {
    let text = text.trim();
    text.contains("://")
        || text.starts_with("www.")
        || text.starts_with("music.youtube.com")
        || text.starts_with("youtube.com")
        || text.starts_with("youtu.be")
}

/// A video that keeps going. `list=RD<id>` is YouTube's own radio mix for that video and
/// `start_radio=1` starts it as a mix — the difference between one song and an evening.
pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}&list=RD{id}&start_radio=1")
}

// ── Reading the results page ──────────────────────────────────────────────────

/// The results page's own JSON, as (id, title) pairs in YouTube's order. One `videoRenderer`
/// per result; the first one is what plays.
fn hits_in(html: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for chunk in html.split("\"videoRenderer\":{").skip(1) {
        let Some(id) = id_field(chunk) else {
            continue;
        };
        // The title has to be close behind the id: further down the chunk the page has moved
        // on to another shelf, and its title is not this video's.
        let window: String = chunk.chars().take(TITLE_WINDOW).collect();
        let Some(title) = title_in(&window) else {
            continue;
        };
        out.push(Hit { id, title });
    }
    out
}

/// The 11-character id of a video, out of the `"videoId":"…"` every renderer starts with.
fn id_field(chunk: &str) -> Option<String> {
    let rest = chunk.strip_prefix("\"videoId\":\"")?;
    let id: String = rest.chars().take(11).collect();
    is_video_id(&id).then_some(id)
}

fn is_video_id(text: &str) -> bool {
    text.chars().count() == 11
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The first title in the window, as text. YouTube writes it as `"title":{"runs":[{"text":…`
/// for a video and as `"title":{"simpleText":…` for some shelves; the video form is what a
/// result uses.
fn title_in(window: &str) -> Option<String> {
    const RUNS: &str = "\"title\":{\"runs\":[{\"text\":\"";
    let rest = window.split_once(RUNS)?.1;
    let raw = rest.split('"').next()?;
    let title = unescape(raw).trim().to_string();
    (!title.is_empty()).then_some(title)
}

/// The escapes a JSON string carries, decoded. Only the handful YouTube actually uses: a
/// quote, a backslash, the newline, and `\uXXXX` — enough that a title is not shown with
/// its own markup in it.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some('u') => {
                let hex: String = chars.by_ref().take(4).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(decoded) => out.push(decoded),
                    // A lone surrogate cannot be a character on its own; the pair it belongs
                    // to is decoded by the caller's next turn, so dropping it is right.
                    None => {}
                }
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

/// The id out of a link or a bare id: a pasted video is played as it is, never searched for.
fn id_in(text: &str) -> Option<String> {
    if is_video_id(text) {
        return Some(text.to_string());
    }
    for marker in ["watch?v=", "youtu.be/", "/shorts/", "/embed/", "/live/"] {
        if let Some(rest) = text.split_once(marker).map(|(_, r)| r) {
            let id: String = rest.chars().take(11).collect();
            if is_video_id(&id) {
                return Some(id);
            }
        }
    }
    None
}

/// Percent-encoding for a search box, with no dependency: everything but the unreserved
/// characters is escaped, and a space becomes `+` the way a search form does it.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two results, in the shape the real page uses: the id first, the title a field later,
    /// and one shelf further down whose title is not a result's.
    const PAGE: &str = r#"{"videoRenderer":{"videoId":"e84RVBLdGj4","thumbnail":{},"title":{"runs":[{"text":"Kazachska wioska snu"}],"accessibility":{}},"ownerText":{"runs":[{"text":"Kanał"}]}}"videoRenderer":{"videoId":"fq15AEJDFp0","title":{"runs":[{"text":"Abandoned Hangar \u0026 Buran"}],"accessibility":{}}}"shelfRenderer":{"title":{"runs":[{"text":"Ludzie też oglądali"}]}}"#;

    #[test]
    fn reads_the_ids_and_titles_in_the_order_the_page_gives_them() {
        let hits = hits_in(PAGE);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert_eq!(hits[0].id, "e84RVBLdGj4");
        assert_eq!(hits[0].title, "Kazachska wioska snu");
        // The escapes of a JSON string are decoded, so a title is not shown with its own
        // markup in it.
        assert_eq!(hits[1].title, "Abandoned Hangar & Buran");
    }

    #[test]
    fn a_page_without_results_gives_nothing_rather_than_a_guess() {
        assert!(hits_in("<html><body>no videos here</body></html>").is_empty());
        // An id with no title behind it is not a result either.
        assert!(hits_in(r#"{"videoRenderer":{"videoId":"e84RVBLdGj4"}}"#).is_empty());
        // …and neither is a title that belongs to another shelf, far down the chunk.
        let far = format!(
            r#"{{"videoRenderer":{{"videoId":"e84RVBLdGj4","pad":"{}","title":{{"runs":[{{"text":"Shelf"}}]}}}}"#,
            "x".repeat(TITLE_WINDOW + 10)
        );
        assert!(hits_in(&far).is_empty());
    }

    #[test]
    fn a_link_or_an_id_is_the_video_itself() {
        for text in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?t=42",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "dQw4w9WgXcQ",
        ] {
            assert_eq!(id_in(text).as_deref(), Some("dQw4w9WgXcQ"), "{text}");
        }
        // A title is not an id, and neither is a link to something else.
        assert_eq!(id_in("urbex history"), None);
        assert_eq!(id_in("https://example.com/watch?v=nope"), None);
        assert_eq!(id_in("https://www.youtube.com/watch?v=tooshort"), None);
    }

    #[test]
    fn the_url_keeps_going_after_one_video() {
        assert_eq!(
            watch_url("dQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ&start_radio=1"
        );
    }

    #[test]
    fn a_query_survives_the_url() {
        assert_eq!(urlencode("lofi do programowania"), "lofi+do+programowania");
        assert_eq!(urlencode("AC/DC & co"), "AC%2FDC+%26+co");
        // A Polish title is escaped byte by byte, not dropped.
        assert_eq!(urlencode("żółć"), "%C5%BC%C3%B3%C5%82%C4%87");
    }

    #[test]
    fn a_link_that_is_not_a_video_is_refused_not_opened_and_not_searched_for() {
        // The report this rule comes from: an invented YT Music playlist, opened twice.
        for link in [
            "https://music.youtube.com/playlist?list=RDCLAK5uy_lqws4k3lKqQbwGmCwJtGYWj9YN-QFqbfg",
            "https://music.youtube.com/search?q=gaming+music+playlist",
            "https://www.youtube.com/@Kanal",
            "youtube.com",
        ] {
            assert!(looks_like_url(link), "{link} is a link");
            // No video id in it, so nothing to play…
            assert_eq!(id_in(link), None, "{link} has no video in it");
        }
        // …while a title that merely contains a slash is not a link at all.
        for text in ["lofi do programowania", "AC/DC", "urbex history"] {
            assert!(!looks_like_url(text), "{text} is not a link");
        }
    }

    #[test]
    fn the_same_video_is_not_opened_twice_in_a_row() {
        let now = Instant::now();
        let previous = Some(("dQw4w9WgXcQ".to_string(), now));
        // A second call for the same video, straight away: answered, not reopened.
        assert!(!should_open(previous.as_ref(), "dQw4w9WgXcQ", now));
        assert!(!should_open(
            previous.as_ref(),
            "dQw4w9WgXcQ",
            now + Duration::from_secs(5)
        ));
        // Something else, or the same one after the window: open it.
        assert!(should_open(previous.as_ref(), "e84RVBLdGj4", now));
        assert!(should_open(
            previous.as_ref(),
            "dQw4w9WgXcQ",
            now + REPEAT_WINDOW
        ));
        assert!(should_open(None, "dQw4w9WgXcQ", now));
    }
}
