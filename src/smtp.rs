//! Sending mail via SMTP (lettre).

use crate::config::{AccountConfig, Security};
use anyhow::{Context, Result};
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

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

pub fn parse_recipients(s: &str) -> Result<Vec<Mailbox>> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<Mailbox>().with_context(|| format!("invalid address: {p}")))
        .collect()
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
pub fn build_draft(acc: &AccountConfig, out: &Outgoing, message_id: &str, signature: Option<&str>) -> Result<Vec<u8>> {
    use mail_builder::headers::address::Address;
    use mail_builder::headers::text::Text;
    fn addrs(s: &str) -> Vec<Address<'static>> {
        s.split([',', ';'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| match p.rsplit_once('<') {
                Some((name, rest)) => {
                    let name = name.trim().trim_matches('"').to_string();
                    let email = rest.trim_end_matches('>').trim().to_string();
                    Address::new_address((!name.is_empty()).then_some(name), email)
                }
                None => Address::new_address(None::<String>, p.to_string()),
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
