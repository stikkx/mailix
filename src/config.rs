//! User configuration (TOML) and platform paths.
//!
//! Passwords and tokens are never written here; they live in the OS keyring
//! (see [`crate::secrets`]).

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Implicit TLS (IMAPS 993 / SMTPS 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143 / SMTP 587, Proton Bridge).
    StartTls,
    /// No encryption. Only sensible for localhost bridges.
    Plain,
}

impl Security {
    pub const ALL: [Security; 3] = [Security::Tls, Security::StartTls, Security::Plain];
}

impl std::fmt::Display for Security {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Security::Tls => "TLS",
            Security::StartTls => "STARTTLS",
            Security::Plain => "None",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    pub username: String,
    /// Needed for Proton Bridge, which uses a self-signed certificate.
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    /// Plain-text signature (also the text alternative of an HTML signature).
    pub body: String,
    /// Imported HTML signature; images are embedded as `data:` URIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountConfig {
    /// Stable identifier, used as key in the database and keyring.
    pub id: String,
    /// Label shown in the sidebar.
    pub name: String,
    pub display_name: String,
    pub email: String,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    #[serde(default)]
    pub signatures: Vec<Signature>,
    /// Name of the signature preselected when composing.
    #[serde(default)]
    pub default_signature: Option<String>,
    /// Append a copy of sent mail to the Sent folder. Gmail and Proton do
    /// this server-side, so presets turn it off for them.
    #[serde(default = "yes")]
    pub save_sent_copy: bool,
    /// How many of the newest messages per folder to fetch on first sync.
    #[serde(default = "initial_sync_limit")]
    pub initial_sync_limit: u32,
}

fn yes() -> bool {
    true
}
fn initial_sync_limit() -> u32 {
    2000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Generic,
    ProtonBridge,
    Gmail,
    Outlook,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Generic, Preset::ProtonBridge, Preset::Gmail, Preset::Outlook];
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Preset::Generic => "Generic IMAP",
            Preset::ProtonBridge => "Proton Mail (Bridge)",
            Preset::Gmail => "Gmail (app password)",
            Preset::Outlook => "Outlook / Microsoft 365",
        })
    }
}

impl AccountConfig {
    pub fn from_preset(preset: Preset, email: &str) -> Self {
        let domain = email.split('@').nth(1).unwrap_or("example.com").to_string();
        let server = |host: &str, port, security, invalid| ServerConfig {
            host: host.into(),
            port,
            security,
            username: email.into(),
            accept_invalid_certs: invalid,
        };
        let (imap, smtp, save_sent) = match preset {
            Preset::Generic => (
                server(&format!("imap.{domain}"), 993, Security::Tls, false),
                server(&format!("smtp.{domain}"), 465, Security::Tls, false),
                true,
            ),
            Preset::ProtonBridge => (
                server("127.0.0.1", 1143, Security::StartTls, true),
                server("127.0.0.1", 1025, Security::StartTls, true),
                false,
            ),
            Preset::Gmail => (
                server("imap.gmail.com", 993, Security::Tls, false),
                server("smtp.gmail.com", 465, Security::Tls, false),
                false,
            ),
            Preset::Outlook => (
                server("outlook.office365.com", 993, Security::Tls, false),
                server("smtp.office365.com", 587, Security::StartTls, false),
                false,
            ),
        };
        AccountConfig {
            id: new_id(),
            name: email.into(),
            display_name: String::new(),
            email: email.into(),
            imap,
            smtp,
            signatures: Vec::new(),
            default_signature: None,
            save_sent_copy: save_sent,
            initial_sync_limit: initial_sync_limit(),
        }
    }

    pub fn signature(&self, name: Option<&str>) -> Option<&Signature> {
        name.and_then(|n| self.signatures.iter().find(|s| s.name == n))
    }
}

pub fn new_id() -> String {
    use rand::Rng;
    let n: u64 = rand::rng().random();
    format!("{n:016x}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LdapConfig {
    pub name: String,
    /// e.g. `ldaps://ldap.example.com:636` or `ldap://host:389`
    pub url: String,
    /// Empty for anonymous bind. Password is kept in the keyring.
    #[serde(default)]
    pub bind_dn: String,
    pub base_dn: String,
    #[serde(default = "ldap_filter")]
    pub filter: String,
}

pub fn ldap_filter() -> String {
    "(&(mail=*)(|(objectClass=person)(objectClass=inetOrgPerson)))".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GoogleConfig {
    pub name: String,
    /// OAuth client of type "Desktop app" from Google Cloud Console.
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContactsConfig {
    #[serde(default)]
    pub ldap: Vec<LdapConfig>,
    #[serde(default)]
    pub google: Vec<GoogleConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AiProvider {
    #[default]
    Anthropic,
    OpenAi,
    Ollama,
    OllamaCloud,
    OpencodeZen,
    OpenRouter,
    /// Any other OpenAI-compatible endpoint (LM Studio, vLLM, LiteLLM, …).
    Custom,
}

impl AiProvider {
    pub const ALL: [AiProvider; 7] = [
        AiProvider::Anthropic,
        AiProvider::OpenAi,
        AiProvider::Ollama,
        AiProvider::OllamaCloud,
        AiProvider::OpencodeZen,
        AiProvider::OpenRouter,
        AiProvider::Custom,
    ];

    pub fn default_base_url(self) -> &'static str {
        match self {
            AiProvider::Anthropic => "https://api.anthropic.com",
            AiProvider::OpenAi => "https://api.openai.com/v1",
            AiProvider::Ollama => "http://localhost:11434/v1",
            AiProvider::OllamaCloud => "https://ollama.com/v1",
            AiProvider::OpencodeZen => "https://opencode.ai/zen/v1",
            AiProvider::OpenRouter => "https://openrouter.ai/api/v1",
            AiProvider::Custom => "http://localhost:8080/v1",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            AiProvider::Anthropic => "claude-opus-5-5",
            AiProvider::OpenAi => "gpt-5",
            AiProvider::Ollama => "llama3.2",
            AiProvider::OllamaCloud => "gpt-oss:120b",
            AiProvider::OpencodeZen => "qwen3-coder",
            AiProvider::OpenRouter => "anthropic/claude-opus-5-5",
            AiProvider::Custom => "",
        }
    }

    /// Local Ollama works without a key.
    pub fn needs_key(self) -> bool {
        !matches!(self, AiProvider::Ollama | AiProvider::Custom)
    }

    pub fn key_name(self) -> String {
        format!("ai:{}:api_key", serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default())
    }
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AiProvider::Anthropic => "Anthropic (Claude)",
            AiProvider::OpenAi => "OpenAI",
            AiProvider::Ollama => "Ollama (local)",
            AiProvider::OllamaCloud => "Ollama Cloud",
            AiProvider::OpencodeZen => "OpenCode Zen",
            AiProvider::OpenRouter => "OpenRouter",
            AiProvider::Custom => "Custom (OpenAI-compatible)",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiConfig {
    pub enabled: bool,
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default = "ai_model")]
    pub model: String,
    #[serde(default = "ai_base")]
    pub base_url: String,
}

fn ai_model() -> String {
    AiProvider::Anthropic.default_model().into()
}
fn ai_base() -> String {
    AiProvider::Anthropic.default_base_url().into()
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig { enabled: false, provider: AiProvider::Anthropic, model: ai_model(), base_url: ai_base() }
    }
}

/// What the sidebar shows; one button cycles through these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarMode {
    /// Account headers only.
    Collapsed,
    /// Inbox, Sent and favorite folders.
    Favorites,
    /// Every folder.
    #[default]
    All,
}

impl SidebarMode {
    pub fn next(self) -> Self {
        match self {
            SidebarMode::Collapsed => SidebarMode::Favorites,
            SidebarMode::Favorites => SidebarMode::All,
            SidebarMode::All => SidebarMode::Collapsed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrammarLang {
    /// German or English, picked per message.
    #[default]
    Auto,
    German,
    English,
}

impl GrammarLang {
    pub const ALL: [GrammarLang; 3] = [GrammarLang::Auto, GrammarLang::German, GrammarLang::English];
}

impl std::fmt::Display for GrammarLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GrammarLang::Auto => "Automatic (German / English)",
            GrammarLang::German => "Deutsch",
            GrammarLang::English => "English",
        })
    }
}

pub const LANGUAGETOOL_LOCAL: &str = "http://localhost:8010";
pub const LANGUAGETOOL_PUBLIC: &str = "https://api.languagetool.org";

/// Grammar and spelling check (LanguageTool).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrammarConfig {
    #[serde(default)]
    pub enabled: bool,
    /// LanguageTool server; the public one gets the text of your mail.
    #[serde(default = "lt_server")]
    pub server: String,
    #[serde(default)]
    pub language: GrammarLang,
    /// British instead of American English.
    #[serde(default)]
    pub english_gb: bool,
    /// Words never flagged (names, product terms).
    #[serde(default)]
    pub ignored: Vec<String>,
}

fn lt_server() -> String {
    LANGUAGETOOL_LOCAL.into()
}

impl Default for GrammarConfig {
    fn default() -> Self {
        GrammarConfig {
            enabled: false,
            server: lt_server(),
            language: GrammarLang::Auto,
            english_gb: false,
            ignored: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    /// Primary / accent colour as `#rrggbb`.
    #[serde(default = "accent")]
    pub accent: String,
}

fn accent() -> String {
    "#2f6bed".into()
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance { accent: accent() }
    }
}

/// `#rrggbb` → (r, g, b)
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((p(0)?, p(2)?, p(4)?))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
    #[serde(default)]
    pub contacts: ContactsConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub grammar: GrammarConfig,
    /// Sidebar sections the user collapsed: account ids, and
    /// `account id + "\u{1f}" + folder` for parent folders.
    #[serde(default)]
    pub collapsed: Vec<String>,
    /// Favorite folders (`account id + "\u{1f}" + folder`).
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default)]
    pub sidebar_mode: SidebarMode,
    /// Group the message list into conversations.
    #[serde(default = "yes")]
    pub threads: bool,
    /// Desktop notification for new inbox mail.
    #[serde(default = "yes")]
    pub notifications: bool,
    /// Background sync interval in minutes (0 = manual only).
    #[serde(default = "sync_interval")]
    pub sync_interval_minutes: u64,
}

fn sync_interval() -> u64 {
    5
}

impl Default for Config {
    fn default() -> Self {
        Config {
            accounts: Vec::new(),
            contacts: ContactsConfig::default(),
            ai: AiConfig::default(),
            appearance: Appearance::default(),
            grammar: GrammarConfig::default(),
            collapsed: Vec::new(),
            favorites: Vec::new(),
            sidebar_mode: SidebarMode::default(),
            threads: true,
            notifications: true,
            sync_interval_minutes: sync_interval(),
        }
    }
}

pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
}

/// Moves config, data and cache from the old project name ("mailixx") to
/// the new one, once. Leaves the old folders alone if the new ones exist.
pub fn migrate_old_name() {
    let (Some(old), Some(new)) =
        (ProjectDirs::from("de", "qwellcode", "mailixx"), ProjectDirs::from("de", "qwellcode", "mailix"))
    else {
        return;
    };
    for (from, to) in [
        (old.config_dir(), new.config_dir()),
        (old.data_dir(), new.data_dir()),
        (old.cache_dir(), new.cache_dir()),
    ] {
        if from == to || !from.exists() || to.exists() {
            continue;
        }
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(from, to) {
            Ok(()) => tracing::info!("moved {} to {}", from.display(), to.display()),
            Err(e) => tracing::warn!("could not move {} to {}: {e}", from.display(), to.display()),
        }
    }
}

pub fn paths() -> Result<Paths> {
    let dirs = ProjectDirs::from("de", "qwellcode", "mailix").context("no home directory")?;
    Ok(Paths {
        config_file: dirs.config_dir().join("config.toml"),
        data_dir: dirs.data_dir().to_path_buf(),
    })
}

impl Config {
    pub fn load() -> Result<Self> {
        let p = paths()?;
        match std::fs::read_to_string(&p.config_file) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", p.config_file.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let p = paths()?;
        if let Some(dir) = p.config_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = p.config_file.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, &p.config_file)?;
        Ok(())
    }

    pub fn account(&self, id: &str) -> Option<&AccountConfig> {
        self.accounts.iter().find(|a| a.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_old_config_compat() {
        assert_eq!(parse_hex("#2f6bed"), Some((0x2f, 0x6b, 0xed)));
        assert_eq!(parse_hex("2F6BED"), Some((0x2f, 0x6b, 0xed)));
        assert_eq!(parse_hex("#xyz"), None);
        // Configs written by the first version still load.
        let c: Config = toml::from_str("[ai]\nenabled = true\nmodel = \"claude-opus-5-5\"\n").unwrap();
        assert_eq!(c.ai.provider, AiProvider::Anthropic);
        assert!(c.notifications);
        assert_eq!(AiProvider::OllamaCloud.key_name(), "ai:ollama-cloud:api_key");
    }
}

#[cfg(test)]
mod rename_tests {
    /// Run alone (changes XDG env): `cargo test migrates_old_folders -- --ignored`
    #[test]
    #[ignore]
    fn migrates_old_folders() {
        let root = std::env::temp_dir().join(format!("mailix-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (var, dir) in [("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_CACHE_HOME", "cache")] {
            // SAFETY: test is run on its own; nothing else reads the env concurrently.
            unsafe { std::env::set_var(var, root.join(dir)) };
        }
        std::fs::create_dir_all(root.join("config/mailixx")).unwrap();
        std::fs::write(root.join("config/mailixx/config.toml"), "threads = false\n").unwrap();
        std::fs::create_dir_all(root.join("data/mailixx")).unwrap();
        std::fs::write(root.join("data/mailixx/mail.db"), b"db").unwrap();
        super::migrate_old_name();
        assert!(root.join("config/mailix/config.toml").exists());
        assert!(root.join("data/mailix/mail.db").exists());
        assert!(!root.join("config/mailixx").exists());
        assert!(!super::Config::load().unwrap().threads);
        // Second run: nothing to do, nothing breaks.
        super::migrate_old_name();
        assert!(root.join("config/mailix/config.toml").exists());
        std::fs::remove_dir_all(&root).ok();
    }
}
