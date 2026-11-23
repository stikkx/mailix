//! IMAP: connecting (TLS / STARTTLS / plain), syncing folders into the
//! local store, and pushing flag changes / moves back to the server.

use crate::config::{AccountConfig, Security};
use crate::net::{self, BoxStream};
use crate::search::{Doc, Search};
use crate::store::{FolderRole, FolderState, NewMessage, Store};
use anyhow::{Context, Result, anyhow};
use async_imap::Session;
use async_imap::types::{Fetch, Flag, NameAttribute};
use futures::TryStreamExt;

pub type ImapSession = Session<BoxStream>;

pub async fn connect(acc: &AccountConfig, password: &str) -> Result<ImapSession> {
    let s = &acc.imap;
    let tcp = net::tcp(&s.host, s.port).await?;
    let stream: BoxStream = match s.security {
        Security::Tls => Box::new(net::wrap_tls(tcp, &s.host, s.accept_invalid_certs).await?),
        Security::Plain => Box::new(tcp),
        Security::StartTls => {
            let mut client = async_imap::Client::new(tcp);
            client.read_response().await?.context("server closed connection")?;
            client.run_command_and_check_ok("STARTTLS", None).await.context("STARTTLS refused")?;
            let tcp = client.into_inner();
            Box::new(net::wrap_tls(tcp, &s.host, s.accept_invalid_certs).await?)
        }
    };
    let mut client = async_imap::Client::new(stream);
    if s.security != Security::StartTls {
        client.read_response().await?.context("server closed connection")?;
    }
    client
        .login(&s.username, password)
        .await
        .map_err(|(e, _)| anyhow!("IMAP login failed: {e}"))
}

/// Decodes IMAP "modified UTF-7" folder names (RFC 3501 §5.1.3):
/// `Entw&APw-rfe` → `Entwürfe`, `&-` → `&`. Invalid input is returned as-is.
pub fn decode_folder_name(name: &str) -> String {
    use base64::Engine;
    if !name.contains('&') {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            return name.to_string();
        };
        let chunk = &after[..end];
        if chunk.is_empty() {
            out.push('&');
        } else {
            let b64 = chunk.replace(',', "/");
            let Ok(bytes) = base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.trim_end_matches('=')) else {
                return name.to_string();
            };
            if bytes.len() % 2 != 0 {
                return name.to_string();
            }
            let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            match String::from_utf16(&units) {
                Ok(s) => out.push_str(&s),
                Err(_) => return name.to_string(),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn attr_role(attrs: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Junk => Some(FolderRole::Junk),
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::All => Some(FolderRole::All),
        _ => None,
    })
}

/// Guess from a folder's own name: (role, preference; lower wins).
fn guess_role(leaf: &str) -> Option<(FolderRole, u8)> {
    const NAMES: &[(FolderRole, &[&str])] = &[
        (FolderRole::Sent, &["sent", "sent messages", "sent items", "sent mail", "gesendet", "gesendete elemente", "gesendete objekte"]),
        (FolderRole::Drafts, &["drafts", "draft", "entwürfe"]),
        (FolderRole::Trash, &["trash", "deleted items", "deleted messages", "papierkorb", "gelöschte elemente", "gelöschte objekte"]),
        (FolderRole::Junk, &["junk", "spam", "junk e-mail", "junk-e-mail", "junk-mail"]),
        (FolderRole::Archive, &["archive", "archives", "archiv"]),
    ];
    let l = leaf.to_lowercase();
    NAMES.iter().find_map(|(role, names)| names.iter().position(|n| *n == l).map(|i| (*role, i as u8)))
}

/// Assigns at most one folder per special role. Server SPECIAL-USE flags
/// win; otherwise names are guessed, but only for top-level folders or
/// direct children of INBOX (so "Archives/Sent Messages" stays a normal
/// folder), and only the best-named candidate gets each role.
fn assign_roles(folders: &[(String, Option<char>, Option<FolderRole>)]) -> Vec<FolderRole> {
    let mut roles: Vec<FolderRole> = folders
        .iter()
        .map(|(name, _, attr)| {
            if name.eq_ignore_ascii_case("INBOX") { FolderRole::Inbox } else { attr.unwrap_or(FolderRole::Other) }
        })
        .collect();
    let flagged: std::collections::HashSet<FolderRole> =
        roles.iter().copied().filter(|r| *r != FolderRole::Other).collect();
    let mut best: std::collections::HashMap<FolderRole, (u8, usize, usize)> = Default::default();
    for (i, (name, delim, attr)) in folders.iter().enumerate() {
        if attr.is_some() || roles[i] != FolderRole::Other {
            continue;
        }
        let decoded = decode_folder_name(name);
        let (parent, leaf) = match delim.and_then(|d| decoded.rsplit_once(d)) {
            Some((p, l)) => (Some(p), l),
            None => (None, decoded.as_str()),
        };
        if parent.is_some_and(|p| !p.eq_ignore_ascii_case("INBOX")) {
            continue;
        }
        let Some((role, pref)) = guess_role(leaf) else { continue };
        if flagged.contains(&role) {
            continue;
        }
        let cand = (pref, decoded.len(), i);
        if best.get(&role).is_none_or(|b| cand < *b) {
            best.insert(role, cand);
        }
    }
    for (role, (_, _, i)) in best {
        roles[i] = role;
    }
    roles
}

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub new_messages: usize,
    pub removed: usize,
    /// Unread mail that arrived in the inbox since the last sync
    /// (sender, subject) — empty on the very first sync.
    pub new_inbox: Vec<(String, String)>,
}

impl SyncReport {
    fn absorb(&mut self, r: SyncReport) {
        self.new_messages += r.new_messages;
        self.removed += r.removed;
        self.new_inbox.extend(r.new_inbox);
    }
}

/// Full sync of one account: folder list, new mail, flag changes, deletions.
pub async fn sync_account(acc: &AccountConfig, password: &str, store: &Store, search: &Search) -> Result<SyncReport> {
    let mut session = connect(acc, password).await?;
    let mut report = SyncReport::default();

    let names: Vec<(String, FolderRole, bool)> = {
        let list = session.list(Some(""), Some("*")).await?;
        let names: Vec<_> = list.try_collect().await?;
        let info: Vec<(String, Option<char>, Option<FolderRole>)> = names
            .iter()
            .map(|n| (n.name().to_string(), n.delimiter().and_then(|d| d.chars().next()), attr_role(n.attributes())))
            .collect();
        let roles = assign_roles(&info);
        names
            .iter()
            .zip(roles)
            .map(|(n, role)| {
                let noselect = n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect));
                (n.name().to_string(), role, noselect)
            })
            .collect()
    };

    let selectable: Vec<String> = names.iter().filter(|(_, _, ns)| !ns).map(|(n, _, _)| n.clone()).collect();
    for (name, role, noselect) in &names {
        if !noselect {
            store.upsert_folder(&acc.id, name, *role)?;
        }
    }
    store.retain_folders(&acc.id, &selectable)?;

    // Inbox first so new mail shows up as early as possible. Skip Gmail's
    // "All Mail" — it duplicates every other folder.
    let mut order: Vec<&(String, FolderRole, bool)> =
        names.iter().filter(|(_, r, ns)| !ns && *r != FolderRole::All).collect();
    order.sort_by_key(|(_, r, _)| r.rank());

    for (name, _, _) in order {
        match sync_folder(&mut session, acc, name, store, search).await {
            Ok(r) => report.absorb(r),
            Err(e) => tracing::warn!("sync {}/{name}: {e:#}", acc.name),
        }
    }
    // Tag cached mail that carries the AgentProcessed header (from any era).
    if let Err(e) = store.backfill_agent_tags() {
        tracing::warn!("sync {}: agent tag backfill: {e:#}", acc.name);
    }
    search.commit()?;
    session.logout().await.ok();
    Ok(report)
}

/// Quick sync of the inbox only — used when IDLE reports a change.
pub async fn sync_inbox(acc: &AccountConfig, password: &str, store: &Store, search: &Search) -> Result<SyncReport> {
    let inbox = store.folder_by_role(&acc.id, FolderRole::Inbox)?.unwrap_or_else(|| "INBOX".into());
    store.upsert_folder(&acc.id, &inbox, FolderRole::Inbox)?;
    let mut session = connect(acc, password).await?;
    let r = sync_folder(&mut session, acc, &inbox, store, search).await?;
    search.commit()?;
    session.logout().await.ok();
    Ok(r)
}

/// Keeps an IDLE connection on INBOX open and calls `on_change` whenever the
/// server reports new mail, expunges or flag changes. Returns on error; the
/// caller reconnects.
pub async fn watch_inbox(acc: &AccountConfig, password: &str, on_change: &mut (dyn FnMut() + Send)) -> Result<()> {
    use async_imap::extensions::idle::IdleResponse;
    let mut session = connect(acc, password).await?;
    if !session.capabilities().await?.has_str("IDLE") {
        anyhow::bail!("server does not support IDLE");
    }
    session.select("INBOX").await?;
    // Connected: catch up on anything that arrived while we were offline.
    on_change();
    loop {
        let mut idle = session.idle();
        idle.init().await?;
        // Re-issue IDLE before the 29-minute server timeout (RFC 2177).
        let resp = {
            let (wait, _stop) = idle.wait_with_timeout(std::time::Duration::from_secs(25 * 60));
            wait.await?
        };
        session = idle.done().await?;
        if let IdleResponse::NewData(_) = resp {
            on_change();
        }
    }
}

/// Downloads the full source of one message (for messages cached before
/// raw storage existed, or if the cache was trimmed).
pub async fn fetch_raw(acc: &AccountConfig, password: &str, folder: &str, uid: u32) -> Result<Vec<u8>> {
    let mut s = connect(acc, password).await?;
    s.examine(folder).await?;
    let raw = {
        let stream = s.uid_fetch(uid.to_string(), "BODY.PEEK[]").await?;
        let fetches: Vec<Fetch> = stream.try_collect().await?;
        fetches.iter().find_map(|f| f.body().map(<[u8]>::to_vec)).context("message no longer on server")?
    };
    s.logout().await.ok();
    Ok(raw)
}

const BATCH: usize = 100;

async fn sync_folder(
    session: &mut ImapSession,
    acc: &AccountConfig,
    folder: &str,
    store: &Store,
    search: &Search,
) -> Result<SyncReport> {
    let mbox = session.select(folder).await?;
    let is_inbox = store.folder_by_role(&acc.id, FolderRole::Inbox)?.as_deref() == Some(folder);
    let uidvalidity = mbox.uid_validity.unwrap_or(0);
    let mut state = store.folder_state(&acc.id, folder)?;
    let mut report = SyncReport::default();
    // Never synced (or UIDVALIDITY reset): don't notify about old mail.
    let first_sync = state.uidvalidity != uidvalidity;

    if state.uidvalidity != uidvalidity {
        let removed = store.reset_folder(&acc.id, folder)?;
        search.remove(&removed);
        state = FolderState { uidvalidity, last_uid: 0 };
    }

    if mbox.exists == 0 {
        let removed = store.reconcile_flags(&acc.id, folder, &[])?;
        search.remove(&removed);
        store.set_folder_state(&acc.id, folder, &state)?;
        return Ok(report);
    }

    // 1. Flags + deletions for what we already have.
    if state.last_uid > 0 {
        let mut flags = Vec::new();
        {
            let stream = session.uid_fetch(format!("1:{}", state.last_uid), "(UID FLAGS)").await?;
            let fetches: Vec<Fetch> = stream.try_collect().await?;
            for f in &fetches {
                if let Some(uid) = f.uid {
                    let (seen, flagged, tags, answered) = flag_bits(f);
                    flags.push((uid, seen, flagged, tags, answered));
                }
            }
        }
        let removed = store.reconcile_flags(&acc.id, folder, &flags)?;
        report.removed += removed.len();
        search.remove(&removed);
    }

    // 2. New UIDs. On first sync only take the newest N.
    let uids: Vec<u32> = {
        let query = if state.last_uid == 0 { "ALL".to_string() } else { format!("UID {}:*", state.last_uid + 1) };
        let mut v: Vec<u32> = session.uid_search(&query).await?.into_iter().filter(|u| *u > state.last_uid).collect();
        v.sort_unstable();
        if state.last_uid == 0 && v.len() > acc.initial_sync_limit as usize {
            v.drain(..v.len() - acc.initial_sync_limit as usize);
        }
        v
    };

    // Newest first, so the top of the list fills in immediately.
    for chunk in uids.rchunks(BATCH) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let fetches: Vec<Fetch> = {
            let stream = session.uid_fetch(&set, "(UID FLAGS INTERNALDATE BODY.PEEK[])").await?;
            stream.try_collect().await?
        };
        let parsed: Vec<NewMessage> = fetches.iter().filter_map(parse_fetch).collect();
        let added = store.insert_messages(&acc.id, folder, parsed)?;
        if is_inbox && !first_sync {
            for (_, m) in added.iter().filter(|(_, m)| !m.seen) {
                let who = if m.from_name.is_empty() { m.from_addr.clone() } else { m.from_name.clone() };
                report.new_inbox.push((who, m.subject.clone()));
            }
        }
        let docs: Vec<Doc<'_>> = added
            .iter()
            .map(|(id, m)| Doc {
                id: *id,
                account: &acc.id,
                subject: &m.subject,
                from: &m.from_addr,
                to: &m.to,
                body: &m.body_text,
            })
            .collect();
        // Index sender name and address together.
        let from_full: Vec<String> = added.iter().map(|(_, m)| format!("{} {}", m.from_name, m.from_addr)).collect();
        let docs: Vec<Doc<'_>> = docs.into_iter().zip(&from_full).map(|(d, f)| Doc { from: f, ..d }).collect();
        search.add(&docs)?;
        report.new_messages += added.len();
    }
    if let Some(max) = uids.last() {
        state.last_uid = state.last_uid.max(*max);
    }
    store.set_folder_state(&acc.id, folder, &state)?;
    if report.new_messages > 0 {
        search.commit()?;
    }
    Ok(report)
}

fn flag_bits(f: &Fetch) -> (bool, bool, u8, bool) {
    let mut seen = false;
    let mut flagged = false;
    let mut answered = false;
    let mut tags = 0u8;
    for fl in f.flags() {
        match fl {
            Flag::Seen => seen = true,
            Flag::Flagged => flagged = true,
            Flag::Answered => answered = true,
            Flag::Custom(k) if k.as_ref() == crate::store::KEYWORD_TODO => tags |= crate::store::TAG_TODO,
            Flag::Custom(k) if k.as_ref() == crate::store::KEYWORD_IMPORTANT => tags |= crate::store::TAG_IMPORTANT,
            _ => {}
        }
    }
    (seen, flagged, tags, answered)
}

fn parse_fetch(f: &Fetch) -> Option<NewMessage> {
    let uid = f.uid?;
    let raw = f.body()?;
    let (seen, flagged, tag_bits, answered) = flag_bits(f);
    let fallback_date = f.internal_date().map(|d| d.timestamp()).unwrap_or(0);
    let mut m = parse_raw(raw)?;
    m.raw = raw.to_vec();
    m.uid = uid;
    m.seen = seen;
    m.flagged = flagged;
    m.answered = answered;
    m.tags |= tag_bits;
    if m.date == 0 {
        m.date = fallback_date;
    }
    Some(m)
}

/// Parses an RFC 5322 message into what we store and index.
pub fn parse_raw(raw: &[u8]) -> Option<NewMessage> {
    let msg = mail_parser::MessageParser::default().parse(raw)?;
    let addr_list = |l: Option<mail_parser::AddressList<'_>>| -> String {
        l.map(|l| {
            l.mailboxes()
                .filter_map(|m| {
                    let a = m.address()?;
                    Some(match m.name() {
                        Some(n) if !n.is_empty() => format!("{n} <{a}>"),
                        _ => a.to_string(),
                    })
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
    };
    let (from_name, from_addr) = msg
        .from()
        .and_then(|l| l.first())
        .map(|m| (m.name().unwrap_or_default().to_string(), m.address().unwrap_or_default().to_lowercase()))
        .unwrap_or_default();

    let html = msg.body_html(0);
    let body_text = match msg.body_text(0) {
        Some(t) if !t.trim().is_empty() => t.into_owned(),
        _ => html.as_deref().map(html_to_text).unwrap_or_default(),
    };
    let preview: String = body_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('>'))
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect();
    let attachments = msg
        .attachments()
        .filter(|p| {
            // Images embedded in the HTML via cid: aren't "attachments" to the user.
            let embedded = p.is_inline()
                && p.content_id().is_some_and(|cid| {
                    html.as_deref().is_some_and(|h| h.contains(&format!("cid:{}", cid.trim_matches(['<', '>']))))
                });
            !embedded
        })
        .map(|p| crate::mime::sanitize_name(p.attachment_name().unwrap_or("attachment")))
        .collect();
    // References chain; fall back to In-Reply-To (conversation grouping).
    let references = msg
        .references()
        .filter(|r| r.iter().next().is_some())
        .or_else(|| msg.in_reply_to())
        .map(|r| r.iter().map(|s| format!("<{s}>")).collect::<Vec<_>>().join(" "));

    Some(NewMessage {
        uid: 0,
        message_id: msg.message_id().map(|s| format!("<{s}>")).unwrap_or_default(),
        references: references.unwrap_or_default(),
        subject: msg.subject().unwrap_or("(no subject)").to_string(),
        from_name,
        from_addr,
        to: addr_list(msg.to()),
        cc: addr_list(msg.cc()),
        reply_to: addr_list(msg.reply_to()),
        date: msg.date().map(|d| d.to_timestamp()).unwrap_or(0),
        seen: false,
        flagged: false,
        answered: false,
        tags: if msg
            .headers()
            .into_iter()
            .any(|h| {
                let n = h.name().as_str().replace('-', "").to_ascii_lowercase();
                n == "agentprocessed" || n == "xagentprocessed"
            })
        {
            crate::store::TAG_AGENT
        } else {
            0
        },
        attachments,
        preview,
        body_text,
        raw: Vec::new(),
    })
}

pub fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 100).unwrap_or_else(|_| html.to_string())
}

// ---- write-back operations ----------------------------------------------

pub async fn set_flag(acc: &AccountConfig, password: &str, folder: &str, uid: u32, flag: &str, on: bool) -> Result<()> {
    let mut s = connect(acc, password).await?;
    s.select(folder).await?;
    let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
    {
        let stream = s.uid_store(uid.to_string(), format!("{op} ({flag})")).await?;
        let _: Vec<Fetch> = stream.try_collect().await?;
    }
    s.logout().await.ok();
    Ok(())
}

/// Moves a message (to Trash / Archive). Uses MOVE when available, falls
/// back to COPY + \Deleted + EXPUNGE. `to = None` deletes permanently.
pub async fn move_message(acc: &AccountConfig, password: &str, folder: &str, uid: u32, to: Option<&str>) -> Result<()> {
    move_messages(acc, password, folder, &[uid], to).await
}

/// Moves several messages of one folder in one go.
pub async fn move_messages(acc: &AccountConfig, password: &str, folder: &str, uids: &[u32], to: Option<&str>) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let mut s = connect(acc, password).await?;
    s.select(folder).await?;
    let caps = s.capabilities().await?;
    match to {
        Some(dest) if caps.has_str("MOVE") => {
            s.uid_mv(&set, dest).await?;
        }
        other => {
            if let Some(dest) = other {
                s.uid_copy(&set, dest).await?;
            }
            {
                let stream = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?;
                let _: Vec<Fetch> = stream.try_collect().await?;
            }
            if caps.has_str("UIDPLUS") {
                let stream = s.uid_expunge(&set).await?;
                let _: Vec<u32> = stream.try_collect().await?;
            } else {
                let stream = s.expunge().await?;
                let _: Vec<u32> = stream.try_collect().await?;
            }
        }
    }
    s.logout().await.ok();
    Ok(())
}

pub async fn append(acc: &AccountConfig, password: &str, folder: &str, raw: &[u8]) -> Result<()> {
    let mut s = connect(acc, password).await?;
    s.append(folder, Some("(\\Seen)"), None, raw).await?;
    s.logout().await.ok();
    Ok(())
}

/// Saves a draft: appends it to `folder` (or a new "Drafts" folder) and
/// removes older copies of the same draft (same Message-ID, or the server
/// copy it was opened from). Returns the folder used.
pub async fn save_draft(
    acc: &AccountConfig,
    password: &str,
    folder: Option<String>,
    raw: &[u8],
    message_id: &str,
    opened_from: Option<u32>,
) -> Result<String> {
    let mut s = connect(acc, password).await?;
    let folder = match folder {
        Some(f) => f,
        None => {
            // Fails harmlessly if it already exists.
            s.create("Drafts").await.ok();
            "Drafts".to_string()
        }
    };
    s.append(&folder, Some("(\\Draft \\Seen)"), None, raw).await.context("saving the draft failed")?;
    purge_draft(&mut s, &folder, message_id, true, opened_from).await?;
    s.logout().await.ok();
    Ok(folder)
}

/// Deletes all server copies of a draft (after sending or discarding).
pub async fn delete_draft(
    acc: &AccountConfig,
    password: &str,
    folder: &str,
    message_id: &str,
    opened_from: Option<u32>,
) -> Result<()> {
    let mut s = connect(acc, password).await?;
    purge_draft(&mut s, folder, message_id, false, opened_from).await?;
    s.logout().await.ok();
    Ok(())
}

async fn purge_draft(
    s: &mut ImapSession,
    folder: &str,
    message_id: &str,
    keep_newest: bool,
    opened_from: Option<u32>,
) -> Result<()> {
    s.select(folder).await?;
    let id = message_id.trim().trim_start_matches('<').trim_end_matches('>').replace('"', "");
    let mut uids: Vec<u32> = if id.is_empty() {
        Vec::new()
    } else {
        s.uid_search(format!("HEADER Message-ID \"{id}\"")).await?.into_iter().collect()
    };
    uids.sort_unstable();
    if keep_newest {
        // The copy just appended has the highest UID.
        uids.pop();
    }
    if let Some(u) = opened_from {
        if !uids.contains(&u) {
            uids.push(u);
        }
    }
    if uids.is_empty() {
        return Ok(());
    }
    let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    {
        let stream = s.uid_store(&set, "+FLAGS.SILENT (\\Deleted)").await?;
        let _: Vec<Fetch> = stream.try_collect().await?;
    }
    let caps = s.capabilities().await?;
    if caps.has_str("UIDPLUS") {
        let stream = s.uid_expunge(&set).await?;
        let _: Vec<u32> = stream.try_collect().await?;
    } else {
        let stream = s.expunge().await?;
        let _: Vec<u32> = stream.try_collect().await?;
    }
    Ok(())
}

/// Opens a connection and logs in — used by "Test connection" in settings.
/// Creates a new folder on the server (IMAP CREATE).
pub async fn create_folder(acc: &AccountConfig, password: &str, name: &str) -> Result<()> {
    let mut s = connect(acc, password).await?;
    s.create(name).await?;
    s.logout().await.ok();
    Ok(())
}

pub async fn test(acc: &AccountConfig, password: &str) -> Result<()> {
    let mut s = connect(acc, password).await?;
    s.logout().await.ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_html_only_mail() {
        let raw = b"From: \"Alice A\" <Alice@Example.com>\r\nTo: bob@example.com\r\nSubject: Hi\r\nMessage-ID: <abc@x>\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>Bob</b></p>";
        let m = parse_raw(raw).unwrap();
        assert_eq!(m.from_name, "Alice A");
        assert_eq!(m.from_addr, "alice@example.com");
        assert_eq!(m.message_id, "<abc@x>");
        assert!(m.body_text.contains("Hello"));
        assert!(m.preview.contains("Bob"));
    }

    #[test]
    fn agent_header_sets_tag() {
        let with = parse_raw(b"From: a@x\r\nX-Agent-Processed: 2026-11-16\r\nSubject: s\r\n\r\nbody").unwrap();
        assert_eq!(with.tags, crate::store::TAG_AGENT);
        let no_hyphen = parse_raw(b"From: a@x\r\nX-AgentProcessed: yes\r\nSubject: s\r\n\r\nbody").unwrap();
        assert_eq!(no_hyphen.tags, crate::store::TAG_AGENT);
        let without = parse_raw(b"From: a@x\r\nX-Receipt: agent processed elsewhere\r\nSubject: s\r\n\r\nbody").unwrap();
        assert_eq!(without.tags, 0, "only the AgentProcessed field matches");
    }

    /// End-to-end against a local GreenMail server:
    /// `docker run --rm -p 3025:3025 -p 3143:3143 -p 3465:3465 -p 3993:3993 \
    ///   -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 \
    ///   -Dgreenmail.users=test:secret@localhost -Dgreenmail.auth.disabled=false" greenmail/standalone:2.1.3`
    /// then `cargo test -- --ignored e2e`
    /// Creating a folder: server LIST must show it afterwards.
    #[tokio::test]
    #[ignore]
    async fn e2e_create_folder() {
        use crate::config::{Preset, Security, ServerConfig};
        rustls::crypto::ring::default_provider().install_default().ok();
        let mut acc = AccountConfig::from_preset(Preset::Generic, "test@localhost");
        acc.imap = ServerConfig {
            host: "127.0.0.1".into(),
            port: 3143,
            security: Security::Plain,
            username: "test".into(),
            accept_invalid_certs: true,
        };
        acc.save_sent_copy = false;

        let tag = crate::config::new_id();
        let name = format!("Created-{}", &tag[..6]);
        create_folder(&acc, "secret", &name).await.unwrap();
        // A plain LIST must contain the new folder.
        let mut s = connect(&acc, "secret").await.unwrap();
        let boxes = s.list(Some("".into()), Some("*".into())).await.unwrap().try_collect::<Vec<_>>().await.unwrap();
        s.logout().await.ok();
        assert!(boxes.iter().any(|b| b.name().eq_ignore_ascii_case(&name)), "{boxes:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_greenmail() {
        use crate::config::{Preset, ServerConfig};
        rustls::crypto::ring::default_provider().install_default().ok();
        let mut acc = AccountConfig::from_preset(Preset::Generic, "test@localhost");
        let srv = |port, security| ServerConfig {
            host: "127.0.0.1".into(),
            port,
            security,
            username: "test".into(),
            accept_invalid_certs: true,
        };
        acc.imap = srv(3993, Security::Tls);
        acc.smtp = srv(3465, Security::Tls);
        acc.save_sent_copy = false;

        let tag = crate::config::new_id();
        for (subj, body) in [("Invoice October", "please pay"), ("Lunch", "pizza at noon")] {
            let m = crate::smtp::build(
                &acc,
                &crate::smtp::Outgoing {
                    to: "test@localhost".into(),
                    subject: format!("{subj} {tag}"),
                    body: body.into(),
                    ..Default::default()
                },
            )
            .unwrap();
            crate::smtp::send(&acc, "secret", &m).await.unwrap();
        }

        let store = Store::in_memory().unwrap();
        let search = Search::in_memory().unwrap();
        let r = sync_account(&acc, "secret", &store, &search).await.unwrap();
        assert!(r.new_messages >= 2, "{r:?}");
        // Attachment round trip: SMTP → IMAP → raw store → parser.
        let mut out = crate::smtp::Outgoing {
            to: "test@localhost".into(),
            subject: format!("Report {tag}"),
            body: "see attached".into(),
            ..Default::default()
        };
        out.attachments.push(crate::mime::Attachment {
            name: "q3.pdf".into(),
            mime: "application/pdf".into(),
            data: std::sync::Arc::new(vec![7u8; 300_000]),
            content_id: None,
            embedded: false,
        });
        crate::smtp::send(&acc, "secret", &crate::smtp::build(&acc, &out).unwrap()).await.unwrap();
        sync_account(&acc, "secret", &store, &search).await.unwrap();
        let rid = search.query(&format!("report {tag}"), 10).unwrap()[0];
        let rmsg = store.message(rid).unwrap().unwrap();
        assert_eq!(rmsg.attachments, vec!["q3.pdf".to_string()]);
        let parsed = crate::mime::parse(&store.raw(rid).unwrap().unwrap());
        let files: Vec<_> = parsed.files().collect();
        assert_eq!(files[0].1.data.len(), 300_000);
        // On-demand download returns the same source.
        let fetched = fetch_raw(&acc, "secret", "INBOX", rmsg.summary.uid).await.unwrap();
        assert_eq!(crate::mime::parse(&fetched).attachments[0].data.len(), 300_000);

        let ids = search.query(&format!("pizza {tag}"), 10).unwrap();
        assert_eq!(ids.len(), 1);
        let msg = store.message(ids[0]).unwrap().unwrap();
        assert!(msg.summary.subject.starts_with("Lunch"));
        assert!(!msg.summary.seen);

        set_flag(&acc, "secret", "INBOX", msg.summary.uid, "\\Seen", true).await.unwrap();
        move_message(&acc, "secret", "INBOX", msg.summary.uid, None).await.unwrap();

        // Plain IMAP port too, and incremental sync picks up the changes.
        acc.imap = srv(3143, Security::Plain);
        let r = sync_account(&acc, "secret", &store, &search).await.unwrap();
        assert_eq!(r.removed, 1, "{r:?}");
        assert!(search.query(&format!("pizza {tag}"), 10).unwrap().is_empty());
        assert_eq!(search.query(&format!("invoice {tag}"), 10).unwrap().len(), 1);
    }

    /// STARTTLS upgrade (what Proton Bridge uses), against Dovecot:
    /// `docker run --rm -e USER_PASSWORD=pass -p 31143:31143 dovecot/dovecot` then `cargo test -- --ignored starttls`
    #[tokio::test]
    #[ignore]
    async fn e2e_drafts_dovecot() {
        use crate::config::{Preset, ServerConfig};
        rustls::crypto::ring::default_provider().install_default().ok();
        let mut acc = AccountConfig::from_preset(Preset::ProtonBridge, "user@localhost");
        acc.imap = ServerConfig {
            host: "127.0.0.1".into(),
            port: 31143,
            security: Security::StartTls,
            username: "user".into(),
            accept_invalid_certs: true,
        };
        let id = format!("<draft{}@mailix>", rand::random::<u32>());
        let mk = |body: &str, to: &str| {
            let out = crate::smtp::Outgoing {
                to: to.into(),
                subject: "Entwurf Grüße".into(),
                body: body.into(),
                ..Default::default()
            };
            crate::smtp::build_draft(&acc, &out, &id, Some("Work"), None).unwrap()
        };
        async fn count(acc: &AccountConfig, folder: &str, id: &str) -> Vec<u32> {
            let mut s = connect(acc, "pass").await.unwrap();
            s.select(folder).await.unwrap();
            let id = id.trim_matches(['<', '>']);
            let mut v: Vec<u32> = s.uid_search(format!("HEADER Message-ID \"{id}\"")).await.unwrap().into_iter().collect();
            v.sort();
            v
        }
        // No Drafts folder known: creates one. Half-typed recipient is fine.
        let folder = save_draft(&acc, "pass", None, &mk("first **version**", "bo"), &id, None).await.unwrap();
        assert_eq!(count(&acc, &folder, &id).await.len(), 1);
        // Saving again replaces the old copy.
        save_draft(&acc, "pass", Some(folder.clone()), &mk("second", "bob@example.com"), &id, None).await.unwrap();
        let uids = count(&acc, &folder, &id).await;
        assert_eq!(uids.len(), 1);
        // The stored draft parses back with body, recipients and signature.
        let raw = fetch_raw(&acc, "pass", &folder, uids[0]).await.unwrap();
        let p = parse_raw(&raw).unwrap();
        assert_eq!(p.subject, "Entwurf Grüße");
        assert!(p.body_text.contains("second"));
        assert!(p.to.contains("bob@example.com"));
        let m = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(m.headers().value("X-Mailix-Signature").unwrap().as_text(), Some("Work"));
        // Deleting removes every copy.
        delete_draft(&acc, "pass", &folder, &id, Some(uids[0])).await.unwrap();
        assert!(count(&acc, &folder, &id).await.is_empty());

        // Several messages moved in one go (drag & drop of a conversation).
        let tag = format!("dnd{}", rand::random::<u32>());
        for i in 0..3 {
            let raw = format!("From: a@x\r\nTo: user@localhost\r\nSubject: {tag} {i}\r\nMessage-ID: <{tag}.{i}@x>\r\n\r\nhi\r\n");
            append(&acc, "pass", "INBOX", raw.as_bytes()).await.unwrap();
        }
        let mut s = connect(&acc, "pass").await.unwrap();
        s.create("Projekte").await.ok();
        s.select("INBOX").await.unwrap();
        let mut found: Vec<u32> = s.uid_search(format!("SUBJECT {tag}")).await.unwrap().into_iter().collect();
        found.sort();
        assert_eq!(found.len(), 3);
        move_messages(&acc, "pass", "INBOX", &found[..2], Some("Projekte")).await.unwrap();
        s.select("INBOX").await.unwrap();
        assert_eq!(s.uid_search(format!("SUBJECT {tag}")).await.unwrap().len(), 1);
        s.select("Projekte").await.unwrap();
        assert_eq!(s.uid_search(format!("SUBJECT {tag}")).await.unwrap().len(), 2);
    }

    #[tokio::test]
    #[ignore]
    async fn e2e_starttls_dovecot() {
        use crate::config::{Preset, ServerConfig};
        rustls::crypto::ring::default_provider().install_default().ok();
        let mut acc = AccountConfig::from_preset(Preset::ProtonBridge, "user@localhost");
        acc.imap = ServerConfig {
            host: "127.0.0.1".into(),
            port: 31143,
            security: Security::StartTls,
            username: "user".into(),
            accept_invalid_certs: true,
        };
        let store = Store::in_memory().unwrap();
        let search = Search::in_memory().unwrap();
        sync_account(&acc, "pass", &store, &search).await.expect("STARTTLS sync");
        assert!(!store.folders(&acc.id).unwrap().is_empty());
        assert!(test(&acc, "wrong").await.is_err());

        // IDLE: a message appended from another connection is pushed to us.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let watcher = {
            let acc = acc.clone();
            tokio::spawn(async move {
                let tx = tx;
                watch_inbox(&acc, "pass", &mut move || {
                    let _ = tx.send(());
                })
                .await
            })
        };
        async fn wait(rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>) {
            tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.expect("no IDLE event");
        }
        wait(&mut rx).await; // initial catch-up on connect
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        {
            let mut other = connect(&acc, "pass").await.unwrap();
            other.append("INBOX", None, None, b"From: a@x\r\nSubject: pushed\r\n\r\nhi\r\n").await.unwrap();
            other.logout().await.ok();
        }
        let t = std::time::Instant::now();
        wait(&mut rx).await;
        assert!(t.elapsed() < std::time::Duration::from_secs(5), "push took {:?}", t.elapsed());
        watcher.abort();

        let r = sync_inbox(&acc, "pass", &store, &search).await.unwrap();
        assert!(r.new_messages >= 1);
        assert_eq!(r.new_inbox.iter().filter(|(_, s)| s == "pushed").count(), 1, "{:?}", r.new_inbox);
    }

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_folder_name("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_folder_name("Gel&APY-schte Elemente"), "Gelöschte Elemente");
        assert_eq!(decode_folder_name("&AMQA1gDcAOQA9gD8AN8-"), "ÄÖÜäöüß");
        assert_eq!(decode_folder_name("R&AOk-sum&AOk-"), "Résumé");
        assert_eq!(decode_folder_name("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_folder_name("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_folder_name("INBOX"), "INBOX");
        assert_eq!(decode_folder_name("broken&xx"), "broken&xx");
    }

    #[test]
    fn assigns_one_role_each() {
        let f = |n: &str, attr: Option<FolderRole>| (n.to_string(), Some('.'), attr);
        // The reported layout: several "sent" candidates, no SPECIAL-USE.
        let r = assign_roles(&[
            f("INBOX", None),
            f("INBOX.Archives.Sent Messages", None),
            f("INBOX.Gesendet", None),
            f("INBOX.Sent", None),
            f("INBOX.Entw&APw-rfe", None),
            f("INBOX.Gel&APY-schte Elemente", None),
            f("INBOX.Archives", None),
        ]);
        assert_eq!(
            r,
            vec![
                FolderRole::Inbox,
                FolderRole::Other,
                FolderRole::Other,
                FolderRole::Sent,
                FolderRole::Drafts,
                FolderRole::Trash,
                FolderRole::Archive
            ]
        );
        // SPECIAL-USE flag wins over names.
        let r = assign_roles(&[f("INBOX", None), f("INBOX.Sent", None), f("INBOX.Gesendet", Some(FolderRole::Sent))]);
        assert_eq!(r, vec![FolderRole::Inbox, FolderRole::Other, FolderRole::Sent]);
        // Top-level folders with '/' delimiter.
        let r = assign_roles(&[("Sent Items".into(), Some('/'), None), ("Projects/Sent".into(), Some('/'), None)]);
        assert_eq!(r, vec![FolderRole::Sent, FolderRole::Other]);
    }
}
