//! Grammar and spelling check via LanguageTool (German and English).
//!
//! Off by default: the text of the message being written is sent to the
//! configured server — a local one (`docker run -p 8010:8010
//! erikvl87/languagetool`) keeps it on the machine.

use crate::config::{GrammarConfig, GrammarLang};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    Spelling,
    Grammar,
    Style,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    /// Byte range in the composer text.
    pub range: Range<usize>,
    pub message: String,
    pub replacements: Vec<String>,
    pub kind: IssueKind,
}

/// German or English, guessed from common words (LanguageTool's own
/// detection may pick other languages for short texts).
pub fn detect(text: &str) -> &'static str {
    const DE: &[&str] = &[
        "der", "die", "das", "und", "ist", "nicht", "ich", "sie", "wir", "mit", "für", "ein", "eine", "zu", "auf",
        "den", "dem", "des", "von", "bitte", "danke", "gerne", "auch", "noch", "aber", "wenn", "dass", "es", "im",
        "hallo", "viele", "grüße", "sind", "haben", "wird", "kann", "uns", "euch", "ihr", "mir", "dir",
    ];
    const EN: &[&str] = &[
        "the", "and", "is", "not", "you", "we", "with", "for", "a", "an", "to", "on", "of", "please", "thanks",
        "thank", "also", "but", "if", "that", "it", "in", "hi", "hello", "best", "regards", "are", "have", "will",
        "can", "us", "your", "my", "me", "this", "be", "would", "could", "i",
    ];
    let (mut de, mut en) = (0, 0);
    for w in text.split(|c: char| !c.is_alphabetic()).filter(|w| !w.is_empty()) {
        let w = w.to_lowercase();
        de += usize::from(DE.contains(&w.as_str()));
        en += usize::from(EN.contains(&w.as_str()));
    }
    // Umlauts are a strong hint.
    if text.contains(['ä', 'ö', 'ü', 'ß', 'Ä', 'Ö', 'Ü']) {
        de += 2;
    }
    if en > de { "en" } else { "de" }
}

fn language(cfg: &GrammarConfig, text: &str) -> String {
    let en = if cfg.english_gb { "en-GB" } else { "en-US" };
    match cfg.language {
        GrammarLang::German => "de-DE".into(),
        GrammarLang::English => en.into(),
        GrammarLang::Auto => if detect(text) == "en" { en.into() } else { "de-DE".into() },
    }
}

/// LanguageTool's annotated input: composer markup (`**`, list prefixes,
/// link targets, quoted lines) is marked as markup so it isn't checked,
/// while offsets still refer to the full text.
pub fn annotate(text: &str) -> Value {
    use crate::markup::{LineKind, Seg, inline, line_kind};
    let mut parts: Vec<Value> = Vec::new();
    let mut push = |kind: &str, s: &str| {
        if s.is_empty() {
            return;
        }
        // Merge with the previous part of the same kind.
        if let Some(last) = parts.last_mut() {
            if let Some(v) = last.get_mut(kind) {
                let joined = format!("{}{s}", v.as_str().unwrap_or(""));
                *v = Value::String(joined);
                return;
            }
        }
        parts.push(json!({ kind: s }));
    };
    let mut lines = text.split('\n').peekable();
    while let Some(line) = lines.next() {
        let (kind, n) = line_kind(line);
        if kind == LineKind::Quote {
            push("markup", line);
        } else {
            push("markup", &line[..n]);
            for seg in inline(line, n) {
                match seg {
                    Seg::Marker(r) => push("markup", &line[r]),
                    Seg::Text(_, t, _) => push("text", t),
                    Seg::Link { range, text: t, .. } => {
                        // "[" text "](url)"
                        push("markup", "[");
                        push("text", t);
                        push("markup", &line[range.start + 1 + t.len()..range.end]);
                    }
                }
            }
        }
        if lines.peek().is_some() {
            push("text", "\n");
        }
    }
    json!({ "annotation": parts })
}

/// Checks `text`; issues on ignored words are left out.
pub async fn check(cfg: &GrammarConfig, text: &str) -> Result<Vec<Issue>> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let base = cfg.server.trim().trim_end_matches('/');
    if base.is_empty() {
        bail!("no LanguageTool server set");
    }
    let url = format!("{base}/v2/check");
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
    let resp = client
        .post(&url)
        .form(&[("data", annotate(text).to_string()), ("language", language(cfg, text))])
        .send()
        .await
        .with_context(|| format!("could not reach {base}"))?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        let msg = body.lines().next().unwrap_or("").chars().take(160).collect::<String>();
        bail!("LanguageTool error ({status}): {msg}");
    }
    let v: Value = serde_json::from_str(&body).context("invalid LanguageTool response")?;
    Ok(parse(&v, text, &cfg.ignored))
}

/// LanguageTool offsets count UTF-16 code units; turn them into byte ranges.
fn parse(v: &Value, text: &str, ignored: &[String]) -> Vec<Issue> {
    let mut utf16_to_byte = Vec::with_capacity(text.len() + 1);
    for (b, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            utf16_to_byte.push(b);
        }
    }
    utf16_to_byte.push(text.len());
    let at = |u: u64| utf16_to_byte.get(u as usize).copied();
    let mut out = Vec::new();
    for m in v["matches"].as_array().into_iter().flatten() {
        let (Some(off), Some(len)) = (m["offset"].as_u64(), m["length"].as_u64()) else { continue };
        let (Some(start), Some(end)) = (at(off), at(off + len)) else { continue };
        if start >= end {
            continue;
        }
        let word = &text[start..end];
        if ignored.iter().any(|w| w == word) {
            continue;
        }
        let kind = match m["rule"]["issueType"].as_str().unwrap_or("") {
            "misspelling" => IssueKind::Spelling,
            "style" | "register" | "locale-violation" | "typographical" | "whitespace" => IssueKind::Style,
            _ => IssueKind::Grammar,
        };
        let message = m["shortMessage"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or(m["message"].as_str())
            .unwrap_or("Possible mistake")
            .to_string();
        let replacements = m["replacements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r["value"].as_str().map(String::from))
            .take(4)
            .collect();
        out.push(Issue { range: start..end, message, replacements, kind });
    }
    out
}

/// Keeps issue ranges valid after `old` was edited into `new`: issues
/// before the change stay, those after it move, overlapping ones go.
pub fn shift(issues: &mut Vec<Issue>, old: &str, new: &str) {
    if old == new {
        return;
    }
    let (o, n) = (old.as_bytes(), new.as_bytes());
    let mut prefix = o.iter().zip(n).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let max_suffix = o.len().min(n.len()) - prefix;
    let mut suffix = o.iter().rev().zip(n.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(o.len() - suffix) || !new.is_char_boundary(n.len() - suffix) {
        suffix -= 1;
    }
    let old_end = o.len() - suffix;
    let delta = n.len() as isize - o.len() as isize;
    issues.retain_mut(|i| {
        if i.range.end <= prefix {
            true
        } else if i.range.start >= old_end {
            i.range = (i.range.start as isize + delta) as usize..(i.range.end as isize + delta) as usize;
            true
        } else {
            false
        }
    });
}

/// Issues as (line, byte range in line, kind) for the editor highlighter.
pub fn by_line(text: &str, issues: &[Issue]) -> Vec<(usize, Range<usize>, IssueKind)> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut out = Vec::new();
    for i in issues {
        let line = starts.partition_point(|&s| s <= i.range.start) - 1;
        let ls = starts[line];
        let le = starts.get(line + 1).map_or(text.len(), |s| s - 1);
        if i.range.end <= le + 1 {
            out.push((line, i.range.start - ls..i.range.end.min(le) - ls, i.kind));
        }
    }
    out
}

/// Byte offset → (line, byte column), as used by the editor cursor.
pub fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count();
    let col = before.rfind('\n').map_or(before.len(), |i| before.len() - i - 1);
    (line, col)
}

/// (line, byte column) → byte offset.
pub fn offset(text: &str, line: usize, col: usize) -> usize {
    let start: usize = text.split('\n').take(line).map(|l| l.len() + 1).sum();
    (start + col).min(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(r: Range<usize>) -> Issue {
        Issue { range: r, message: String::new(), replacements: Vec::new(), kind: IssueKind::Spelling }
    }

    #[test]
    fn detects_language() {
        assert_eq!(detect("Hallo Bob, vielen Dank für die Info"), "de");
        assert_eq!(detect("Hi Bob, thanks for the update, I will check it"), "en");
        assert_eq!(detect("Grüße"), "de");
    }

    #[test]
    fn annotates_markup() {
        let a = annotate("Hi **Bob**\n- see [site](https://x.y)\n> quoted txt");
        let parts = a["annotation"].as_array().unwrap();
        let joined: String =
            parts.iter().map(|p| p["text"].as_str().or(p["markup"].as_str()).unwrap()).collect();
        assert_eq!(joined, "Hi **Bob**\n- see [site](https://x.y)\n> quoted txt");
        let text: String = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        assert_eq!(text, "Hi Bob\nsee site\n");
    }

    #[test]
    fn parses_utf16_offsets() {
        // "😀 Grüsse" — the emoji is two UTF-16 units, four bytes.
        let text = "😀 Grüsse";
        let v = json!({ "matches": [{
            "offset": 3, "length": 6, "shortMessage": "Spelling",
            "replacements": [{ "value": "Grüße" }], "rule": { "issueType": "misspelling" }
        }]});
        let i = parse(&v, text, &[]);
        assert_eq!(&text[i[0].range.clone()], "Grüsse");
        assert_eq!(i[0].replacements, ["Grüße"]);
        assert!(parse(&v, text, &["Grüsse".into()]).is_empty());
    }

    #[test]
    fn shifts_with_edits() {
        let old = "Teh cat and teh dog";
        let mut v = vec![issue(0..3), issue(12..15)];
        shift(&mut v, old, "Teh big cat and teh dog");
        assert_eq!(v.iter().map(|i| i.range.clone()).collect::<Vec<_>>(), [0..3, 16..19]);
        // Editing inside an issue drops it.
        shift(&mut v, "Teh big cat and teh dog", "The big cat and teh dog");
        assert_eq!(v.iter().map(|i| i.range.clone()).collect::<Vec<_>>(), [16..19]);
        // Umlaut edits stay on char boundaries.
        // ü → ö shares the first UTF-8 byte; "foo" (6..9) must stay put.
        let mut w = vec![issue(6..9), issue(0..4)];
        shift(&mut w, "Grü x foo", "Grö x foo");
        assert_eq!(w.iter().map(|i| i.range.clone()).collect::<Vec<_>>(), [6..9]);
        // ß → ss is two bytes either way; the comma adds one.
        let mut w = vec![issue(10..13)];
        shift(&mut w, "Grüße x foo", "Grüsse, x foo");
        assert_eq!(w[0].range, 11..14);
    }

    #[test]
    fn maps_lines_and_positions() {
        let text = "Hello\nWrold here";
        let lines = by_line(text, &[issue(6..11)]);
        assert_eq!(lines, [(1, 0..5, IssueKind::Spelling)]);
        assert_eq!(position(text, 8), (1, 2));
        assert_eq!(offset(text, 1, 2), 8);
    }

    /// Real check against a server: `MAILIX_LT=https://api.languagetool.org cargo test real_check -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn real_check() {
        let cfg = GrammarConfig { enabled: true, server: std::env::var("MAILIX_LT").unwrap(), ..Default::default() };
        let de = check(&cfg, "Hallo Bob,\nich habe die **Datei** gestern geschikt. Das sind ein gute Idee.").await.unwrap();
        println!("{de:#?}");
        assert!(de.iter().any(|i| i.replacements.iter().any(|r| r == "geschickt")));
        let en = check(&cfg, "Hi Bob, I has send the file yesterday. Thanks for you help.").await.unwrap();
        println!("{en:#?}");
        assert!(!en.is_empty());
    }
}
