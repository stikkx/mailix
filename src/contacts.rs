//! Address book sources: LDAP directories and Google Contacts (People API).
//! Each source is pulled into the local `contacts` table, which powers
//! autocomplete and the address book view.

use crate::config::{GoogleConfig, LdapConfig};
use crate::secrets;
use crate::store::Contact;
use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::Deserialize;
use sha2::Digest;

// ---- LDAP ----------------------------------------------------------------

pub async fn sync_ldap(cfg: &LdapConfig) -> Result<Vec<Contact>> {
    use ldap3::{LdapConnAsync, Scope, SearchEntry};

    let (conn, mut ldap) = LdapConnAsync::new(&cfg.url).await.context("LDAP connect")?;
    ldap3::drive!(conn);
    if !cfg.bind_dn.is_empty() {
        let pw = secrets::get(&secrets::ldap_key(&cfg.name))?.unwrap_or_default();
        ldap.simple_bind(&cfg.bind_dn, &pw).await?.success().context("LDAP bind")?;
    }
    let attrs = vec!["cn", "displayName", "mail", "telephoneNumber", "mobile", "o", "company"];
    let (entries, _res) = ldap
        .search(&cfg.base_dn, Scope::Subtree, &cfg.filter, attrs)
        .await?
        .success()
        .context("LDAP search")?;
    let mut out = Vec::new();
    for e in entries {
        let e = SearchEntry::construct(e);
        let first = |k: &str| e.attrs.get(k).and_then(|v| v.first()).cloned().unwrap_or_default();
        let name = Some(first("displayName")).filter(|s| !s.is_empty()).unwrap_or_else(|| first("cn"));
        let phone = Some(first("telephoneNumber")).filter(|s| !s.is_empty()).unwrap_or_else(|| first("mobile"));
        let org = Some(first("o")).filter(|s| !s.is_empty()).unwrap_or_else(|| first("company"));
        for mail in e.attrs.get("mail").into_iter().flatten() {
            out.push(Contact {
                source: format!("ldap:{}", cfg.name),
                name: name.clone(),
                email: mail.clone(),
                phone: phone.clone(),
                org: org.clone(),
            });
        }
    }
    ldap.unbind().await.ok();
    Ok(out)
}

// ---- Google Contacts -----------------------------------------------------

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/contacts.readonly";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
}

/// Interactive OAuth (installed-app loopback flow with PKCE). Opens the
/// browser, waits for the redirect on 127.0.0.1, stores the refresh token.
pub async fn google_authorize(cfg: &GoogleConfig) -> Result<()> {
    use rand::Rng;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier: String = {
        let bytes: [u8; 32] = rand::rng().random();
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    };
    let challenge =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    let state = crate::config::new_id();
    let url = url::Url::parse_with_params(
        AUTH_URL,
        &[
            ("client_id", cfg.client_id.as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", &state),
        ],
    )?;
    open::that(url.as_str()).context("could not open browser")?;

    let (mut sock, _) = tokio::time::timeout(std::time::Duration::from_secs(300), listener.accept())
        .await
        .context("timed out waiting for Google sign-in")??;
    let mut buf = vec![0u8; 8192];
    let n = sock.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.split_whitespace().nth(1).unwrap_or("/");
    let cb = url::Url::parse(&format!("http://localhost{path}"))?;
    let param = |k: &str| cb.query_pairs().find(|(q, _)| q == k).map(|(_, v)| v.into_owned());
    let body = "<html><body style='font-family:sans-serif'>Mailix is connected. You can close this tab.</body></html>";
    sock.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}", body.len())
            .as_bytes(),
    )
    .await
    .ok();
    if param("state").as_deref() != Some(&state) {
        bail!("OAuth state mismatch");
    }
    let code = param("code").context(param("error").unwrap_or_else(|| "no code returned".into()))?;

    let tok: TokenResponse = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("code", &code),
            ("code_verifier", &verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", &redirect),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let refresh = tok.refresh_token.context("Google returned no refresh token")?;
    secrets::set(&secrets::google_key(&cfg.name), &refresh)?;
    Ok(())
}

async fn google_access_token(cfg: &GoogleConfig) -> Result<String> {
    let refresh = secrets::get(&secrets::google_key(&cfg.name))?
        .context("not connected to Google yet — click Connect in Settings")?;
    let tok: TokenResponse = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("refresh_token", &refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(tok.access_token)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Connections {
    #[serde(default)]
    connections: Vec<Person>,
    next_page_token: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Person {
    #[serde(default)]
    names: Vec<Named>,
    #[serde(default)]
    email_addresses: Vec<Valued>,
    #[serde(default)]
    phone_numbers: Vec<Valued>,
    #[serde(default)]
    organizations: Vec<Named>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Named {
    display_name: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize, Default)]
struct Valued {
    value: String,
}

pub async fn sync_google(cfg: &GoogleConfig) -> Result<Vec<Contact>> {
    let token = google_access_token(cfg).await?;
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut page: Option<String> = None;
    loop {
        let mut req = client
            .get("https://people.googleapis.com/v1/people/me/connections")
            .bearer_auth(&token)
            .query(&[("personFields", "names,emailAddresses,phoneNumbers,organizations"), ("pageSize", "1000")]);
        if let Some(p) = &page {
            req = req.query(&[("pageToken", p)]);
        }
        let resp: Connections = req.send().await?.error_for_status()?.json().await?;
        for p in resp.connections {
            let name = p.names.first().and_then(|n| n.display_name.clone()).unwrap_or_default();
            let phone = p.phone_numbers.first().map(|v| v.value.clone()).unwrap_or_default();
            let org = p.organizations.first().and_then(|o| o.name.clone()).unwrap_or_default();
            for e in p.email_addresses {
                out.push(Contact {
                    source: format!("google:{}", cfg.name),
                    name: name.clone(),
                    email: e.value,
                    phone: phone.clone(),
                    org: org.clone(),
                });
            }
        }
        match resp.next_page_token {
            Some(t) => page = Some(t),
            None => break,
        }
    }
    Ok(out)
}
