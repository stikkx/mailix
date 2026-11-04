//! Lightweight formatting for the composer.
//!
//! The editor stores text with small inline markers (shown dimmed while
//! editing, the formatted text itself is rendered bold/italic/…):
//!
//! | toolbar        | markup             |
//! |----------------|--------------------|
//! | bold           | `**text**`         |
//! | italic         | `*text*`           |
//! | underline      | `__text__`         |
//! | strikethrough  | `~~text~~`         |
//! | link           | `[text](https://…)`|
//! | bullet list    | `- item`           |
//! | numbered list  | `1. item`          |
//! | quote          | `> text`           |
//!
//! On send the markup becomes HTML (plus a clean plain-text alternative).

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seg<'a> {
    /// Formatting marker (hidden in output).
    Marker(Range<usize>),
    Text(Range<usize>, &'a str, Style),
    Link { range: Range<usize>, text: &'a str, url: &'a str, style: Style },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Bullet,
    Numbered,
    Quote,
}

/// Splits off a block prefix ("- ", "1. ", "> "): (kind, prefix length).
pub fn line_kind(line: &str) -> (LineKind, usize) {
    if line.starts_with("- ") || line.starts_with("* ") {
        return (LineKind::Bullet, 2);
    }
    if let Some(rest) = line.strip_prefix("> ") {
        let _ = rest;
        return (LineKind::Quote, 2);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(". ") {
        return (LineKind::Numbered, digits + 2);
    }
    (LineKind::Normal, 0)
}

/// Inline formatting of one line (after its block prefix). Ranges are byte
/// offsets into `line`, starting at `offset`.
pub fn inline<'a>(line: &'a str, offset: usize) -> Vec<Seg<'a>> {
    let mut segs = Vec::new();
    let mut style = Style::default();
    let mut text_start = offset;
    let mut i = offset;
    let b = line.as_bytes();

    let flush = |segs: &mut Vec<Seg<'a>>, from: usize, to: usize, style: Style| {
        if to > from {
            segs.push(Seg::Text(from..to, &line[from..to], style));
        }
    };

    while i < line.len() {
        let rest = &line[i..];
        // Two-character toggles; only open when a closing marker follows.
        let toggle = [("**", 0), ("__", 1), ("~~", 2)].into_iter().find(|(m, _)| rest.starts_with(m));
        if let Some((m, which)) = toggle {
            let on = match which {
                0 => style.bold,
                1 => style.underline,
                _ => style.strike,
            };
            if on || line[i + 2..].contains(m) {
                flush(&mut segs, text_start, i, style);
                segs.push(Seg::Marker(i..i + 2));
                match which {
                    0 => style.bold = !on,
                    1 => style.underline = !on,
                    _ => style.strike = !on,
                }
                i += 2;
                text_start = i;
                continue;
            }
        }
        if b[i] == b'*' {
            let closing = line[i + 1..].contains('*');
            let opens = !style.italic && closing && b.get(i + 1).is_some_and(|c| !c.is_ascii_whitespace());
            if style.italic || opens {
                flush(&mut segs, text_start, i, style);
                segs.push(Seg::Marker(i..i + 1));
                style.italic = !style.italic;
                i += 1;
                text_start = i;
                continue;
            }
        }
        if b[i] == b'[' {
            if let Some(close) = rest.find("](") {
                if let Some(end) = rest[close + 2..].find(')') {
                    let url = &rest[close + 2..close + 2 + end];
                    let text = &rest[1..close];
                    if !text.is_empty()
                        && !text.contains(']')
                        && (url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:"))
                    {
                        flush(&mut segs, text_start, i, style);
                        let total = close + 2 + end + 1;
                        segs.push(Seg::Link { range: i..i + total, text, url, style });
                        i += total;
                        text_start = i;
                        continue;
                    }
                }
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    flush(&mut segs, text_start, line.len(), style);
    segs
}

pub fn has_markup(text: &str) -> bool {
    text.lines().any(|l| {
        let (kind, n) = line_kind(l);
        kind != LineKind::Normal || inline(l, n).iter().any(|s| !matches!(s, Seg::Text(..)))
    })
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

fn styled_html(text: &str, st: Style) -> String {
    let mut s = escape(text);
    if st.bold {
        s = format!("<strong>{s}</strong>");
    }
    if st.italic {
        s = format!("<em>{s}</em>");
    }
    if st.underline {
        s = format!("<u>{s}</u>");
    }
    if st.strike {
        s = format!("<s>{s}</s>");
    }
    s
}

fn inline_html(line: &str, offset: usize) -> String {
    inline(line, offset)
        .into_iter()
        .map(|seg| match seg {
            Seg::Marker(_) => String::new(),
            Seg::Text(_, t, st) => styled_html(t, st),
            Seg::Link { text, url, style, .. } => format!("<a href=\"{}\">{}</a>", escape(url), styled_html(text, style)),
        })
        .collect()
}

fn inline_plain(line: &str, offset: usize) -> String {
    inline(line, offset)
        .into_iter()
        .map(|seg| match seg {
            Seg::Marker(_) => String::new(),
            Seg::Text(_, t, _) => t.to_string(),
            Seg::Link { text, url, .. } if text == url => url.to_string(),
            Seg::Link { text, url, .. } => format!("{text} <{url}>"),
        })
        .collect()
}

/// Markup → HTML body fragment.
pub fn to_html(text: &str) -> String {
    let mut out = String::new();
    let mut open: Option<LineKind> = None;
    let mut para: Vec<String> = Vec::new();
    let close = |out: &mut String, open: &mut Option<LineKind>| {
        match open.take() {
            Some(LineKind::Bullet) => out.push_str("</ul>\n"),
            Some(LineKind::Numbered) => out.push_str("</ol>\n"),
            Some(LineKind::Quote) => out.push_str("</blockquote>\n"),
            _ => {}
        }
    };
    let flush_para = |out: &mut String, para: &mut Vec<String>| {
        if !para.is_empty() {
            out.push_str(&format!("<p>{}</p>\n", para.join("<br>")));
            para.clear();
        }
    };
    for line in text.lines() {
        let (kind, n) = line_kind(line);
        if kind == LineKind::Normal {
            close(&mut out, &mut open);
            if line.trim().is_empty() {
                flush_para(&mut out, &mut para);
            } else {
                para.push(inline_html(line, 0));
            }
            continue;
        }
        flush_para(&mut out, &mut para);
        if open != Some(kind) {
            close(&mut out, &mut open);
            out.push_str(match kind {
                LineKind::Bullet => "<ul>\n",
                LineKind::Numbered => "<ol>\n",
                _ => "<blockquote style=\"margin:0 0 0 .8em;padding-left:.8em;border-left:3px solid #ccc;color:#555\">\n",
            });
            open = Some(kind);
        }
        match kind {
            LineKind::Quote => out.push_str(&format!("{}<br>\n", inline_html(line, n))),
            _ => out.push_str(&format!("<li>{}</li>\n", inline_html(line, n))),
        }
    }
    flush_para(&mut out, &mut para);
    close(&mut out, &mut open);
    out
}

/// Markup → readable plain text (markers removed, lists kept).
pub fn to_plain(text: &str) -> String {
    text.lines()
        .map(|line| {
            let (kind, n) = line_kind(line);
            let prefix = match kind {
                LineKind::Bullet => "• ".to_string(),
                LineKind::Numbered | LineKind::Quote => line[..n].to_string(),
                LineKind::Normal => String::new(),
            };
            format!("{prefix}{}", inline_plain(line, n))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plain text (signature, quoted original) → HTML, no markup parsing.
pub fn plain_html(text: &str) -> String {
    escape(text).replace('\n', "<br>\n")
}

/// Full HTML document for sending.
pub fn document(body_html: &str) -> String {
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"></head>\n\
         <body style=\"font-family:-apple-system,Segoe UI,Helvetica,Arial,sans-serif;font-size:14px;line-height:1.5\">\n\
         {body_html}</body></html>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_styles() {
        let s = inline("a **b** *c* __d__ ~~e~~ [f](https://x.y) 2*3", 0);
        let texts: Vec<_> = s
            .iter()
            .filter_map(|g| match g {
                Seg::Text(_, t, st) => Some((*t, *st)),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&("b", Style { bold: true, ..Default::default() })));
        assert!(texts.contains(&("c", Style { italic: true, ..Default::default() })));
        assert!(texts.contains(&("d", Style { underline: true, ..Default::default() })));
        assert!(texts.contains(&("e", Style { strike: true, ..Default::default() })));
        assert!(s.iter().any(|g| matches!(g, Seg::Link { text: "f", url: "https://x.y", .. })));
        // A lone asterisk isn't formatting.
        assert!(texts.iter().any(|(t, _)| t.contains("2*3")));
    }

    #[test]
    fn html_output() {
        let md = "Hi **Bob**,\nline two\n\n- one\n- *two*\n\n1. first\n> quoted <b>\nSee [site](https://x.y?a=1&b=2)";
        let h = to_html(md);
        assert!(h.contains("<p>Hi <strong>Bob</strong>,<br>line two</p>"), "{h}");
        assert!(h.contains("<ul>\n<li>one</li>\n<li><em>two</em></li>\n</ul>"), "{h}");
        assert!(h.contains("<ol>\n<li>first</li>\n</ol>"), "{h}");
        assert!(h.contains("quoted &lt;b&gt;<br>"), "{h}");
        assert!(h.contains("<a href=\"https://x.y?a=1&amp;b=2\">site</a>"), "{h}");
        assert_eq!(to_plain("**Hi** [x](https://x.y)\n- a"), "Hi x <https://x.y>\n• a");
        assert!(has_markup("a **b**"));
        assert!(!has_markup("just text, 2*3=6"));
    }

    #[test]
    fn unicode_safe() {
        let s = inline("Grüße **für** dich", 0);
        assert!(s.iter().any(|g| matches!(g, Seg::Text(_, "für", st) if st.bold)));
    }
}
