// Tool calls that arrive as text instead of the `tool_calls` field.

use serde_json::{json, Map, Value};

pub struct TextCall {
    pub name: String,
    pub args: Value,
}

/// Pipes and angle brackets sometimes come back as their full-width forms.
fn normalize(text: &str) -> String {
    text.replace('\u{ff5c}', "|")
        .replace('\u{ff1c}', "<")
        .replace('\u{ff1e}', ">")
}

/// The calls found in the text, ignoring anything that is not one. `known` is
/// the list of tools actually offered this turn: a JSON object that merely has a
/// "name" key is prose, not a call, so the name has to be a real tool.
pub fn parse(text: &str, known: &[&str]) -> Vec<TextCall> {
    spans(text)
        .into_iter()
        .filter(|(_, _, call)| known.contains(&call.name.as_str()))
        .map(|(_, _, call)| call)
        .collect()
}

/// The text with the calls taken out, so the user never sees the markup.
pub fn strip(text: &str, known: &[&str]) -> String {
    let norm = normalize(text);
    let found: Vec<(usize, usize, TextCall)> = spans(&norm)
        .into_iter()
        .filter(|(_, _, call)| known.contains(&call.name.as_str()))
        .collect();
    if found.is_empty() {
        return tidy(&norm);
    }
    let mut out = String::new();
    let mut cursor = 0;
    for (start, end, _) in found {
        out.push_str(&norm[cursor..start]);
        cursor = end;
    }
    out.push_str(&norm[cursor..]);
    tidy(&out)
}

/// Fences and blank lines left behind by a removed block.
fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "```" || trimmed == "```json" {
            continue;
        }
        if trimmed.is_empty() && lines.last().map(|l| l.trim().is_empty()).unwrap_or(true) {
            continue;
        }
        lines.push(line);
    }
    while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

/// Every call in the text with the span it occupies, in normalised coordinates.
fn spans(text: &str) -> Vec<(usize, usize, TextCall)> {
    let found = invoke_spans(text);
    if found.is_empty() {
        json_spans(text)
    } else {
        found
    }
}

/// `<|DSML|invoke name="run_terminal"> … <|DSML|parameter name="command">dir<…>`
fn invoke_spans(text: &str) -> Vec<(usize, usize, TextCall)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find("invoke name=") {
        let at = cursor + offset;
        let after = &text[at + "invoke name=".len()..];
        let Some((name, used)) = read_quoted(after) else {
            cursor = at + 1;
            continue;
        };
        // The block ends at its closing tag when there is one, otherwise at the
        // next invoke or the end of the text: a model that forgets the closing
        // tag still gets its call read, without swallowing the prose after it.
        let body_start = at + "invoke name=".len() + used;
        let rest = &text[body_start..];
        let next_invoke = rest
            .find("invoke name=")
            .map(|i| body_start + i)
            .unwrap_or(text.len());
        let closing = rest
            .find("invoke>")
            .map(|i| body_start + i + "invoke>".len())
            .unwrap_or(text.len());
        let body_end = next_invoke.min(closing);
        // Start the span at the opening tag so stripping removes it too: after
        // the previous tag when there is one, otherwise at this tag's angle
        // bracket, so prose on the line above is not swallowed.
        let after_previous = text[..at].rfind('>').map(|i| i + 1).unwrap_or(0);
        let at_bracket = text[..at].rfind('<').unwrap_or(0);
        let start = after_previous.max(at_bracket);
        found.push((
            start,
            body_end,
            TextCall {
                name: name.to_string(),
                args: read_parameters(&text[body_start..body_end]),
            },
        ));
        cursor = body_end;
    }
    found
}

/// `…<parameter name="command">dir<…>`
fn read_parameters(body: &str) -> Value {
    let mut map = Map::new();
    let mut cursor = 0;
    while let Some(offset) = body[cursor..].find("parameter name=") {
        let at = cursor + offset;
        let after = &body[at + "parameter name=".len()..];
        let Some((key, used)) = read_quoted(after) else {
            cursor = at + 1;
            continue;
        };
        let rest = &after[used..];
        let Some(gt) = rest.find('>') else {
            cursor = at + 1;
            continue;
        };
        let value = &rest[gt + 1..];
        let end = value.find('<').unwrap_or(value.len());
        map.insert(key.to_string(), as_json(value[..end].trim()));
        cursor = at + "parameter name=".len() + used + gt + 1 + end;
    }
    // Some models put the whole call into JSON inside a single parameter.
    if map.len() == 1 {
        if let Some(Value::String(inner)) = map.values().next() {
            if let Ok(Value::Object(obj)) = serde_json::from_str(inner) {
                return Value::Object(obj);
            }
        }
    }
    Value::Object(map)
}

/// A quoted string starting at `text`, plus how many bytes it took.
fn read_quoted(text: &str) -> Option<(&str, usize)> {
    let text = text.trim_start();
    let skipped = text.len();
    let quote = text.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &text[quote.len_utf8()..];
    let end = rest.find(quote)?;
    Some((&rest[..end], skipped - rest.len() + end + quote.len_utf8()))
}

/// Numbers, booleans and nulls become real JSON; everything else stays a string.
fn as_json(value: &str) -> Value {
    if value.is_empty() {
        return Value::String(String::new());
    }
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
}

/// `{"name": "run_terminal", "arguments": {"command": "…"}}`, bare or fenced.
fn json_spans(text: &str) -> Vec<(usize, usize, TextCall)> {
    let mut found = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(open) = text[cursor..].find('{').map(|i| cursor + i) else {
            break;
        };
        let Some(end) = balanced_end(text, open) else {
            break;
        };
        if let Some(call) = as_call(&text[open..end]) {
            found.push((open, end, call));
        }
        cursor = end;
    }
    found
}

/// The end of the object starting at `open`, ignoring braces inside strings.
fn balanced_end(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, byte) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn as_call(block: &str) -> Option<TextCall> {
    let value: Value = serde_json::from_str(block).ok()?;
    // Unwrap the wrappers models like to add around a call.
    let inner = ["tool_call", "tool", "function", "call"]
        .iter()
        .find_map(|key| value.get(*key))
        .unwrap_or(&value);
    let name = inner.get("name").and_then(Value::as_str)?;
    let args = inner
        .get("arguments")
        .or_else(|| inner.get("parameters"))
        .or_else(|| inner.get("args"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let args = match args {
        Value::String(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| json!({})),
        other => other,
    };
    if !args.is_object() {
        return None;
    }
    Some(TextCall {
        name: name.to_string(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLS: &[&str] = &[
        "run_terminal",
        "terminal_job",
        "terminal_output",
        "terminal_kill",
        "web_search",
        "fetch_url",
    ];

    /// The shape DeepSeek's flash model really produced, full-width pipes and all.
    const DSML: &str = "Pulpit nie istnieje. Szukam PZUpdater.\n\
<\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} calls>\n\
<\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} invoke name=\"run_terminal\">\n\
<\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} parameter name=\"command\">dir C:\\Users</\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} parameter>\n\
</\u{ff5c}\u{ff5c}DSML\u{ff5c}\u{ff5c} invoke>";
    const PLAIN: &str = "<|DSML|invoke name=\"terminal_job\">\n\
<|DSML|parameter name=\"command\">start PZUpdater.exe</|DSML|parameter>\n\
</|DSML|invoke>";

    #[test]
    fn reads_the_full_width_form() {
        let calls = parse(DSML, TOOLS);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "run_terminal");
        assert_eq!(calls[0].args["command"], "dir C:\\Users");
    }

    #[test]
    fn reads_the_plain_form() {
        let calls = parse(PLAIN, TOOLS);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "terminal_job");
        assert_eq!(calls[0].args["command"], "start PZUpdater.exe");
    }

    #[test]
    fn reads_two_calls_in_one_reply() {
        let both = format!("{DSML}\n{PLAIN}");
        assert_eq!(parse(&both, TOOLS).len(), 2);
    }

    #[test]
    fn reads_a_json_block() {
        let text = "I will look it up.\n```json\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"pogoda Kraków\"}}\n```";
        let calls = parse(text, TOOLS);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].args["query"], "pogoda Kraków");
    }

    #[test]
    fn reads_arguments_that_arrived_as_a_string() {
        let text = "{\"name\": \"run_terminal\", \"arguments\": \"{\\\"command\\\": \\\"dir\\\"}\"}";
        let calls = parse(text, TOOLS);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args["command"], "dir");
    }

    #[test]
    fn reads_a_wrapped_call() {
        let text = "{\"tool_call\": {\"name\": \"terminal_kill\", \"arguments\": {\"job\": 2}}}";
        let calls = parse(text, TOOLS);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].args["job"], 2);
    }

    #[test]
    fn keeps_the_numbers_real() {
        let text = "{\"name\": \"terminal_output\", \"arguments\": {\"job\": 3, \"tailChars\": 500}}";
        let calls = parse(text, TOOLS);
        assert!(calls[0].args["job"].is_number());
        assert!(calls[0].args["tailChars"].is_number());
    }

    #[test]
    fn an_answer_without_calls_stays_untouched() {
        assert!(parse("Zwykła odpowiedź, żadnych narzędzi.", TOOLS).is_empty());
        let prose = "PZUpdater jest na pulpicie.";
        assert_eq!(strip(prose, TOOLS), prose);
    }

    #[test]
    fn ignores_json_that_is_not_a_call() {
        assert!(parse("{\"weather\": \"deszcz\", \"city\": \"Kraków\"}", TOOLS).is_empty());
        assert!(parse("{\"name\": \"człowiek\"}", TOOLS).is_empty());
    }

    #[test]
    fn stripping_leaves_only_the_prose() {
        let text = format!("Pulpit nie istnieje.\n{PLAIN}\nSzukam dalej.");
        let stripped = strip(&text, TOOLS);
        assert!(!stripped.contains("DSML"), "stripped: {stripped}");
        assert!(!stripped.contains("invoke"), "stripped: {stripped}");
        assert!(stripped.contains("Pulpit nie istnieje."));
        assert!(stripped.contains("Szukam dalej."));
    }

    #[test]
    fn stripping_a_bare_call_leaves_nothing() {
        assert_eq!(strip(PLAIN, TOOLS), "");
    }

    #[test]
    fn stripping_a_fenced_json_block_drops_the_fence() {
        let text = "Sprawdzam.\n```json\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"x\"}}\n```";
        assert_eq!(strip(text, TOOLS), "Sprawdzam.");
    }

    #[test]
    fn a_parameter_holding_broken_json_stays_text() {
        let text = "<|DSML|invoke name=\"run_terminal\">\n\
<|DSML|parameter name=\"command\">echo {not json}</|DSML|parameter>\n\
</|DSML|invoke>";
        let calls = parse(text, TOOLS);
        assert_eq!(calls[0].args["command"], "echo {not json}");
    }
}
