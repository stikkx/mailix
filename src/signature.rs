//! HTML signatures (e.g. exported from Outlook or a signature generator).
//!
//! On import the HTML is made self-contained: local images become `data:`
//! URIs. On send, `data:` images are turned into inline MIME parts
//! (`cid:`), because Gmail and Outlook don't show `data:` images.

use crate::mime::Attachment;
use anyhow::{Context, Result};
use base64::Engine;
use std::path::Path;
use std::sync::Arc;

/// Reads an HTML signature file: (html, plain-text version).
pub fn import(path: &Path) -> Result<(String, String)> {
    let raw = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
    let html = String::from_utf8_lossy(&raw).into_owned();
    let html = strip_scripts(body_inner(&html)).trim().to_string();
    anyhow::ensure!(!html.is_empty(), "the file contains no HTML");
    let dir = path.parent().unwrap_or(Path::new("."));
    let html = rewrite_srcs(&html, |src| {
        if src.starts_with("data:") || src.starts_with("http://") || src.starts_with("https://") || src.starts_with("cid:")
        {
            return None;
        }
        let file = dir.join(src.strip_prefix("file://").unwrap_or(src));
        let data = std::fs::read(&file).ok()?;
        let mime = mime_guess::from_path(&file).first_or_octet_stream();
        Some(format!("data:{};base64,{}", mime.essence_str(), base64::engine::general_purpose::STANDARD.encode(data)))
    });
    let plain = to_plain(&html);
    Ok((html, plain))
}

pub fn to_plain(html: &str) -> String {
    use html2text::render::RichAnnotation as A;
    let Ok(lines) = html2text::config::rich().raw_mode(true).lines_from_read(html.as_bytes(), 10_000) else {
        return String::new();
    };
    let text: Vec<String> = lines
        .iter()
        .map(|line| {
            line.tagged_strings()
                .filter(|ts| !ts.tag.iter().any(|a| matches!(a, A::Image(..))))
                .map(|ts| ts.s.as_str())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    // Collapse runs of blank lines left by images.
    let mut out: Vec<&str> = Vec::new();
    for l in &text {
        if !(l.is_empty() && out.last().is_some_and(|p| p.is_empty())) {
            out.push(l);
        }
    }
    out.join("\n").trim_matches('\n').to_string()
}

/// Replaces embedded `data:` images with `cid:` references; returns the
/// rewritten HTML and the inline parts.
pub fn inline_images(html: &str) -> (String, Vec<Attachment>) {
    let mut parts = Vec::new();
    let tag: u32 = rand::random();
    let out = rewrite_srcs(html, |src| {
        let (mime, data) = decode_data_uri(src)?;
        let ext = mime_guess::get_mime_extensions_str(&mime).and_then(|e| e.first()).copied().unwrap_or("bin");
        let cid = format!("sig{}.{tag:08x}@mailix", parts.len() + 1);
        parts.push(Attachment {
            name: format!("signature-{}.{ext}", parts.len() + 1),
            mime,
            data: Arc::new(data),
            content_id: Some(cid.clone()),
            embedded: true,
        });
        Some(format!("cid:{cid}"))
    });
    (out, parts)
}

/// Decoded embedded images, for previews.
pub fn images(html: &str) -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    rewrite_srcs(html, |src| {
        if let Some((_, data)) = decode_data_uri(src) {
            v.push(data);
        }
        None
    });
    v
}

pub fn decode_data_uri(src: &str) -> Option<(String, Vec<u8>)> {
    let rest = src.trim().strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let clean: String = payload.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let data = base64::engine::general_purpose::STANDARD.decode(clean).ok()?;
    Some((if mime.is_empty() { "application/octet-stream".into() } else { mime.to_string() }, data))
}

/// Calls `f` for every `src` attribute value (quoted with ' or ") and
/// substitutes the value when `f` returns Some.
fn rewrite_srcs(html: &str, mut f: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(html.len());
    let lower = html.to_ascii_lowercase();
    let mut pos = 0;
    while let Some(off) = lower[pos..].find("src") {
        let start = pos + off;
        // Must be an attribute name: preceded by whitespace, followed by `=`.
        let before_ok = html[..start].chars().next_back().is_some_and(char::is_whitespace);
        let after = &html[start + 3..];
        let eq = after.len() - after.trim_start().len();
        let rest = after.trim_start();
        let Some(rest) = rest.strip_prefix('=').filter(|_| before_ok) else {
            out.push_str(&html[pos..start + 3]);
            pos = start + 3;
            continue;
        };
        let ws = rest.len() - rest.trim_start().len();
        let rest = rest.trim_start();
        let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            out.push_str(&html[pos..start + 3]);
            pos = start + 3;
            continue;
        };
        let value_start = start + 3 + eq + 1 + ws + 1;
        let Some(len) = html[value_start..].find(q) else { break };
        let value = &html[value_start..value_start + len];
        out.push_str(&html[pos..value_start]);
        match f(value) {
            Some(new) => out.push_str(&new),
            None => out.push_str(value),
        }
        pos = value_start + len;
    }
    out.push_str(&html[pos..]);
    out
}

/// The content of `<body>` if the file is a full document.
fn body_inner(html: &str) -> &str {
    let lower = html.to_ascii_lowercase();
    let Some(b) = lower.find("<body") else { return html };
    let Some(open_end) = lower[b..].find('>') else { return html };
    let start = b + open_end + 1;
    let end = lower[start..].find("</body").map_or(html.len(), |e| start + e);
    &html[start..end]
}

fn strip_scripts(html: &str) -> String {
    let mut out = html.to_string();
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(s) = lower.find("<script") else { break };
        let end = lower[s..].find("</script>").map_or(out.len(), |e| s + e + "</script>".len());
        out.replace_range(s..end, "");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    #[test]
    fn inlines_data_images() {
        let html = format!(
            "<b>Markus</b><br><img style='width:200px' src='data:image/png;base64,{PNG_1X1}' alt=\"Logo\">\
             <img src=\"https://x.y/a.png\"><a href='mailto:m@x.de'>m@x.de</a>"
        );
        let (out, parts) = inline_images(&html);
        assert_eq!(parts.len(), 1);
        let cid = parts[0].content_id.clone().unwrap();
        assert!(out.contains(&format!("src='cid:{cid}'")), "{out}");
        assert!(out.contains("src=\"https://x.y/a.png\""));
        assert_eq!(parts[0].mime, "image/png");
        assert_eq!(&parts[0].data[..4], b"\x89PNG");
        assert_eq!(images(&html).len(), 1);
    }

    #[test]
    fn imports_file() {
        let dir = std::env::temp_dir().join(format!("mailix-sig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("logo.png"),
            base64::engine::general_purpose::STANDARD.decode(PNG_1X1).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("sig.html"),
            "<html><head><script>x()</script></head><body><font face='arial'>Grüße<br><b>Markus</b><br>\
             Mail: <a href='mailto:m@x.de'>m@x.de</a><br><img src=\"logo.png\" alt=\"Logo\"></font></body></html>",
        )
        .unwrap();
        let (html, plain) = import(&dir.join("sig.html")).unwrap();
        assert!(html.starts_with("<font"), "{html}");
        assert!(!html.contains("script"));
        assert!(html.contains("src=\"data:image/png;base64,"), "{html}");
        assert!(plain.contains("Grüße\nMarkus\nMail: m@x.de"), "{plain}");
        std::fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod real_file {
    /// `MAILIX_SIG=/path/sig.html cargo test real_signature -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_signature() {
        let path = std::env::var("MAILIX_SIG").expect("set MAILIX_SIG");
        let (html, plain) = super::import(std::path::Path::new(&path)).unwrap();
        let (_, parts) = super::inline_images(&html);
        println!("{plain}\n--- {} bytes html, {} inline image(s): {:?}", html.len(), parts.len(),
            parts.iter().map(|p| (&p.mime, p.data.len())).collect::<Vec<_>>());
        let runs = crate::mime::html_runs(&html);
        for b in crate::mime::blocks(&runs, |s| s.starts_with("data:")) {
            match b {
                crate::mime::Block::Text { text, .. } => println!("TEXT: {:?}", &text[..text.len().min(60)]),
                crate::mime::Block::Image { src, .. } => println!("IMAGE: {}…", &src[..30]),
            }
        }
    }
}
