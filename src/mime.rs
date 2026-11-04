//! Turning a raw RFC 5322 message into what the reader shows: formatted
//! HTML (as styled text runs), plain text and attachments.

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub data: Arc<Vec<u8>>,
    /// Referenced from the HTML body as `cid:…` (not offered as download).
    pub content_id: Option<String>,
    pub embedded: bool,
}

impl Attachment {
    pub fn size_label(&self) -> String {
        human_size(self.data.len())
    }
}

/// One run of formatted text from an HTML body.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub link: Option<String>,
    pub strong: bool,
    pub em: bool,
    pub code: bool,
    pub strike: bool,
    /// `src` of an <img>; `text` holds its alt text.
    pub image: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Parsed {
    pub html: Option<String>,
    pub rich: Vec<Run>,
    pub attachments: Vec<Attachment>,
}

impl Parsed {
    /// Attachments the user should see (not images embedded in the HTML).
    pub fn files(&self) -> impl Iterator<Item = (usize, &Attachment)> {
        self.attachments.iter().enumerate().filter(|(_, a)| !a.embedded)
    }
}

pub fn parse(raw: &[u8]) -> Parsed {
    let Some(msg) = mail_parser::MessageParser::default().parse(raw) else { return Parsed::default() };
    // body_html() also synthesises HTML from plain text; only use it when
    // the message really has an HTML part.
    let has_html = msg.html_body().any(|p| p.is_content_type("text", "html"));
    let html = if has_html { msg.body_html(0).map(|h| h.into_owned()) } else { None };
    let html_refs = html.as_deref().unwrap_or("");
    let attachments = msg
        .attachments()
        .map(|p| {
            let mime = p
                .content_type()
                .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".into())
                .to_lowercase();
            let content_id = p.content_id().map(|c| c.trim_matches(['<', '>']).to_string());
            let embedded = p.is_inline()
                && content_id.as_ref().is_some_and(|cid| html_refs.contains(&format!("cid:{cid}")));
            Attachment {
                name: sanitize_name(p.attachment_name().unwrap_or("attachment")),
                mime,
                data: Arc::new(p.decoded().into_owned()),
                content_id,
                embedded,
            }
        })
        .collect();
    let rich = html.as_deref().map(html_runs).unwrap_or_default();
    Parsed { html, rich, attachments }
}

/// HTML → formatted runs. Paragraph structure is kept; layout tables,
/// colours and remote content are dropped (privacy + readable in dark mode).
pub fn html_runs(html: &str) -> Vec<Run> {
    use html2text::render::RichAnnotation as A;
    // Huge width: we only want the structure; iced does the wrapping.
    let html = normalize_tags(html);
    // Raw mode reads table cells one after another: mail layouts (signatures,
    // newsletters) are tables, and laid out as columns at this width they
    // turn into huge gaps and stray `|` separators.
    let Ok(lines) = html2text::config::rich().raw_mode(true).lines_from_read(html.as_bytes(), 10_000) else {
        return Vec::new();
    };
    let mut runs: Vec<Run> = Vec::new();
    let mut blank = 0;
    for line in lines {
        let mut any = false;
        let mut first = true;
        let mut heading = false;
        for ts in line.tagged_strings() {
            if ts.s.is_empty() {
                continue;
            }
            let mut text = ts.s.clone();
            if first {
                first = false;
                // Leading &nbsp; padding (e.g. left over from removed icons).
                if text.starts_with('\u{a0}') {
                    text = text.trim_start_matches([' ', '\u{a0}']).to_string();
                }
                // html2text marks headings with "# " and list items with "* ".
                let trimmed = text.trim_start_matches('#');
                if trimmed.len() < text.len() && trimmed.starts_with(' ') {
                    heading = true;
                    text = trimmed[1..].to_string();
                } else if let Some(rest) = text.strip_prefix("* ") {
                    text = format!("•  {rest}");
                } else if let Some(i) = text.find("* ").filter(|&i| text[..i].trim().is_empty() && i > 0) {
                    text = format!("{}•  {}", &text[..i], &text[i + 2..]);
                }
            }
            let mut r = Run { text, strong: heading, ..Default::default() };
            for a in &ts.tag {
                match a {
                    A::Link(url) => r.link = Some(url.clone()),
                    A::Image(src) => {
                        r.image = Some(src.clone());
                        r.text = r.text.trim().trim_matches(NO_ALT).to_string();
                    }
                    A::Strong => r.strong = true,
                    A::Emphasis => r.em = true,
                    A::Code | A::Preformat(_) => r.code = true,
                    A::Strikeout => r.strike = true,
                    _ => {}
                }
            }
            if !r.text.trim().is_empty() || r.image.is_some() {
                any = true;
            }
            // Merge with previous run when formatting is identical.
            match runs.last_mut() {
                Some(prev) if Run { text: String::new(), ..prev.clone() } == Run { text: String::new(), ..r.clone() } => {
                    prev.text.push_str(&r.text)
                }
                _ => runs.push(r),
            }
        }
        if any {
            blank = 0;
        } else {
            blank += 1;
            if blank > 1 {
                continue;
            }
        }
        match runs.last_mut() {
            Some(prev) if *prev == Run { text: prev.text.clone(), ..Default::default() } => prev.text.push('\n'),
            _ => runs.push(Run { text: "\n".into(), ..Default::default() }),
        }
    }
    while runs.last().is_some_and(|r| r.image.is_none() && r.text.trim().is_empty()) {
        runs.pop();
    }
    runs
}

impl Parsed {
    /// Bytes for images that need no network: `cid:` parts and `data:` URIs.
    pub fn local_images(&self) -> Vec<(String, Vec<u8>)> {
        use base64::Engine;
        let mut out = Vec::new();
        for r in &self.rich {
            let Some(src) = &r.image else { continue };
            if let Some(cid) = src.strip_prefix("cid:") {
                if let Some(a) = self.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid)) {
                    out.push((src.clone(), a.data.as_ref().clone()));
                }
            } else if let Some(data) = src.strip_prefix("data:") {
                if let Some((meta, payload)) = data.split_once(',') {
                    if meta.ends_with(";base64") {
                        let clean: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(clean) {
                            out.push((src.clone(), bytes));
                        }
                    }
                }
            }
        }
        out
    }

    /// Remote image URLs (only fetched when the user asks).
    pub fn remote_images(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .rich
            .iter()
            .filter_map(|r| r.image.clone())
            .filter(|s| s.starts_with("https://") || s.starts_with("http://"))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

/// Formatting of a range of text in the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hl {
    Strong,
    Link,
    Code,
    Muted,
}

/// What the reader shows: selectable text blocks with formatting, and images
/// between them.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Text {
        text: String,
        /// Per line: byte ranges and their formatting.
        lines: Vec<Vec<(std::ops::Range<usize>, Hl)>>,
        /// (line, byte range, url)
        links: Vec<(usize, std::ops::Range<usize>, String)>,
    },
    Image { src: String, link: Option<String> },
}

impl Block {
    /// Link under a cursor position in a text block.
    pub fn link_at(&self, line: usize, column: usize) -> Option<&str> {
        let Block::Text { text, links, .. } = self else { return None };
        // Editor columns count chars; ranges are bytes.
        let l = text.lines().nth(line)?;
        let byte = l.char_indices().nth(column).map(|(i, _)| i).unwrap_or(l.len());
        links.iter().find(|(ln, r, _)| *ln == line && r.start <= byte && byte < r.end.max(r.start + 1)).map(|(_, _, u)| u.as_str())
    }
}

/// Splits runs into blocks. `available(src)` says whether an image can be
/// shown; unavailable images become their alt text (or vanish if they have
/// none — typically tracking pixels).
pub fn blocks(runs: &[Run], available: impl Fn(&str) -> bool) -> Vec<Block> {
    #[derive(Default)]
    struct Cur {
        text: String,
        lines: Vec<Vec<(std::ops::Range<usize>, Hl)>>,
        links: Vec<(usize, std::ops::Range<usize>, String)>,
    }
    impl Cur {
        fn push(&mut self, s: &str, hl: Option<Hl>, link: Option<&str>) {
            if self.lines.is_empty() {
                self.lines.push(Vec::new());
            }
            for (i, part) in s.split('\n').enumerate() {
                if i > 0 {
                    self.text.push('\n');
                    self.lines.push(Vec::new());
                }
                if part.is_empty() {
                    continue;
                }
                let line = self.lines.len() - 1;
                let start = self.text.len() - self.text.rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.text.push_str(part);
                let range = start..start + part.len();
                if let Some(h) = hl {
                    self.lines[line].push((range.clone(), h));
                }
                if let Some(u) = link {
                    self.links.push((line, range, u.to_string()));
                }
            }
        }
        fn flush(&mut self, out: &mut Vec<Block>) {
            let c = std::mem::take(self);
            // Drop leading/trailing blank lines.
            let lead = c.text.chars().take_while(|&ch| ch == '\n').count();
            let mut lines = c.lines;
            let mut links = c.links;
            if lead > 0 {
                lines.drain(..lead.min(lines.len()));
                links.retain(|(l, _, _)| *l >= lead);
                links.iter_mut().for_each(|(l, _, _)| *l -= lead);
            }
            let text = c.text[lead..].trim_end_matches('\n').to_string();
            lines.truncate(text.split('\n').count());
            if !text.trim().is_empty() {
                out.push(Block::Text { text, lines, links });
            }
        }
    }
    let mut out = Vec::new();
    let mut cur = Cur::default();
    for r in runs {
        if let Some(src) = &r.image {
            if available(src) {
                cur.flush(&mut out);
                out.push(Block::Image { src: src.clone(), link: r.link.clone() });
            } else if !r.text.is_empty() {
                cur.push(&format!("[{}]", r.text), Some(Hl::Muted), r.link.as_deref());
            }
            continue;
        }
        let hl = if r.link.is_some() {
            Some(Hl::Link)
        } else if r.code {
            Some(Hl::Code)
        } else if r.strong {
            Some(Hl::Strong)
        } else {
            None
        };
        cur.push(&r.text, hl, r.link.as_deref());
    }
    cur.flush(&mut out);
    out
}

/// Placeholder alt text for images that have none (word joiner: invisible).
pub const NO_ALT: char = '\u{2060}';

/// html2text only annotates <strong>/<em>; mail uses <b>/<i> everywhere.
/// An `<img>` declared at most 24px wide or high.
fn tiny_image(tag: &str) -> bool {
    let num = |key: &str| -> Option<u32> {
        let mut found = None;
        for (i, _) in tag.match_indices(key) {
            // Attribute (`width=`) or CSS (`width:`) — not `max-width`, `data-width`.
            if tag[..i].ends_with(['-', '_']) {
                continue;
            }
            let v = tag[i + key.len()..].trim_start_matches(['=', ':', ' ', '"', '\'']);
            let digits: String = v.chars().take_while(char::is_ascii_digit).collect();
            let unit = &v[digits.len()..];
            if !digits.is_empty() && !unit.starts_with('%') {
                found = digits.parse().ok();
                break;
            }
        }
        found
    };
    num("width").is_some_and(|w| w <= 24) || num("height").is_some_and(|h| h <= 24)
}

fn normalize_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 64);
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let (close, name_start) = if rest.as_bytes().get(1) == Some(&b'/') { (true, 2) } else { (false, 1) };
        let tag = rest.as_bytes().get(name_start).map(|c| c.to_ascii_lowercase());
        let after = rest.as_bytes().get(name_start + 1).copied();
        let ends = matches!(after, Some(b'>' | b' ' | b'\t' | b'\n' | b'\r' | b'/'));
        let repl = match tag {
            Some(b'b') if ends => Some("strong"),
            Some(b'i') if ends => Some("em"),
            _ => None,
        };
        // <img> without alt text produces no text at all in html2text, so the
        // image would vanish. Give it an invisible placeholder alt.
        if !close && rest.len() > 4 && rest[1..4].eq_ignore_ascii_case("img") {
            let end = rest.find('>').unwrap_or(rest.len());
            let tag_l = rest[..end].to_ascii_lowercase();
            // Tiny decorations (phone/mail icons in signatures, spacers,
            // tracking pixels) would each take a line of their own.
            if tiny_image(&tag_l) {
                rest = &rest[(end + 1).min(rest.len())..];
                continue;
            }
            let has_alt = tag_l
                .match_indices("alt=")
                .any(|(i, _)| !matches!(tag_l.as_bytes().get(i + 4..i + 6), Some(b"\"\"") | Some(b"''")));
            if !has_alt {
                out.push_str("<img alt=\"");
                out.push(NO_ALT);
                out.push('"');
                rest = &rest[4..];
                continue;
            }
        }
        match repl {
            Some(r) => {
                out.push_str(if close { "</" } else { "<" });
                out.push_str(r);
                rest = &rest[name_start + 1..];
            }
            None => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Writes the HTML body to a file for viewing in the browser. Embedded
/// images are inlined; scripts are always blocked and remote content
/// (tracking pixels) only loads when `remote` is set.
pub fn html_file(dir: &Path, id: i64, parsed: &Parsed, remote: bool) -> std::io::Result<PathBuf> {
    use base64::Engine;
    let mut html = parsed.html.clone().unwrap_or_default();
    for a in parsed.attachments.iter() {
        if let Some(cid) = &a.content_id {
            let uri = format!(
                "data:{};base64,{}",
                a.mime,
                base64::engine::general_purpose::STANDARD.encode(a.data.as_slice())
            );
            html = html.replace(&format!("cid:{cid}"), &uri);
        }
    }
    let img = if remote { "data: https: http:" } else { "data:" };
    let csp = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {img}; \
         style-src 'unsafe-inline'{}; font-src data:\">\n<meta charset=\"utf-8\">\n",
        if remote { " https:" } else { "" }
    );
    // CSP must come before anything else in the document.
    let doc = format!("<!doctype html>\n{csp}{html}");
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("message-{id}{}.html", if remote { "-remote" } else { "" }));
    std::fs::write(&path, doc)?;
    Ok(path)
}

pub fn sanitize_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '\0' | '<' | '>' | '"' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() { "attachment".into() } else { n }
}

pub fn human_size(n: usize) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIL: &[u8] = b"From: a@x\r\nTo: b@x\r\nSubject: s\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"B\"\r\n\r\n\
--B\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n\
--R\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>bold</b> <a href=\"https://ex.com\">link</a></p><img src=\"cid:logo\">\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <logo>\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n\
--R--\r\n\
--B\r\nContent-Type: application/pdf; name=\"../../etc/passwd.pdf\"\r\nContent-Disposition: attachment; filename=\"../../etc/passwd.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERg==\r\n\
--B--\r\n";

    #[test]
    fn parses_html_and_attachments() {
        let p = parse(MAIL);
        assert!(p.html.as_deref().unwrap().contains("<b>bold</b>"));
        let files: Vec<_> = p.files().collect();
        assert_eq!(files.len(), 1, "{:?}", p.attachments);
        assert_eq!(files[0].1.name, "_.._etc_passwd.pdf");
        assert_eq!(files[0].1.data.as_slice(), b"%PDF");
        assert!(p.attachments.iter().any(|a| a.embedded && a.content_id.as_deref() == Some("logo")));
        assert_eq!(p.local_images(), vec![("cid:logo".to_string(), b"hello".to_vec())]);
        assert!(p.remote_images().is_empty());
        assert!(p.rich.iter().any(|r| r.strong && r.text.contains("bold")));
        assert!(p.rich.iter().any(|r| r.link.as_deref() == Some("https://ex.com")));
        assert!(parse(b"From: a@x\r\nSubject: s\r\n\r\nplain only").html.is_none());
        let runs = html_runs("<h2>Title</h2><ul><li>one</li></ul><img src=x alt=logo>");
        let all: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert!(all.starts_with("Title\n"), "{all:?}");
        assert!(runs[0].strong);
        assert!(all.contains("•  one"), "{all:?}");
        assert!(runs.iter().any(|r| r.image.as_deref() == Some("x") && r.text == "logo"), "{runs:?}");
    }

    #[test]
    fn builds_selectable_blocks() {
        let runs = html_runs("<p>Hi <b>Bob</b></p><p>see <a href=\"https://x.y\">site</a></p><img src=\"cid:a\"><p>end</p><img src=\"https://t/p\">");
        let b = blocks(&runs, |s| s == "cid:a");
        assert_eq!(b.len(), 3, "{b:#?}");
        let Block::Text { text, lines, links } = &b[0] else { panic!() };
        assert_eq!(text, "Hi Bob\n\nsee site");
        assert_eq!(lines[0], vec![(3..6, Hl::Strong)]);
        assert_eq!(links, &vec![(2, 4..8, "https://x.y".to_string())]);
        assert_eq!(b[0].link_at(2, 5), Some("https://x.y"));
        assert_eq!(b[0].link_at(2, 1), None);
        assert_eq!(b[1], Block::Image { src: "cid:a".into(), link: None });
        // Remote image without alt is hidden while blocked.
        let Block::Text { text, .. } = &b[2] else { panic!() };
        assert_eq!(text, "end");
    }

    #[test]
    fn images_without_alt_survive() {
        let runs = html_runs("<p>a</p><img src=\"cid:x\"><p>b</p><img src=\"https://t/p.gif\" alt=\"\">");
        let imgs: Vec<_> = runs.iter().filter_map(|r| r.image.clone()).collect();
        assert_eq!(imgs, vec!["cid:x", "https://t/p.gif"], "{runs:?}");
        assert!(runs.iter().filter(|r| r.image.is_some()).all(|r| r.text.is_empty()));
    }

    #[test]
    fn normalizes_b_and_i() {
        assert_eq!(normalize_tags("<B>x</b> <i class=a>y</I> <br><img src=x>"), "<strong>x</strong> <em class=a>y</em> <br><img alt=\"\u{2060}\" src=x>");
    }

    #[test]
    fn html_file_inlines_cid_and_blocks_remote() {
        let dir = std::env::temp_dir().join("mailix-test-html");
        let path = html_file(&dir, 1, &parse(MAIL), false).unwrap();
        let s = std::fs::read_to_string(path).unwrap();
        assert!(s.contains("data:image/png;base64,aGVsbG8="));
        assert!(s.starts_with("<!doctype html>\n<meta http-equiv=\"Content-Security-Policy\""));
        assert!(s.contains("img-src data:;"));
    }

    #[test]
    fn drops_tiny_images_and_linearizes_tables() {
        let html = "<table><tr><td><b>Hubert</b><br>Developer</td><td><img width=\"110\" src=\"cid:logo\"></td></tr>\
                    <tr><td><img width=\"13\" height=\"13\" style=\"width: 13px\" src=\"cid:phone\">&nbsp; +48 661</td></tr></table>";
        let runs = html_runs(html);
        let text: String = runs.iter().map(|r| r.text.as_str()).collect();
        assert!(!text.contains('|'), "{text:?}");
        assert!(text.contains("Hubert"));
        assert!(text.contains("\n+48 661") || text.starts_with("+48 661"), "{text:?}");
        let imgs: Vec<_> = runs.iter().filter_map(|r| r.image.as_deref()).collect();
        assert_eq!(imgs, ["cid:logo"]);
        assert!(tiny_image("<img style=\"width:14px;height:14px\" src=x"));
        assert!(!tiny_image("<img style=\"max-width:20px\" width=\"200\" src=x"));
        assert!(!tiny_image("<img width=\"100%\" src=x"));
    }

    /// `MAILIX_HTML=/path/message.html cargo test render_file -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn render_file() {
        let html = std::fs::read_to_string(std::env::var("MAILIX_HTML").unwrap()).unwrap();
        for b in blocks(&html_runs(&html), |_| true) {
            match b {
                Block::Text { text, .. } => println!("{text}"),
                Block::Image { src, .. } => println!("[IMAGE {}]", &src[..src.len().min(30)]),
            }
        }
    }
}
