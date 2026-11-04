//! Optional AI assistant. Off by default; nothing leaves the machine unless
//! the user enables it and triggers an AI action.
//!
//! Providers: Anthropic (native Messages API) and anything OpenAI-compatible
//! (OpenAI, local Ollama, Ollama Cloud, OpenCode Zen, OpenRouter, custom).

use crate::config::{AiConfig, AiProvider};
use crate::secrets;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const ANTHROPIC_VERSION: &str = "2026-10-21";

#[derive(Clone, Copy)]
enum Effort {
    Low,
    Medium,
}

fn key(cfg: &AiConfig) -> Result<Option<String>> {
    // Providers without a key (local Ollama, custom servers) must work on
    // machines without an OS keyring — don't touch the keyring at all.
    if !cfg.provider.needs_key() {
        return Ok(None);
    }
    let k = secrets::get(&cfg.provider.key_name())?;
    if k.is_none() {
        bail!("no API key for {} — add it in Settings → AI assistant", cfg.provider);
    }
    Ok(k)
}

fn base(cfg: &AiConfig) -> String {
    let b = if cfg.base_url.trim().is_empty() { cfg.provider.default_base_url() } else { cfg.base_url.trim() };
    b.trim_end_matches('/').to_string()
}

async fn complete(cfg: &AiConfig, system: &str, user: String, effort: Effort) -> Result<String> {
    let key = key(cfg)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).build()?;
    match cfg.provider {
        AiProvider::Anthropic => anthropic(&client, cfg, key.unwrap_or_default(), system, user, effort).await,
        _ => openai_compatible(&client, cfg, key, system, user).await,
    }
}

async fn anthropic(
    client: &reqwest::Client,
    cfg: &AiConfig,
    key: String,
    system: &str,
    user: String,
    effort: Effort,
) -> Result<String> {
    let body = json!({
        "model": cfg.model,
        "max_tokens": 16000,
        "system": system,
        "output_config": { "effort": match effort { Effort::Low => "low", Effort::Medium => "medium" } },
        // Re-run on Anthropic's recommended model if a safety classifier declines.
        "fallbacks": "default",
        "messages": [{ "role": "user", "content": user }],
    });
    let resp = client
        .post(format!("{}/v1/messages", base(cfg)))
        .header("x-api-key", key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", "server-side-fallback-2026-07-01")
        .json(&body)
        .send()
        .await
        .context("AI request failed")?;
    let status = resp.status();
    let v: Value = resp.json().await.context("invalid AI response")?;
    if !status.is_success() {
        bail!("AI error ({status}): {}", v["error"]["message"].as_str().unwrap_or("unknown error"));
    }
    if v["stop_reason"] == "refusal" {
        bail!("The AI declined this request.");
    }
    Ok(v["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<String>()
        .trim()
        .to_string())
}

async fn openai_compatible(
    client: &reqwest::Client,
    cfg: &AiConfig,
    key: Option<String>,
    system: &str,
    user: String,
) -> Result<String> {
    let body = json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let mut req = client.post(format!("{}/chat/completions", base(cfg))).json(&body);
    if let Some(k) = key {
        req = req.bearer_auth(k);
    }
    let resp = req.send().await.with_context(|| format!("could not reach {}", base(cfg)))?;
    let status = resp.status();
    let text = resp.text().await?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("invalid AI response ({status})"))?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().or(v["error"].as_str()).unwrap_or(&text);
        bail!("AI error ({status}): {msg}");
    }
    let content = v["choices"][0]["message"]["content"].as_str().context("AI returned no text")?;
    Ok(strip_think(content).trim().to_string())
}

/// Reasoning models served via Ollama often prefix their answer with
/// `<think>…</think>`.
fn strip_think(s: &str) -> &str {
    match (s.find("<think>"), s.find("</think>")) {
        (Some(0), Some(end)) => &s[end + "</think>".len()..],
        _ => s,
    }
}

/// Model ids offered by the provider (for the settings dropdown).
pub async fn list_models(cfg: &AiConfig) -> Result<Vec<String>> {
    let key = key(cfg)?;
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20)).build()?;
    let req = match cfg.provider {
        AiProvider::Anthropic => client
            .get(format!("{}/v1/models?limit=100", base(cfg)))
            .header("x-api-key", key.unwrap_or_default())
            .header("anthropic-version", ANTHROPIC_VERSION),
        _ => {
            let r = client.get(format!("{}/models", base(cfg)));
            match key {
                Some(k) => r.bearer_auth(k),
                None => r,
            }
        }
    };
    let resp = req.send().await.with_context(|| format!("could not reach {}", base(cfg)))?;
    let status = resp.status();
    let v: Value = resp.json().await.context("invalid response")?;
    if !status.is_success() {
        bail!("{status}: {}", v["error"]["message"].as_str().unwrap_or("could not list models"));
    }
    let mut ids: Vec<String> =
        v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str().map(String::from)).collect();
    ids.sort();
    Ok(ids)
}

const SEARCH_SYSTEM: &str = "You translate a natural-language request about the user's email into a query \
for a Tantivy full-text index. Fields: subject, from, to, body (default: all). Syntax: plain words are \
ANDed; use OR between alternatives, \"quoted phrases\", field:term (e.g. from:alice), and -term to exclude. \
Include likely synonyms and German/English variants with OR when helpful. Reply with the query only, \
on one line, no explanation.";

/// "the invoice from the hotel in Berlin last spring" → `(invoice OR rechnung) hotel berlin`
pub async fn search_query(cfg: &AiConfig, question: &str) -> Result<String> {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let q = complete(cfg, SEARCH_SYSTEM, format!("Today is {today}.\nRequest: {question}"), Effort::Low).await?;
    Ok(q.lines().find(|l| !l.trim().is_empty()).unwrap_or_default().trim().trim_matches('`').to_string())
}

const WRITE_SYSTEM: &str = "You help write emails. You get the user's instruction, the current draft \
(may be empty) and possibly the email being replied to. Return only the email body text to use as the \
new draft — no subject line, no signature, no commentary. Match the language of the conversation \
unless told otherwise. Keep the user's tone and facts; never invent commitments, dates or numbers.";

pub async fn write(cfg: &AiConfig, instruction: &str, draft: &str, replying_to: Option<&str>) -> Result<String> {
    let mut prompt = format!("Instruction: {instruction}\n\n<draft>\n{draft}\n</draft>\n");
    if let Some(orig) = replying_to {
        prompt.push_str(&format!("\n<original_email>\n{orig}\n</original_email>\n"));
    }
    complete(cfg, WRITE_SYSTEM, prompt, Effort::Medium).await
}

pub async fn summarize(cfg: &AiConfig, email: &str) -> Result<String> {
    complete(
        cfg,
        "Summarize the email in 2-4 short bullet points in the email's language. Mention any requested \
         actions and deadlines. Output only the bullets.",
        email.to_string(),
        Effort::Low,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One-shot HTTP server returning `body`; hands back the raw request.
    async fn mock(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", l.local_addr().unwrap());
        let h = tokio::spawn(async move {
            let (mut sock, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            let mut req = String::new();
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                req.push_str(&String::from_utf8_lossy(&buf[..n]));
                if let Some(i) = req.find("\r\n\r\n") {
                    let len: usize = req
                        .lines()
                        .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                        .unwrap_or(0);
                    if req.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            req
        });
        (url, h)
    }

    #[tokio::test]
    async fn openai_compatible_roundtrip() {
        let (url, h) = mock(r#"{"choices":[{"message":{"role":"assistant","content":"<think>x</think>(invoice OR rechnung) hotel"}}]}"#).await;
        let cfg = AiConfig { enabled: true, provider: AiProvider::Ollama, model: "llama3.2".into(), base_url: url };
        let q = search_query(&cfg, "hotel bill").await.unwrap();
        assert_eq!(q, "(invoice OR rechnung) hotel");
        let req = h.await.unwrap();
        assert!(req.starts_with("POST /v1/chat/completions"), "{req}");
        assert!(req.contains("\"model\":\"llama3.2\""));
        assert!(req.contains("\"role\":\"system\""));

        let (url, h) = mock(r#"{"object":"list","data":[{"id":"qwen3"},{"id":"llama3.2"}]}"#).await;
        let cfg = AiConfig { base_url: url, ..cfg };
        assert_eq!(list_models(&cfg).await.unwrap(), vec!["llama3.2", "qwen3"]);
        assert!(h.await.unwrap().starts_with("GET /v1/models"));
    }

    #[test]
    fn strips_reasoning() {
        assert_eq!(super::strip_think("<think>hmm</think>\nHello"), "\nHello");
        assert_eq!(super::strip_think("Hello"), "Hello");
    }
}
