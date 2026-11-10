//! Sending mail via SMTP (lettre).

use crate::config::{AccountConfig, Security};
use anyhow::{Context, Result};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

#[derive(Debug, Clone, Default)]
pub struct Outgoing {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<crate::mime::Attachment>,
    /// HTML alternative of `body` (sent as multipart/alternative).
    pub html: Option<String>,
    /// Images referenced from `html` via `cid:` (multipart/related).
    pub inline: Vec<crate::mime::Attachment>,
}

fn content_type(mime: &str) -> ContentType {
    ContentType::parse(mime).unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap())
}

/// A recipient list like `Rueter, Marc <marc@host>, a@b`: items are split at
/// commas and semicolons that sit outside angle brackets and quoted display
/// names. A leading fragment with no `@` and no `<` keeps consuming commas —
/// that is an unquoted display name ("Rueter, Marc <…>" from a reply header);
/// such a fragment before a plain address or at the end of the list makes the
/// item invalid on purpose, so half-typed input surfaces as an error.
pub fn split_recipients(s: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut angle = 0usize;
    let mut quote = false;
    let mut name_only = true;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' if angle == 0 => quote = !quote,
            '<' if !quote => {
                angle += 1;
                name_only = false;
            }
            '>' if !quote && angle > 0 => angle -= 1,
            '@' if angle == 0 && !quote => name_only = false,
            ',' | ';' if !quote && angle == 0 => {
                if name_only && s[start..i].contains(|c: char| !c.is_whitespace()) {
                    continue;
                }
                items.push(&s[start..i]);
                start = i + 1;
                name_only = true;
            }
            _ => {}
        }
    }
    items.push(&s[start..]);
    items.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// Separates `Display Name <user@host>` into its parts, lenient about commas
/// in the display name. `None` for a bare `user@host`.
pub fn mailbox_parts(item: &str) -> (Option<String>, String) {
    let item = item.trim();
    if let (Some(open), Some(close)) = (item.rfind('<'), item.rfind('>')) {
        if close > open {
            let name = unquote_name(item[..open].trim());
            let email = item[open + 1..close].trim().to_string();
            return (((!name.is_empty())).then_some(name), email);
        }
    }
    (None, item.to_string())
}

fn unquote_name(name: &str) -> String {
    let s = name.trim();
    let s = if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') { &s[1..s.len() - 1] } else { s };
    // Unescape \" and \\ (in that order, via state machine).
    let mut out = String::with_capacity(s.len());
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

/// The text a composer field shows for one recipient: names containing
/// separators are quoted ("Arning, Fabian <…>") so the text parses back.
pub fn mailbox_text(name: &str, addr: &str) -> String {
    if name.trim().is_empty() {
        return addr.to_string();
    }
    if name.contains([',', ';', '"', '<', '>', '\\']) {
        let quoted = name.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{quoted}\" <{addr}>")
    } else {
        format!("{name} <{addr}>")
    }
}

fn parse_recipient(p: &str) -> Result<Mailbox> {
    let (name, email) = mailbox_parts(p);
    let address: Address = email
        .parse()
        .with_context(|| format!("invalid address: {}", p.trim()))?;
    Ok(Mailbox::new(name, address))
}

pub fn parse_recipients(s: &str) -> Result<Vec<Mailbox>> {
    split_recipients(s).into_iter().map(parse_recipient).collect()
}

/// Builds the RFC 5322 message. Returned separately so a copy can be
/// appended to the Sent folder.
pub fn build(acc: &AccountConfig, out: &Outgoing) -> Result<lettre::Message> {
    let from: Mailbox = if acc.display_name.is_empty() {
        acc.email.parse()?
    } else {
        format!("{} <{}>", acc.display_name, acc.email).parse()?
    };
    let mut b = lettre::Message::builder().from(from).subject(&out.subject);
    let to = parse_recipients(&out.to)?;
    anyhow::ensure!(!to.is_empty() || !out.cc.trim().is_empty() || !out.bcc.trim().is_empty(), "no recipients");
    for m in to {
        b = b.to(m);
    }
    for m in parse_recipients(&out.cc)? {
        b = b.cc(m);
    }
    for m in parse_recipients(&out.bcc)? {
        b = b.bcc(m);
    }
    if let Some(id) = &out.in_reply_to {
        b = b.in_reply_to(id.clone());
    }
    if let Some(r) = &out.references {
        b = b.references(r.clone());
    }
    // text/plain, or alternative(plain, html), with html wrapped in
    // related(html, images…) when it has inline images.
    let alternative = |h: &String| -> MultiPart {
        if out.inline.is_empty() {
            return MultiPart::alternative_plain_html(out.body.clone(), h.clone());
        }
        let mut related = MultiPart::related().singlepart(SinglePart::html(h.clone()));
        for a in &out.inline {
            let cid = a.content_id.clone().unwrap_or_default();
            related = related.singlepart(Attachment::new_inline(cid).body(a.data.as_ref().clone(), content_type(&a.mime)));
        }
        MultiPart::alternative().singlepart(SinglePart::plain(out.body.clone())).multipart(related)
    };
    if out.attachments.is_empty() {
        return Ok(match &out.html {
            Some(h) => b.multipart(alternative(h))?,
            None => b.header(ContentType::TEXT_PLAIN).body(out.body.clone())?,
        });
    }
    let mut mp = match &out.html {
        Some(h) => MultiPart::mixed().multipart(alternative(h)),
        None => MultiPart::mixed().singlepart(SinglePart::plain(out.body.clone())),
    };
    for a in &out.attachments {
        mp = mp.singlepart(Attachment::new(a.name.clone()).body(a.data.as_ref().clone(), content_type(&a.mime)));
    }
    Ok(b.multipart(mp)?)
}

/// A draft as RFC 5322 bytes. Unlike `build` it accepts missing or
/// half-typed recipients. The body is stored as composer markup so
/// reopening keeps the formatting; `html` is for other mail programs.
/// `forward` writes the source id of a rich forward, so reopening knows
/// which stored message to re-load the HTML body and images from.
pub fn build_draft(acc: &AccountConfig, out: &Outgoing, message_id: &str, signature: Option<&str>, forward: Option<i64>) -> Result<Vec<u8>> {
    use mail_builder::headers::address::Address;
    use mail_builder::headers::text::Text;
    fn addrs(s: &str) -> Vec<Address<'static>> {
        split_recipients(s)
            .into_iter()
            .map(|p| {
                let (name, email) = mailbox_parts(p);
                Address::new_address(name, email)
            })
            .collect()
    }
    let id = message_id.trim().trim_start_matches('<').trim_end_matches('>').to_string();
    let from = Address::new_address(
        (!acc.display_name.is_empty()).then(|| acc.display_name.clone()),
        acc.email.clone(),
    );
    let mut b = mail_builder::MessageBuilder::new()
        .from(from)
        .subject(out.subject.clone())
        .message_id(id)
        .date(chrono::Utc::now().timestamp())
        .text_body(out.body.clone());
    for (field, list) in [("to", &out.to), ("cc", &out.cc), ("bcc", &out.bcc)] {
        let a = addrs(list);
        if !a.is_empty() {
            b = match field {
                "to" => b.to(a),
                "cc" => b.cc(a),
                _ => b.bcc(a),
            };
        }
    }
    let strip = |s: &str| s.trim().trim_start_matches('<').trim_end_matches('>').to_string();
    if let Some(r) = &out.in_reply_to {
        b = b.in_reply_to(strip(r));
    }
    if let Some(r) = &out.references {
        let refs: Vec<String> = r.split_whitespace().map(strip).filter(|s| !s.is_empty()).collect();
        if !refs.is_empty() {
            b = b.references(refs);
        }
    }
    if let Some(sig) = signature {
        b = b.header("X-Mailix-Signature", Text::new(sig.to_string()));
    }
    if let Some(source) = forward {
        b = b.header("X-Mailix-Forward", Text::new(source.to_string()));
    }
    if let Some(h) = &out.html {
        b = b.html_body(h.clone());
    }
    for a in &out.attachments {
        b = b.attachment(a.mime.clone(), a.name.clone(), a.data.as_ref().clone());
    }
    Ok(b.write_to_vec()?)
}

pub async fn send(acc: &AccountConfig, password: &str, msg: &lettre::Message) -> Result<()> {
    let s = &acc.smtp;
    let mut tls = TlsParameters::builder(s.host.clone());
    if s.accept_invalid_certs {
        tls = tls.dangerous_accept_invalid_certs(true).dangerous_accept_invalid_hostnames(true);
    }
    let tls = tls.build()?;
    let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&s.host)
        .port(s.port)
        .tls(match s.security {
            Security::Tls => Tls::Wrapper(tls),
            Security::StartTls => Tls::Required(tls),
            Security::Plain => Tls::None,
        })
        .credentials(Credentials::new(s.username.clone(), password.to_string()))
        .timeout(Some(std::time::Duration::from_secs(30)));
    builder.build().send(msg.clone()).await.context("SMTP send failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Preset;

    #[test]
    fn parses_display_names_with_separators() {
        let list = "Rueter, Marc <marc.rueter@bega-consult.de>, \"Arning, Fabian; jr\" <arning@bega-consult.de>;carol@example.com";
        let got = parse_recipients(list).unwrap();
        let have: Vec<_> = got.iter().map(|m| (m.name.clone().unwrap_or_default(), m.email.to_string())).collect();
        assert_eq!(
            have,
            vec![
                ("Rueter, Marc".to_string(), "marc.rueter@bega-consult.de".to_string()),
                ("Arning, Fabian; jr".to_string(), "arning@bega-consult.de".to_string()),
                ("".to_string(), "carol@example.com".to_string()),
            ],
            "{got:?}"
        );
        assert!(parse_recipients("Rueter").is_err());
        assert!(parse_recipients("a@b, <broken>").is_err());
    }

    #[test]
    fn mailbox_text_quotes_names() {
        assert_eq!(mailbox_text("Rueter, Marc", "a@b"), "\"Rueter, Marc\" <a@b>");
        assert_eq!(mailbox_text("Marc \"M\" Rueter", "a@b"), "\"Marc \\\"M\\\" Rueter\" <a@b>");
        assert_eq!(mailbox_text("Marc", "a@b"), "Marc <a@b>");
        assert_eq!(mailbox_text("", "a@b"), "a@b");
        // Roundtrip: text -> mailbox keeps the name with its comma.
        let m = parse_recipients(&mailbox_text("Rueter, Marc", "a@b")).unwrap();
        assert_eq!(m[0].name.as_deref(), Some("Rueter, Marc"));
        assert_eq!(m[0].email.to_string(), "a@b");
    }

    #[test]
    fn writes_quoted_names_to_headers() {
        let acc = AccountConfig::from_preset(Preset::Generic, "me@example.com");
        let out = Outgoing {
            to: mailbox_text("Rueter, Marc", "marc.rueter@bega-consult.de"),
            subject: "AW: Dispotool".into(),
            body: "hi".into(),
            ..Default::default()
        };
        let raw = String::from_utf8(build(&acc, &out).unwrap().formatted()).unwrap();
        let folded = raw.replace("\r\n ", "").replace("\r\n\t", "");
        // Quoted name is RFC 2047 encoded (=?utf-8?b?<base64>?=); decode it.
        let enc = folded.split("To: ").nth(1).unwrap().split(" <").next().unwrap();
        let payload = enc.trim().trim_start_matches("=?utf-8?b?").trim_end_matches("?=");
        use base64::Engine;
        assert_eq!(
            base64::engine::general_purpose::STANDARD.decode(payload).unwrap(),
            b"Rueter, Marc",
            "{raw}"
        );
        assert!(folded.contains("<marc.rueter@bega-consult.de>"), "{raw}");
        // A saved draft keeps the name too.
        let draft = String::from_utf8(build_draft(&acc, &out, "<id@x>", None, None).unwrap()).unwrap();
        assert!(draft.contains("marc.rueter@bega-consult.de"), "{draft}");
    }

    #[test]
    fn builds_reply() {
        let mut acc = AccountConfig::from_preset(Preset::Generic, "me@example.com");
        acc.display_name = "Me".into();
        let m = build(
            &acc,
            &Outgoing {
                to: "Bob <bob@example.com>, carol@example.com".into(),
                subject: "Re: hi".into(),
                body: "text".into(),
                in_reply_to: Some("<abc@x>".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let raw = String::from_utf8(m.formatted()).unwrap();
        assert!(raw.contains("In-Reply-To: <abc@x>"));
        assert!(raw.contains("carol@example.com"));
        assert!(build(&acc, &Outgoing::default()).is_err());

        let with_file = build(
            &acc,
            &Outgoing {
                to: "bob@example.com".into(),
                subject: "file".into(),
                body: "see attached".into(),
                attachments: vec![crate::mime::Attachment {
                    name: "report.pdf".into(),
                    mime: "application/pdf".into(),
                    data: std::sync::Arc::new(b"%PDF-1.4".to_vec()),
                    content_id: None,
                    embedded: false,
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let html = build(
            &acc,
            &Outgoing {
                to: "bob@example.com".into(),
                subject: "fmt".into(),
                body: "Hi Bob".into(),
                html: Some("<p>Hi <strong>Bob</strong></p>".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let p = crate::mime::parse(&html.formatted());
        assert!(p.html.as_deref().unwrap().contains("<strong>Bob</strong>"));
        let parsed = crate::mime::parse(&with_file.formatted());
        let files: Vec<_> = parsed.files().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1.name, "report.pdf");
        assert_eq!(files[0].1.data.as_slice(), b"%PDF-1.4");

        // HTML signature with an embedded image → multipart/related, not a file.
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let (sig, inline) = crate::signature::inline_images(&format!("<b>Me</b><img src='data:image/png;base64,{png}'>"));
        for with_attachment in [false, true] {
            let m = build(
                &acc,
                &Outgoing {
                    to: "bob@example.com".into(),
                    subject: "sig".into(),
                    body: "Hi\n\n-- \nMe".into(),
                    html: Some(format!("<p>Hi</p>{sig}")),
                    inline: inline.clone(),
                    attachments: if with_attachment { with_file_attachment() } else { Vec::new() },
                    ..Default::default()
                },
            )
            .unwrap();
            let raw = m.formatted();
            assert!(String::from_utf8_lossy(&raw).contains("multipart/related"));
            let p = crate::mime::parse(&raw);
            assert_eq!(p.files().count(), usize::from(with_attachment));
            let imgs = p.local_images();
            assert_eq!(imgs.len(), 1, "{:?}", imgs.iter().map(|i| &i.0).collect::<Vec<_>>());
            assert_eq!(&imgs[0].1[..4], b"\x89PNG");
        }
    }

    fn with_file_attachment() -> Vec<crate::mime::Attachment> {
        vec![crate::mime::Attachment {
            name: "report.pdf".into(),
            mime: "application/pdf".into(),
            data: std::sync::Arc::new(b"%PDF-1.4".to_vec()),
            content_id: None,
            embedded: false,
        }]
    }
}
